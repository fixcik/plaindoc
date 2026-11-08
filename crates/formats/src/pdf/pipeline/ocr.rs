//! OCR pass of the job and the folding of a page's OCR into its section.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use pdf_oxide::PdfDocument;
use plaindoc_core::telemetry;
use plaindoc_core::{
    Block, HiddenText, Level, ParseError, ParseOptions, Result, Section, Stage, WarningKind,
};
use rayon::prelude::*;

use super::super::PdfConfig;
use super::super::diagnose::PageDecision;
use super::super::hidden_ocr::ocr_mismatch_warning;
use super::super::ocr_stage::{OcrPageCounts, PageOcr, ocr_failed_warning, run_page_ocr};
use super::super::table_place::block_bbox;
use super::{RawPage, in_page};

/// What the OCR pass did: `done < wanted` when `ocr_max_pages` cut pages off.
pub(super) struct OcrPass {
    pub(super) done: usize,
    pub(super) wanted: usize,
    pub(super) workers: usize,
}

/// OCR pass of the job (rulings I4, I6): the first `ocr_max_pages` pages that need OCR, in page
/// order, in batches of `engine.max_concurrency()` (parallel inside a batch), so the recognised set
/// does not depend on the thread count and one job never waits on more engine sessions than exist.
/// After the first `Unavailable` the remaining pages are not rendered.
pub(super) fn ocr_pass(
    doc: &PdfDocument,
    raw: &mut [RawPage],
    cfg: &PdfConfig,
    opts: &ParseOptions,
    cancel: &AtomicBool,
    parallel: bool,
) -> Result<OcrPass> {
    let want: Vec<(usize, PageDecision)> = raw
        .iter()
        .enumerate()
        .filter_map(|(p, r)| r.ocr_want.map(|d| (p, d)))
        .collect();
    let engine = cfg.ocr.engine.as_deref();
    // Without an engine nothing is rendered: every wanted page stays in the `OcrUnavailable` count.
    let limit = engine.and(opts.ocr_max_pages).unwrap_or(usize::MAX);
    let todo = &want[..want.len().min(limit)];
    let workers = engine.map_or(1, |e| e.max_concurrency()).max(1);
    let mut gone: Option<String> = None;
    let parent = telemetry::Span::current();
    for chunk in todo.chunks(workers) {
        if cancel.load(Ordering::Relaxed) {
            return Err(ParseError::Timeout(Duration::ZERO));
        }
        let run = |&(p, d): &(usize, PageDecision)| {
            in_page(&parent, p, || {
                telemetry::stage(Stage::Ocr, || {
                    let t = Instant::now();
                    let mut c = OcrPageCounts::default();
                    let o = match &gone {
                        Some(m) => PageOcr::Unavailable(m.clone()),
                        None => {
                            let layer = raw[p].structured.as_ref().ok();
                            let tables = &*cfg.tables;
                            run_page_ocr(doc, p, d, layer, &cfg.ocr, tables, opts, cancel, &mut c)
                        }
                    };
                    c.took = t.elapsed();
                    (o, c)
                })
            })
        };
        let out: Vec<(PageOcr, OcrPageCounts)> = if parallel && chunk.len() > 1 {
            chunk.par_iter().map(run).collect()
        } else {
            chunk.iter().map(run).collect()
        };
        for (&(p, d), (o, c)) in chunk.iter().zip(out) {
            if gone.is_none()
                && let PageOcr::Unavailable(m) = &o
            {
                gone = Some(m.clone());
            }
            raw[p].ocr = Some((o, d));
            raw[p].ocr_counts = c;
        }
    }
    Ok(OcrPass {
        done: todo.len(),
        wanted: want.len(),
        // Real batch width for the memory estimate (`max_concurrency` may be `usize::MAX`).
        workers: workers.min(todo.len()).max(1),
    })
}

/// Folds one page's OCR into its section: replace or merge blocks, stage-2 warnings, source and
/// quality. A recovered page keeps its stage-1 loss as `Info` (the fact stays, the loss is gone).
/// A text layer the full-page merge OCR does not read (`PageOcr::Done::mismatch`) gets a
/// `HiddenContent` `Warn`, and in `Strip` the page's text becomes the OCR (when OCR found text with
/// confidence; the layer's tables and their warnings go too). Returns whether the layer was swapped.
pub(super) fn apply_ocr(
    section: &mut Section,
    ocr: Option<&(PageOcr, PageDecision)>,
    mode: HiddenText,
    unavailable: &mut Vec<(PageDecision, String)>,
) -> bool {
    use plaindoc_core::TextSource;
    let Some((ocr, decision)) = ocr else {
        return false;
    };
    match ocr {
        PageOcr::Unavailable(m) => unavailable.push((*decision, m.clone())),
        PageOcr::Failed(e) => section.warnings.push(ocr_failed_warning(e)),
        PageOcr::Done {
            blocks,
            warnings,
            quality,
            replace,
            mismatch,
        } => {
            let found = !warnings.iter().any(|w| w.kind == WarningKind::OcrEmpty);
            // Only a confident OCR that found text replaces the layer.
            let swap = mismatch.as_ref().filter(|_| {
                mode == HiddenText::Strip
                    && found
                    && !warnings
                        .iter()
                        .any(|w| w.kind == WarningKind::LowOcrConfidence)
            });
            if let Some(m) = swap {
                section.blocks = m.blocks.clone();
                section.source = TextSource::Ocr;
                section.quality = *quality;
                section
                    .warnings
                    .retain(|w| w.kind != WarningKind::TableUncertain);
                for w in &mut section.warnings {
                    if w.level == Level::Warn && w.kind == WarningKind::ImageTextPage {
                        w.level = Level::Info;
                    }
                }
            } else if *replace {
                if found {
                    section.blocks = blocks.clone();
                    section.source = TextSource::Ocr;
                    section.quality = *quality;
                    // Only the stage-1 losses that OCR recovers; other losses stay.
                    for w in &mut section.warnings {
                        if w.level == Level::Loss
                            && matches!(
                                w.kind,
                                WarningKind::NoTextLayer | WarningKind::SuspectedGarbage
                            )
                        {
                            w.level = Level::Info;
                        }
                    }
                }
            } else {
                // Merge OCR read the image text: the stage-1 hint is no longer a warning.
                if found {
                    for w in &mut section.warnings {
                        if w.level == Level::Warn && w.kind == WarningKind::ImageTextPage {
                            w.level = Level::Info;
                        }
                    }
                }
                if !blocks.is_empty() {
                    for b in blocks {
                        place_by_top(&mut section.blocks, b.clone());
                    }
                    section.source = TextSource::Mixed;
                }
            }
            section.warnings.extend(warnings.iter().cloned());
            if let Some(m) = mismatch {
                section
                    .warnings
                    .push(ocr_mismatch_warning(m.check, swap.is_some()));
            }
            return swap.is_some();
        }
    }
    false
}

/// Inserts `b` before the first block whose top edge is not above `b`'s and that overlaps it
/// horizontally (as SP4 `place_tables`); blocks without a box go to the end.
fn place_by_top(blocks: &mut Vec<Block>, b: Block) {
    let at = block_bbox(&b).and_then(|t| {
        let top = t.y + t.h;
        blocks.iter().position(|o| {
            block_bbox(o)
                .is_some_and(|bb| bb.y + bb.h <= top + 1.0 && bb.x < t.x + t.w && t.x < bb.x + bb.w)
        })
    });
    blocks.insert(at.unwrap_or(blocks.len()), b);
}
