//! Page pipeline: per-page extraction (stage A), sequential layout and diagnostics (stage B).

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use pdf_oxide::StructuredPage;
use plaindoc_core::telemetry::{self, PanicSite};
use plaindoc_core::{
    Document, HiddenText, Level, LimitKind, ParseError, ParseOptions, ParseStats, Result, Section,
    SectionKind, Stage, Warning, WarningKind, apply_quality,
};

use super::PdfConfig;
#[cfg(not(feature = "ocr"))]
use super::diagnose::ocr_needs;
use super::diagnose::{
    ClassifyOutcome, FactKind, PageDecision, PageDiagnosis, bigrams_missing, ocr_unavailable,
};
use super::heading_rank::{RankedHead, rank_levels};
use super::headings::{font_stats, line_pitch_ratio};
use super::hidden::HiddenCounts;
use super::layout::{PageContext, ReadingOrderMode, body_font_size, page_to_blocks_with};
use super::links::{PageLinks, add_links};
use super::ocg::layer_filter;
#[cfg(feature = "ocr")]
use super::ocr_stage::{OcrPageCounts, PageOcr};
use super::open::OpenedPdf;
use super::order::order_blocks;
use super::outline::Outline;
use super::running::{mark_running, move_out};
use super::table_place::{place_tables, strip_table_text};
use super::tables::{DetectedTable, TableInputCounts, continues};
use crate::tables::{InputPath, InputWord};

mod pages;
use pages::{page_workers, par_pages};
mod stage_a;
use stage_a::stage_a;
#[cfg(feature = "ocr")]
mod ocr;
#[cfg(feature = "ocr")]
use ocr::{apply_ocr, ocr_pass};

/// Per-job context shared with the runner.
pub(crate) struct JobCtx {
    pub cancel: Arc<AtomicBool>,
    pub page_parallel_allowed: bool,
}

/// `decisions` feeds the OCR stage (SP5); `ran_parallel` is asserted by tests; `stats` holds the
/// in-flight counters (the registry adds the post-hoc ones).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct ParsedPdf {
    pub document: Document,
    pub decisions: Vec<PageDecision>,
    pub ran_parallel: bool,
    pub stats: ParseStats,
}

/// Stage-A counters of one page; stage B sums them in page order.
#[derive(Default)]
struct PageCounters {
    text: Duration,
    tables: Duration,
    diagnose: Duration,
    spans: u64,
    page_bytes: u64,
    table_in: TableInputCounts,
}

/// Stage A output for one page.
struct RawPage {
    structured: std::result::Result<StructuredPage, String>,
    classify: ClassifyOutcome,
    tables: std::result::Result<Vec<DetectedTable>, String>,
    diag: PageDiagnosis,
    /// Hidden text of the page (removed or flagged in stage A, reported in stage B).
    hidden: HiddenCounts,
    /// Link annotations of the page (set by stage A).
    links: PageLinks,
    /// Small filled shapes that may be list bullets (set by stage A).
    bullets: Vec<super::bullets::Bullet>,
    /// The decision to OCR the page under, when it needs OCR (set by stage A).
    #[cfg(feature = "ocr")]
    ocr_want: Option<PageDecision>,
    /// OCR of the page, when it ran: its result and the decision it ran under (set by the OCR pass).
    #[cfg(feature = "ocr")]
    ocr: Option<(PageOcr, PageDecision)>,
    counters: PageCounters,
    /// Counters of the page's OCR (set by the OCR pass).
    #[cfg(feature = "ocr")]
    ocr_counts: OcrPageCounts,
}

/// Peak working bytes of the document (decision 5): structured pages all live until stage B ends,
/// the table input and the OCR rasters are per page.
#[derive(Default)]
struct WorkEstimate {
    pages: u64,
    table_max: u64,
    raster_max: u64,
}

/// Runs `f` inside the span of `stage` and returns its wall time.
pub(super) fn timed<R>(stage: Stage, f: impl FnOnce() -> R) -> (R, Duration) {
    let t = Instant::now();
    let r = telemetry::stage(stage, f);
    (r, t.elapsed())
}

/// Runs `f` in a `page` span under `parent`: rayon workers do not inherit the job thread's span.
fn in_page<R>(parent: &telemetry::Span, p: usize, f: impl FnOnce() -> R) -> R {
    parent.in_scope(|| telemetry::page(p).in_scope(f))
}

/// Folds one page's stage-A and OCR counters into `stats`; limit and panic events of the page go
/// out here, in page order (the same sequence at any thread count).
fn count_page(stats: &mut ParseStats, p: usize, r: &RawPage, est: &mut WorkEstimate) {
    let c = &r.counters;
    stats.add_stage(Stage::Text, c.text);
    stats.add_stage(Stage::Tables, c.tables);
    stats.add_stage(Stage::Diagnose, c.diagnose);
    let z = &mut stats.sizes;
    z.spans = z.spans.saturating_add(c.spans);
    let (w, pr) = (c.table_in.words as u64, c.table_in.prims as u64);
    z.table_words = z.table_words.saturating_add(w);
    z.table_words_max = z.table_words_max.max(w);
    z.table_prims = z.table_prims.saturating_add(pr);
    z.table_prims_max = z.table_prims_max.max(pr);
    est.pages = est.pages.saturating_add(c.page_bytes);
    let table = w
        .saturating_mul(std::mem::size_of::<InputWord>() as u64)
        .saturating_add(pr.saturating_mul(std::mem::size_of::<InputPath>() as u64));
    est.table_max = est.table_max.max(table);
    if let Some(k) = c.table_in.limit {
        stats.hit(k, Some(p));
    }
    if c.table_in.panicked {
        stats.panics_isolated = stats.panics_isolated.saturating_add(1);
        telemetry::panic_isolated(PanicSite::TableDetector, Some(p));
    }
    if r.structured.is_err() {
        stats.pages.failed = stats.pages.failed.saturating_add(1);
    }
    #[cfg(feature = "ocr")]
    if let Some((o, _)) = &r.ocr {
        let oc = &r.ocr_counts;
        stats.add_stage(Stage::Ocr, oc.took);
        match o {
            PageOcr::Done { .. } => stats.pages.ocr_run = stats.pages.ocr_run.saturating_add(1),
            PageOcr::Failed(_) => {
                stats.pages.ocr_failed = stats.pages.ocr_failed.saturating_add(1);
            }
            PageOcr::Unavailable(_) => {}
        }
        if oc.timed_out {
            stats.hit(LimitKind::OcrPageTimeout, Some(p));
        }
        if oc.boxes_dropped > 0 {
            stats.hit(LimitKind::OcrMaxBoxes, Some(p));
        }
        // RGBA render plus its RGB copy
        est.raster_max = est.raster_max.max(oc.raster_px.saturating_mul(7));
    }
}

pub(crate) fn parse_opened(
    opened: OpenedPdf,
    opts: &ParseOptions,
    cfg: &PdfConfig,
    ctx: &JobCtx,
) -> Result<ParsedPdf> {
    let doc = &opened.doc;
    doc.set_char_x_offset_refinement(cfg.char_x_refinement);
    // Once per document, before stage A (the setting clears the fork's span caches).
    if cfg.visibility_facts && opts.hidden_text != HiddenText::Off {
        doc.set_visibility_facts(true);
    }
    let total = doc
        .page_count()
        .map_err(|e| ParseError::Corrupt(e.to_string()))?;
    if total == 0 {
        return Err(ParseError::Corrupt("PDF has no pages".into()));
    }
    let n = total.min(opts.max_pages.unwrap_or(usize::MAX));

    // Optional content off by default, once per document (none in almost every file); counted
    // in the text stage, like the per-page hidden-text filter.
    let (layers, ocgs_took) = timed(Stage::Text, || layer_filter(doc, opts.hidden_text));
    // Stage A: page 0 warms up shared document caches, then the rest.
    let cancel = &*ctx.cancel;
    let workers = if ctx.page_parallel_allowed && opts.page_parallel && n >= cfg.min_parallel_pages
    {
        page_workers(opts.page_threads, n - 1)
    } else {
        1
    };
    let parallel = workers > 1;
    let parent = telemetry::Span::current();
    let page_a = |p: usize| {
        in_page(&parent, p, || {
            stage_a(doc, p, cfg, opts, layers.as_ref(), cancel)
        })
    };
    // The fork's running header/footer scan (every page, once per document) runs inside page 0,
    // on this thread alone; with pages in parallel it is started first on `workers` tasks of
    // PDF_POOL, while no page task of this document is queued. Same result (the fork aggregates in
    // page order); an error is left to page 0, which runs the scan again and fails as before. Not
    // after a cancel: page 0 then stops before reaching the scan, as it did.
    let scan_took = if parallel && !cancel.load(Ordering::Relaxed) {
        timed(Stage::Text, || {
            let _ = doc.scan_running_artifacts_on_current_pool(workers);
        })
        .1
    } else {
        Duration::ZERO
    };
    let mut raw = Vec::with_capacity(n);
    if n > 0 {
        raw.push(page_a(0)?);
    }
    let rest: Vec<RawPage> = if parallel {
        par_pages(1..n, workers, page_a)?
    } else {
        (1..n).map(page_a).collect::<Result<Vec<_>>>()?
    };
    raw.extend(rest);
    // OCR pass in the same job; small documents too (each OCR page is heavy).
    #[cfg(feature = "ocr")]
    let pass = ocr_pass(
        doc,
        &mut raw,
        cfg,
        opts,
        cancel,
        ctx.page_parallel_allowed && opts.page_parallel,
    )?;

    // Stage B: sequential layout and diagnostics.
    let _assemble = telemetry::stage_guard(Stage::Assemble);
    let assemble = Instant::now();
    let mut stats = ParseStats::default();
    stats.page_parallel = parallel;
    stats.add_stage(Stage::Text, ocgs_took + scan_took);
    let mut est = WorkEstimate::default();
    let ok_pages: Vec<&StructuredPage> = raw
        .iter()
        .filter_map(|r| r.structured.as_ref().ok())
        .collect();
    let body = body_font_size(&ok_pages);
    let fonts = font_stats(&ok_pages);
    let pitch = line_pitch_ratio(&ok_pages);
    let mut outline = Outline::read(doc);
    // Pass 1: running headers/footers need the neighbouring pages, so they are decided once over
    // the pages without table text (SP4) and applied page by page in pass 2. Each stripped page is
    // built twice (here and in pass 2): one stripped or changed page is alive at a time.
    let mut running = mark_running(
        raw.iter().filter_map(|r| {
            let page = r.structured.as_ref().ok()?;
            let tables = r.tables.as_deref().unwrap_or(&[]);
            let stripped = strip_table_text(page, tables);
            Some((stripped.map_or(Cow::Borrowed(page), Cow::Owned), tables))
        }),
        body,
    )
    .into_iter();
    let mut sections = Vec::with_capacity(n);
    let mut ranked: Vec<Vec<RankedHead>> = vec![Vec::new(); raw.len()];
    let mut decisions = Vec::with_capacity(n);
    #[cfg(feature = "ocr")]
    let mut unavailable: Vec<(PageDecision, String)> = Vec::new();
    let mut prev_last: Option<&DetectedTable> = None;
    // Pass 2: layout of each page.
    for (p, r) in raw.iter().enumerate() {
        count_page(&mut stats, p, r, &mut est);
        let mut section = Section::new(SectionKind::Page, p);
        let tables: &[DetectedTable] = r.tables.as_deref().unwrap_or(&[]);
        let mut columns = false;
        let diag = match &r.structured {
            Err(_) => r.diag.clone(),
            Ok(page) => {
                let stripped = strip_table_text(page, tables);
                let stripped = stripped.as_ref().unwrap_or(page);
                let moves = running.next().unwrap_or_default();
                let marked = (!moves.is_empty()).then(|| move_out(stripped, &moves));
                let mut ctx = PageContext {
                    page: p,
                    tables,
                    bullets: &r.bullets,
                    outline: &mut outline,
                    ranked: Vec::new(),
                };
                let pb = page_to_blocks_with(
                    marked.as_ref().unwrap_or(stripped),
                    body,
                    &fonts,
                    pitch,
                    &mut ctx,
                    &cfg.layout,
                );
                ranked[p] = ctx.ranked;
                section.blocks = pb.blocks;
                columns = pb.columns;
                if let (Some(prev), Some(first)) = (prev_last, tables.first())
                    && continues(prev, first, page.page_height)
                {
                    section.warnings.push(Warning::new(
                        WarningKind::TableUncertain,
                        Level::Info,
                        format!("table continues from page {p} (fragments are not merged)"),
                    ));
                }
                r.diag.clone()
            }
        };
        let mut warnings = std::mem::take(&mut section.warnings);
        section.warnings = diag.warnings;
        section.warnings.append(&mut warnings);
        section.warnings.extend(r.hidden.warnings());
        match &r.tables {
            Ok(v) => {
                place_tables(&mut section.blocks, v);
                section
                    .warnings
                    .extend(v.iter().flat_map(|t| t.warnings.iter().cloned()));
            }
            Err(e) => section.warnings.push(Warning::new(
                WarningKind::TableUncertain,
                Level::Warn,
                format!("table detection failed: {e}"),
            )),
        }
        // XY-cut on pages with columns only: one-column pages keep the region order (diagram
        // labels and figures move under XY-cut, SP8 task 9)
        if cfg.layout.reading_order == ReadingOrderMode::XyCut
            && columns
            && !order_blocks(&mut section.blocks, body)
        {
            section.warnings.push(Warning::new(
                WarningKind::PartialContent,
                Level::Info,
                "XY-cut reading order skipped (blocks over the limit or without finite boxes): region order kept",
            ));
        }
        prev_last = tables.last();
        section.quality = diag.quality;
        // The layer's tables went with the layer: no "table continues" from them.
        #[cfg(feature = "ocr")]
        if apply_ocr(
            &mut section,
            r.ocr.as_ref(),
            opts.hidden_text,
            &mut unavailable,
        ) {
            prev_last = None;
        }
        add_links(&mut section, &r.links);
        sections.push(section);
        decisions.push(diag.decision);
    }

    rank_levels(&mut sections, &ranked, body, cfg.layout.h2_ratio);
    // Document warnings.
    let mut document = Document::new(sections);
    if opened.truncated {
        document.warnings.push(Warning::new(
            WarningKind::Truncated,
            Level::Warn,
            "no %%EOF in the last 1024 bytes; the file looks truncated",
        ));
    }
    if opened.encrypted {
        document.warnings.push(Warning::new(
            WarningKind::Encrypted,
            Level::Info,
            "document is encrypted; opened with the supplied or empty user password",
        ));
    }
    if n < total {
        document.warnings.push(Warning::new(
            WarningKind::PartialContent,
            Level::Warn,
            format!("processed {n} of {total} pages (max_pages)"),
        ));
        stats.pages.skipped = total - n;
        stats.hit(LimitKind::MaxPages, None);
    }
    #[cfg(feature = "ocr")]
    if pass.done < pass.wanted {
        document.warnings.push(Warning::new(
            WarningKind::PartialContent,
            Level::Warn,
            format!(
                "OCR limited to {} of {} pages that need it",
                pass.done, pass.wanted
            ),
        ));
        stats.hit(LimitKind::OcrMaxPages, None);
    }
    if n > 0
        && raw
            .iter()
            .all(|r| matches!(&r.classify, ClassifyOutcome::Facts(f) if f.kind == FactKind::Empty))
    {
        document.warnings.push(Warning::new(
            WarningKind::EmptyOutput,
            Level::Warn,
            "document contains no text",
        ));
    }
    #[cfg(not(feature = "ocr"))]
    let missing = (
        ocr_needs(&decisions, &cfg.diagnostics),
        "OCR is not compiled in (feature `ocr`)",
        false,
    );
    // Pages whose OCR ran but was unavailable, under the decision they ran with; the reason is the
    // first page's. With an engine set, unavailable means its models could not be loaded.
    #[cfg(feature = "ocr")]
    let missing = (
        unavailable.iter().map(|u| u.0).collect::<Vec<_>>(),
        unavailable.first().map_or("", |u| u.1.as_str()),
        cfg.ocr.engine.is_some(),
    );
    if let Some(w) = ocr_unavailable(&missing.0, opts, missing.1, missing.2) {
        stats.ocr_unavailable = true;
        telemetry::ocr_unavailable();
        document.warnings.push(w);
    }
    document.warnings.extend(bigrams_missing(&cfg.diagnostics));

    document.metadata.page_count = Some(total);
    apply_quality(&mut document, &opts.quality);
    #[cfg(feature = "ocr")]
    {
        stats.pages.ocr_wanted = pass.wanted;
        stats.pages.ocr_skipped = pass
            .wanted
            .saturating_sub(stats.pages.ocr_run.saturating_add(stats.pages.ocr_failed));
        est.raster_max = est.raster_max.saturating_mul(pass.workers as u64);
    }
    stats.sizes.est_work_bytes = est
        .pages
        .saturating_add(est.table_max)
        .saturating_add(est.raster_max);
    stats.add_stage(Stage::Assemble, assemble.elapsed());
    Ok(ParsedPdf {
        document,
        decisions,
        ran_parallel: parallel,
        stats,
    })
}

#[cfg(test)]
#[path = "../pipeline_test.rs"]
mod tests;
