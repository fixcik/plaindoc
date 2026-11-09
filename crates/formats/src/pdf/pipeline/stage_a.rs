//! Stage A: per-page extraction, hidden-text filtering, table detection and diagnosis.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use pdf_oxide::optional_content::LayerFilter;
use pdf_oxide::{PdfDocument, RegionRole, StructuredPage};
use plaindoc_core::OcrMode;
use plaindoc_core::telemetry;
use plaindoc_core::{HiddenText, ParseError, ParseOptions, Result, Stage};

use super::super::PdfConfig;
use super::super::bullets::bullet_boxes;
#[cfg(feature = "ocr")]
use super::super::diagnose::PageDecision;
use super::super::diagnose::ocr_needs;
use super::super::diagnose::{
    ClassifyOutcome, FactKind, PageDiagnosis, PageFacts, diagnose_page, extraction_failed,
};
use super::super::garbage::garbage_score;
use super::super::hidden::{HiddenCounts, SCAN_IMAGE_SHARE, filter_page_boxes, page_facts};
use super::super::layout::non_ws_chars;
use super::super::links::page_links;
use super::super::need_ocr::page_signals;
use super::super::ocg::extract_page;
#[cfg(feature = "ocr")]
use super::super::ocr_stage::OcrPageCounts;
use super::super::pictures::{self, PagePictures};
use super::super::spacing::tighten_page;
use super::super::tables::{TableInputCounts, WordFilter};
use super::{PageCounters, RawPage};

/// Text spans of a page and a rough byte count of its structured form.
fn page_size(page: &StructuredPage) -> (u64, u64) {
    let span = std::mem::size_of::<pdf_oxide::layout::TextSpan>() as u64;
    page.regions.iter().fold((0u64, 0u64), |(n, b), r| {
        let text = r
            .spans
            .iter()
            .fold(0u64, |t, s| t.saturating_add(s.text.len() as u64));
        let k = r.spans.len() as u64;
        let bytes = (r.text.len() as u64)
            .saturating_add(k.saturating_mul(span))
            .saturating_add(text);
        (n.saturating_add(k), b.saturating_add(bytes))
    })
}

/// Stage-1 diagnosis of one page. The classifier signals (extra char pass, gated 50-dpi render)
/// are computed only when `need_ocr` is switched on.
fn page_diagnosis(
    doc: &PdfDocument,
    p: usize,
    page: &StructuredPage,
    classify: &ClassifyOutcome,
    cfg: &PdfConfig,
) -> PageDiagnosis {
    let live = || page.regions.iter().filter(|r| !r.text.trim().is_empty());
    let text = live()
        .filter(|reg| reg.kind != RegionRole::Artifact)
        .map(|reg| reg.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    // Same rule as `page_to_blocks` (empty regions skipped), but over the whole page: stage B
    // lays out the page without table text, the diagnosis (and the OCR decision) sees all of it.
    let (dropped_regions, dropped_chars) = live()
        .filter(|reg| reg.kind == RegionRole::Artifact)
        .fold((0, 0), |(n, c), reg| (n + 1, c + non_ws_chars(&reg.text)));
    let d = &cfg.diagnostics;
    let signals = d.need_ocr.as_ref().map(|nc| {
        let facts = match classify {
            ClassifyOutcome::Facts(f) => Some(f),
            ClassifyOutcome::Failed(_) => None,
        };
        page_signals(doc, p, page, facts, d.bigrams.as_deref(), nc)
    });
    diagnose_page(
        classify,
        signals.as_ref(),
        garbage_score(&text),
        dropped_regions,
        dropped_chars,
        d,
    )
}

/// Images of a page that keeps its text layer (`pictures::takes_pictures`), listed without
/// decoding; under `OcrMode::Auto` the policy then takes the page's image OCR off
/// (`pictures::skip_image_ocr`, which rewrites `diag`).
fn page_pictures(
    doc: &PdfDocument,
    p: usize,
    structured: &std::result::Result<StructuredPage, String>,
    classify: &ClassifyOutcome,
    diag: &mut PageDiagnosis,
    cfg: &PdfConfig,
    opts: &ParseOptions,
) -> Option<PagePictures> {
    let (Ok(page), ClassifyOutcome::Facts(f)) = (structured, classify) else {
        return None;
    };
    let wanted = ocr_needs(&[diag.decision], &cfg.diagnostics)[0];
    let cover = f.image_area_ratio.min(1.0);
    // cheap test first: most pages have no images
    if cover <= 0.0 {
        return None;
    }
    let nonws = page
        .regions
        .iter()
        .filter(|r| r.kind != RegionRole::Artifact)
        .map(|r| non_ws_chars(&r.text))
        .sum();
    if !pictures::takes_pictures(wanted, cover, nonws) {
        return None;
    }
    let mut pics = pictures::collect(doc, p)?;
    if opts.ocr == OcrMode::Auto {
        pictures::skip_image_ocr(diag, wanted, &mut pics);
    }
    Some(pics)
}

pub(super) fn stage_a(
    doc: &PdfDocument,
    p: usize,
    cfg: &PdfConfig,
    opts: &ParseOptions,
    layers: Option<&LayerFilter>,
    cancel: &AtomicBool,
) -> Result<RawPage> {
    if cancel.load(Ordering::Relaxed) {
        return Err(ParseError::Timeout(Duration::ZERO));
    }
    let t0 = Instant::now();
    // The hidden-text filter is part of the text stage: it runs before the page diagnosis, so
    // hidden text does not count towards the garbage score. It filters the page that is used,
    // after the column gate (a re-extraction reuses the cached raw spans and paints).
    let (structured, classify, mut hidden, facts) = telemetry::stage(Stage::Text, || {
        let (structured, ocg) = extract_page(doc, p, layers, opts.hidden_text);
        let mut structured = structured.map_err(|e| e.to_string());
        if let Ok(page) = &mut structured {
            tighten_page(page);
        }
        let classify = match doc.classify_page(p) {
            Ok(c) => ClassifyOutcome::Facts(PageFacts::from(&c)),
            Err(e) => ClassifyOutcome::Failed(e.to_string()),
        };
        let (hidden, facts) = match &mut structured {
            Ok(page) if opts.hidden_text != HiddenText::Off => {
                let scan = matches!(&classify, ClassifyOutcome::Facts(f)
                    if matches!(f.kind, FactKind::Scanned | FactKind::ImageText)
                        || f.image_area_ratio >= SCAN_IMAGE_SHARE);
                let mut facts = page_facts(doc, p, page, scan);
                // default-off layers first, from the raw pages (`ocg`), then the span rules
                let (mut c, mut removed) = (ocg.counts, ocg.removed);
                c.merge_max(&filter_page_boxes(
                    page,
                    opts.hidden_text,
                    &facts,
                    &mut removed,
                ));
                facts.removed = removed;
                (c, Some(facts))
            }
            _ => (HiddenCounts::default(), None),
        };
        (structured, classify, hidden, facts)
    });
    let t1 = Instant::now();
    let mut table_in = TableInputCounts::default();
    // The table words' hidden-text filter runs in the tables stage: the glyphs of the spans
    // removed above, then the span rules (another extraction of the same glyphs: counts combine
    // by the larger per rule, `HiddenCounts::merge_max`).
    let tables = telemetry::stage(Stage::Tables, || match (&structured, &facts) {
        (Ok(page), Some(f)) => {
            let mut words = WordFilter::new(opts.hidden_text, f);
            let r = cfg
                .tables
                .detect_filtered(doc, page, &mut table_in, &mut words);
            hidden.merge_max(&words.counts);
            r.map_err(|e| e.to_string())
        }
        (Ok(page), None) => cfg
            .tables
            .detect_counted(doc, page, &mut table_in)
            .map_err(|e| e.to_string()),
        (Err(_), _) => Ok(Vec::new()),
    });
    let links = page_links(doc, p, &structured, facts.as_ref(), opts.links);
    let bullets = match &structured {
        // a page over the table adapter's input limits is not read for paths again
        Ok(page) if !page.regions.is_empty() && table_in.limit.is_none() => bullet_boxes(doc, p),
        _ => Vec::new(),
    };
    drop(facts);
    let t2 = Instant::now();
    let mut diag = telemetry::stage(Stage::Diagnose, || match &structured {
        Ok(page) => page_diagnosis(doc, p, page, &classify, cfg),
        Err(e) => extraction_failed(e),
    });
    let t3 = Instant::now();
    let pictures = telemetry::stage(Stage::Images, || {
        page_pictures(doc, p, &structured, &classify, &mut diag, cfg, opts)
    });
    let t4 = Instant::now();
    // After the timers: the span walk is not part of any stage.
    let (spans, page_bytes) = structured.as_ref().map_or((0, 0), page_size);
    let counters = PageCounters {
        text: t1 - t0,
        tables: t2 - t1,
        diagnose: t3 - t2,
        images: t4 - t3,
        spans,
        page_bytes,
        table_in,
    };
    // With the classifier off, `Auto` OCRs only full-page losses (`ocr_needs`); `Always` OCRs
    // every page, merging on pages with a usable layer.
    #[cfg(feature = "ocr")]
    let ocr_want = {
        let decision = ocr_needs(&[diag.decision], &cfg.diagnostics)[0];
        let run = match opts.ocr {
            OcrMode::Never => false,
            OcrMode::Always => true,
            _ => decision != PageDecision::UseTextLayer,
        };
        run.then_some(decision)
    };
    // Done with this page's extractions: its entries leave the document's per-page caches, which
    // then hold only the pages in flight (OCR pages keep theirs for the OCR pass).
    #[cfg(feature = "ocr")]
    let keep = ocr_want.is_some();
    #[cfg(not(feature = "ocr"))]
    let keep = false;
    if !keep {
        doc.forget_page(p);
    }
    Ok(RawPage {
        structured,
        classify,
        tables,
        diag,
        hidden,
        links,
        bullets,
        pictures,
        #[cfg(feature = "ocr")]
        ocr_want,
        #[cfg(feature = "ocr")]
        ocr: None,
        counters,
        #[cfg(feature = "ocr")]
        ocr_counts: OcrPageCounts::default(),
    })
}
