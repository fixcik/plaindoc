//! Stage-1 page diagnostics: `classify_page` facts plus our garbage score become
//! page warnings, a page decision and a page quality.
//!
//! All decision logic (rules table, decision, quality) lives here behind
//! `diagnose_page`, `extraction_failed` and `ocr_unavailable`. Two paths:
//! - `DiagnosticsConfig::need_ocr = Some(cfg)` (default, `RECALL` with the bundled bigram table):
//!   the need-OCR classifier (`need_ocr::decide`) plus the SP2 mojibake safety net;
//! - `None`: the SP2 rules table on engine facts.

use std::sync::Arc;

use pdf_oxide::extractors::auto::{PageClassification, PageKind, ReasonCode};
use plaindoc_core::{Level, OcrMode, ParseOptions, Warning, WarningKind};

use super::garbage::GarbageScore;
use super::need_ocr::{Bigrams, EngineHints, NeedOcrConfig, NeedReason, PageSignals, decide};

/// Thresholds for stage-1 page diagnostics.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct DiagnosticsConfig {
    /// Minimum scored tokens before the garbage ratio is trusted.
    pub garbage_min_tokens: usize,
    /// Mojibake token ratio at which a text layer counts as garbage.
    pub garbage_ratio: f32,
    /// Engine garbled-char ratio at which a scanned page's text layer is rejected.
    pub engine_garbled_ratio: f32,
    /// Image area ratio at which a scanned page with text needs OCR merged in.
    pub scanned_image_ratio: f32,
    /// Need-OCR classifier switch: `Some` (default [`NeedOcrConfig::RECALL`]) decides with the
    /// classifier at these thresholds; `None` keeps the SP2 rules table above. On by default by
    /// the user's decision with the bigram table (task 14a gate: R2 recall 0.973, see `RECALL`).
    ///
    /// With `None`, OCR needs under `OcrMode::Auto` are only `RunOcr` and `Reject` pages: the SP2
    /// `RunOcrAndMerge` of ImageText/Mixed pages is not a reliable need-OCR signal (ruling Р-6:
    /// no true positives on 203 pages). `OcrMode::Always` counts every page either way.
    pub need_ocr: Option<NeedOcrConfig>,
    /// Bigram table for the classifier's prose-garbage rule; `None` skips that clause (and with
    /// the classifier on, each PDF gets one Info warning). Default: [`Bigrams::bundled`].
    pub bigrams: Option<Arc<Bigrams>>,
    /// With the classifier on: a kept layer on a page with images and at least this many
    /// glyph-like components outside the layer gets `ImageTextPage`/Info (text in images below
    /// the OCR threshold `g_abs` is reported, not lost silently).
    pub image_text_min_cc: u32,
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            garbage_min_tokens: 20,
            garbage_ratio: 0.20,
            engine_garbled_ratio: 0.05,
            scanned_image_ratio: 0.50,
            need_ocr: Some(NeedOcrConfig::RECALL),
            bigrams: Bigrams::bundled(),
            image_text_min_cc: 20,
        }
    }
}

/// Engine page kind, reduced to the cases the rules table distinguishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FactKind {
    TextLayer,
    Scanned,
    ImageText,
    Mixed,
    Empty,
}

/// The `classify_page` signals the rules table reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PageFacts {
    pub kind: FactKind,
    pub no_text_layer: bool,
    pub glyphs: usize,
    pub image_area_ratio: f32,
    pub garbled_ratio: f32,
    pub vector_path_count: usize,
}

impl From<&PageClassification> for PageFacts {
    fn from(c: &PageClassification) -> Self {
        let kind = match c.kind {
            PageKind::TextLayer => FactKind::TextLayer,
            PageKind::Scanned => FactKind::Scanned,
            PageKind::ImageText => FactKind::ImageText,
            PageKind::Empty => FactKind::Empty,
            _ => FactKind::Mixed,
        };
        Self {
            kind,
            no_text_layer: c.reason == ReasonCode::NoTextLayerPresent,
            glyphs: c.signals.text_glyph_count,
            image_area_ratio: c.signals.image_area_ratio,
            garbled_ratio: c.signals.garbled_ratio,
            vector_path_count: c.signals.vector_path_count,
        }
    }
}

impl PageFacts {
    /// Engine signals the classifier falls back on when the page is not rendered.
    pub(crate) fn hints(&self) -> EngineHints {
        EngineHints {
            scanned_no_layer: self.no_text_layer,
            image_area_ratio: self.image_area_ratio,
            vector_path_count: self.vector_path_count,
        }
    }
}

/// What to do with a page's text: keep it, replace it with OCR, merge OCR in, or drop the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PageDecision {
    UseTextLayer,
    RunOcr,
    RunOcrAndMerge,
    Reject,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PageDiagnosis {
    pub warnings: Vec<Warning>,
    pub decision: PageDecision,
    pub quality: f32,
}

/// `classify_page` result; a failure keeps only its message.
pub(crate) enum ClassifyOutcome {
    Facts(PageFacts),
    Failed(String),
}

/// Applies the stage-1 rules: with `cfg.need_ocr` set and the page's classifier signals `s`
/// given, the need-OCR classifier (`classifier_row`); otherwise the SP2 rules table, where the
/// first matching row wins.
pub(crate) fn diagnose_page(
    c: &ClassifyOutcome,
    s: Option<&PageSignals>,
    g: GarbageScore,
    dropped_regions: usize,
    dropped_chars: usize,
    cfg: &DiagnosticsConfig,
) -> PageDiagnosis {
    use PageDecision::*;
    let garbage = g.tokens >= cfg.garbage_min_tokens && g.ratio >= cfg.garbage_ratio;
    let (warning, decision, quality) = match (c, s.zip(cfg.need_ocr.as_ref())) {
        (ClassifyOutcome::Failed(e), _) => (
            Some(Warning::new(
                WarningKind::PartialContent,
                Level::Warn,
                format!("page classification failed: {e}"),
            )),
            UseTextLayer,
            1.0,
        ),
        (ClassifyOutcome::Facts(_), Some((s, nc))) => classifier_row(s, nc, g, garbage, cfg),
        (ClassifyOutcome::Facts(f), None) => {
            // Formats the signals only when a warning is actually emitted.
            let w = |kind, level, what: &str| {
                Some(Warning::new(
                    kind,
                    level,
                    format!(
                        "{what} ({} glyphs, {:.2} image area, {:.2} garbled, {} tokens, {:.2} mojibake ratio)",
                        f.glyphs, f.image_area_ratio, f.garbled_ratio, g.tokens, g.ratio
                    ),
                ))
            };
            match f.kind {
                FactKind::Empty => (
                    w(WarningKind::EmptyOutput, Level::Info, "blank page"),
                    UseTextLayer,
                    1.0,
                ),
                FactKind::Scanned if f.no_text_layer => (
                    w(
                        WarningKind::NoTextLayer,
                        Level::Loss,
                        "scanned page without a text layer",
                    ),
                    RunOcr,
                    0.0,
                ),
                FactKind::Scanned if garbage || f.garbled_ratio >= cfg.engine_garbled_ratio => (
                    w(
                        WarningKind::SuspectedGarbage,
                        Level::Loss,
                        "scanned page with an unreadable text layer",
                    ),
                    RunOcr,
                    0.0,
                ),
                FactKind::Scanned if f.image_area_ratio >= cfg.scanned_image_ratio => (
                    w(
                        WarningKind::ImageTextPage,
                        Level::Warn,
                        "page image covers text that may be missing from the text layer",
                    ),
                    RunOcrAndMerge,
                    1.0,
                ),
                FactKind::Scanned => (
                    w(
                        WarningKind::SuspectedGarbage,
                        Level::Info,
                        "engine flagged the text layer, own check passed",
                    ),
                    UseTextLayer,
                    1.0,
                ),
                _ if garbage => (
                    w(
                        WarningKind::SuspectedGarbage,
                        Level::Loss,
                        "text layer looks like mojibake",
                    ),
                    RunOcr,
                    0.0,
                ),
                FactKind::ImageText | FactKind::Mixed => (
                    w(
                        WarningKind::ImageTextPage,
                        Level::Info,
                        "page has images that may contain text",
                    ),
                    RunOcrAndMerge,
                    1.0,
                ),
                FactKind::TextLayer => (None, UseTextLayer, 1.0),
            }
        }
    };
    let mut warnings: Vec<Warning> = warning.into_iter().collect();
    if dropped_regions > 0 {
        warnings.push(Warning::new(
            WarningKind::ContentDropped,
            Level::Info,
            format!("dropped {dropped_regions} artifact regions ({dropped_chars} chars)"),
        ));
    }
    PageDiagnosis {
        warnings,
        decision,
        quality,
    }
}

/// Need-OCR classifier verdict (rulings Р-8, В5-4) as a warning, a decision and a quality; the
/// SP2 mojibake check then overrides a "keep the layer" verdict.
fn classifier_row(
    s: &PageSignals,
    nc: &NeedOcrConfig,
    g: GarbageScore,
    garbage: bool,
    cfg: &DiagnosticsConfig,
) -> (Option<Warning>, PageDecision, f32) {
    use PageDecision::*;
    // Formats the signals only when a warning is actually emitted.
    let w = |kind, level, what: &str| {
        let ink = s.ink.map_or("not rendered".to_string(), |i| {
            format!(
                "{} glyph-like components outside the layer",
                i.glyph_cc_outside
            )
        });
        Some(Warning::new(
            kind,
            level,
            format!(
                "{what} ({} chars, {:.2} image cover, {ink}, {} tokens, {:.2} mojibake ratio)",
                s.text.nonws, s.image_cover, g.tokens, g.ratio
            ),
        ))
    };
    match decide(s, nc) {
        NeedReason::NoTextLayer => (
            w(
                WarningKind::NoTextLayer,
                Level::Loss,
                "page text exists only as an image",
            ),
            RunOcr,
            0.0,
        ),
        NeedReason::Blank => (
            w(WarningKind::EmptyOutput, Level::Info, "blank page"),
            UseTextLayer,
            1.0,
        ),
        NeedReason::GarbageLayer => (
            w(
                WarningKind::SuspectedGarbage,
                Level::Loss,
                "text layer looks like garbage",
            ),
            RunOcr,
            0.0,
        ),
        NeedReason::ScanLayerBad => (
            w(
                WarningKind::SuspectedGarbage,
                Level::Loss,
                "scanned page with an unreadable text layer",
            ),
            RunOcr,
            0.0,
        ),
        NeedReason::InkOutsideLayer => (
            w(
                WarningKind::ImageTextPage,
                Level::Warn,
                "page has text in images that is missing from the text layer",
            ),
            RunOcrAndMerge,
            1.0,
        ),
        NeedReason::ScanLayerOk | NeedReason::TextOk if garbage => (
            w(
                WarningKind::SuspectedGarbage,
                Level::Loss,
                "text layer looks like mojibake",
            ),
            RunOcr,
            0.0,
        ),
        NeedReason::ScanLayerOk | NeedReason::TextOk
            if s.image_cover > 0.0
                && s.ink
                    .is_some_and(|i| i.glyph_cc_outside >= cfg.image_text_min_cc) =>
        {
            (
                w(
                    WarningKind::ImageTextPage,
                    Level::Info,
                    "page may have text in images that is not in the text layer",
                ),
                UseTextLayer,
                1.0,
            )
        }
        NeedReason::ScanLayerOk | NeedReason::TextOk => (None, UseTextLayer, 1.0),
    }
}

/// Text extraction of the page failed: the page is lost.
pub(crate) fn extraction_failed(err: &str) -> PageDiagnosis {
    PageDiagnosis {
        warnings: vec![Warning::new(
            WarningKind::PartialContent,
            Level::Loss,
            format!("page text extraction failed: {err}"),
        )],
        decision: PageDecision::Reject,
        quality: 0.0,
    }
}

/// Document-level note that the classifier runs without its bigram table (a build without
/// `data/need_ocr/bigrams.json`, or `bigrams: None`): the prose-garbage rule is skipped, so
/// garbage text layers are caught less often (task 14a: R2 recall 0.945 instead of 0.973).
pub(crate) fn bigrams_missing(cfg: &DiagnosticsConfig) -> Option<Warning> {
    (cfg.need_ocr.is_some() && cfg.bigrams.is_none()).then(|| {
        Warning::new(
            WarningKind::SuspectedGarbage,
            Level::Info,
            "need-OCR classifier runs without its bigram table: garbage text layers are detected less reliably",
        )
    })
}

/// Page decisions as OCR needs. With the classifier on they are taken as they are. Off (SP2
/// table), only `RunOcr` and `Reject` pages need OCR (as full pages); `RunOcrAndMerge` is not a
/// reliable signal there (ruling Р-6) and counts as `UseTextLayer`.
pub(crate) fn ocr_needs(decisions: &[PageDecision], cfg: &DiagnosticsConfig) -> Vec<PageDecision> {
    use PageDecision::*;
    if cfg.need_ocr.is_some() {
        return decisions.to_vec();
    }
    decisions
        .iter()
        .map(|d| match d {
            RunOcr | Reject => RunOcr,
            UseTextLayer | RunOcrAndMerge => UseTextLayer,
        })
        .collect()
}

/// Document-level note that pages needing OCR did not get it: `reason` says why (not compiled
/// in, models missing). `Info` under `Auto` (page warnings already carry the loss), `Warn` under
/// `Always` (the caller asked for OCR, so every page counts). Nothing under `Never`. An engine
/// whose models are missing (`models_missing`) is a broken setup, not a choice: `Warn` under
/// `Auto` too, so the CLI shows the reason and the fix next to the empty page.
pub(crate) fn ocr_unavailable(
    decisions: &[PageDecision],
    opts: &ParseOptions,
    reason: &str,
    models_missing: bool,
) -> Option<Warning> {
    let count = |want| decisions.iter().filter(|d| **d == want).count();
    let (full, merge) = (
        count(PageDecision::RunOcr),
        count(PageDecision::RunOcrAndMerge),
    );
    let (level, n) = match opts.ocr {
        OcrMode::Never => return None,
        OcrMode::Always => (Level::Warn, decisions.len()),
        _ if models_missing => (Level::Warn, full + merge),
        _ => (Level::Info, full + merge),
    };
    (n > 0).then(|| {
        Warning::new(
            WarningKind::OcrUnavailable,
            level,
            format!(
                "{n} page(s) need OCR ({full} full, {merge} image regions) but OCR is unavailable: {reason}"
            ),
        )
    })
}

#[cfg(test)]
#[path = "diagnose_test.rs"]
mod tests;
