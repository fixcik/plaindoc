//! Per-parse statistics, always collected (no dependencies beyond `serde`).
//!
//! Content-free by construction: counts, durations, sizes and enum codes only. Never document
//! text, warning messages, error strings, metadata (title, author) or input paths.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;

use crate::{Block, Document, Format, Level, ParseError, Result, TextSource, WarningKind};

/// Pipeline stage. PDF: `Open` = `open_input`; `Text` = `extract_structured` + `classify_page`;
/// `Tables` = table detector with its input; `Diagnose` = stage-1 diagnosis including the need-OCR
/// classifier (50-dpi render); `Ocr` = OCR pass; `Assemble` = stage B and document warnings.
/// Other formats report no stages yet (only `wall_us`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Stage {
    Open,
    Text,
    Tables,
    Diagnose,
    Ocr,
    Assemble,
}

impl Stage {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Stage::Open => "open",
            Stage::Text => "text",
            Stage::Tables => "tables",
            Stage::Diagnose => "diagnose",
            Stage::Ocr => "ocr",
            Stage::Assemble => "assemble",
        }
    }
}

/// A limit or timeout that cut work short. Document limits count 1 per document, page limits
/// count pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LimitKind {
    /// `ParseError::TooLarge`: input over `max_size` (or `max_text_bytes`).
    InputSize,
    /// `ParseError::Limit` from a parser (decompression, XML nodes, table cells, image pixels).
    ParserLimit,
    /// `ParseOptions::timeout` (`ParseError::Timeout`).
    DocumentTimeout,
    /// Pages past `ParseOptions::max_pages` were not processed.
    MaxPages,
    /// Pages that need OCR past `ParseOptions::ocr_max_pages` were not OCRed.
    OcrMaxPages,
    /// A page's OCR hit `ParseOptions::ocr_page_timeout`.
    OcrPageTimeout,
    /// A page's OCR dropped boxes over `OcrParams::max_boxes`.
    OcrMaxBoxes,
    /// The table detector skipped a page: words over the cap.
    TableWords,
    /// ... paths, segments or derived primitives over the cap.
    TablePrims,
    /// ... a grid over the cell cap.
    TableCells,
    /// ... layout-model regions over the cap.
    TableModelRegions,
}

impl LimitKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            LimitKind::InputSize => "input_size",
            LimitKind::ParserLimit => "parser_limit",
            LimitKind::DocumentTimeout => "document_timeout",
            LimitKind::MaxPages => "max_pages",
            LimitKind::OcrMaxPages => "ocr_max_pages",
            LimitKind::OcrPageTimeout => "ocr_page_timeout",
            LimitKind::OcrMaxBoxes => "ocr_max_boxes",
            LimitKind::TableWords => "table_words",
            LimitKind::TablePrims => "table_prims",
            LimitKind::TableCells => "table_cells",
            LimitKind::TableModelRegions => "table_model_regions",
        }
    }
}

/// How the parse ended: `Ok` or the `ParseError` variant (never its message).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Outcome {
    #[default]
    Ok,
    Unsupported,
    Corrupt,
    Encrypted,
    TooLarge,
    Limit,
    Timeout,
    Io,
    Internal,
}

impl Outcome {
    #[must_use]
    pub fn of(result: &Result<Document>) -> Self {
        match result {
            Ok(_) => Outcome::Ok,
            Err(e) => Self::of_error(e),
        }
    }

    #[must_use]
    pub fn of_error(e: &ParseError) -> Self {
        match e {
            ParseError::Unsupported(_) => Outcome::Unsupported,
            ParseError::Corrupt(_) => Outcome::Corrupt,
            ParseError::Encrypted => Outcome::Encrypted,
            ParseError::TooLarge { .. } => Outcome::TooLarge,
            ParseError::Limit { .. } => Outcome::Limit,
            ParseError::Timeout(_) => Outcome::Timeout,
            ParseError::Io(_) => Outcome::Io,
            ParseError::Internal(_) => Outcome::Internal,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Unsupported => "unsupported",
            Outcome::Corrupt => "corrupt",
            Outcome::Encrypted => "encrypted",
            Outcome::TooLarge => "too_large",
            Outcome::Limit => "limit",
            Outcome::Timeout => "timeout",
            Outcome::Io => "io",
            Outcome::Internal => "internal",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct LevelCounts {
    pub info: u32,
    pub warn: u32,
    pub loss: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct PageCounts {
    /// Pages (sheets, slides) in the source: `metadata.page_count`, else the sections. Office
    /// formats report document pages here but give one `Body` section: `total != processed` is not
    /// a loss, `skipped` is.
    pub total: usize,
    /// Sections in the result.
    pub processed: usize,
    /// Not processed because of `max_pages` (PDF).
    pub skipped: usize,
    /// Text extraction failed (PDF: `extract_structured` error, the page carries a loss warning).
    pub failed: usize,
    /// Sections whose text is OCR (`TextSource::Ocr`) / text layer plus OCR (`TextSource::Mixed`).
    pub ocr: usize,
    pub mixed: usize,
    /// PDF OCR pass: pages that needed OCR, OCR that finished, wanted but not run (`ocr_max_pages`
    /// or no engine/models), OCR that failed.
    pub ocr_wanted: usize,
    pub ocr_run: usize,
    pub ocr_skipped: usize,
    pub ocr_failed: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Sizes {
    /// Blocks at every depth (list items and table cells included).
    pub blocks: u64,
    /// Whitespace-separated words of all text (`Image::ocr_text` included).
    pub words: u64,
    /// UTF-8 bytes of that text.
    pub text_bytes: u64,
    pub tables: u64,
    /// Source cells of all tables (positions under a span are not repeated, as in the model).
    pub cells: u64,
    /// PDF: text spans of all pages after extraction.
    pub spans: u64,
    /// PDF: table-detector input, summed over pages and the largest page.
    pub table_words: u64,
    pub table_words_max: u64,
    pub table_prims: u64,
    pub table_prims_max: u64,
    /// Parser's estimate of its working bytes at the peak (see `ParseStats::est_peak_bytes`).
    pub est_work_bytes: u64,
}

/// Statistics of one parse (see the module docs for what never goes in).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ParseStats {
    /// Detected format; `None` when detection failed.
    pub format: Option<Format>,
    pub outcome: Outcome,
    pub input_bytes: u64,
    /// Wall time of the whole parse, detection included.
    pub wall_us: u64,
    /// Busy time per stage summed over pages; with page parallelism it can exceed `wall_us`.
    pub stages_us: BTreeMap<Stage, u64>,
    /// Pages of the document were processed in parallel.
    pub page_parallel: bool,
    pub pages: PageCounts,
    pub limits: BTreeMap<LimitKind, u32>,
    /// Warnings of the document and of its sections, by kind and level.
    pub warnings: BTreeMap<WarningKind, LevelCounts>,
    pub sizes: Sizes,
    /// Panics caught below the document level after which the document survived (PDF: the table
    /// detector of a page). A panic of the whole parse is `outcome: internal`.
    pub panics_isolated: u32,
    /// Pages needed OCR but the engine or its models were unavailable.
    pub ocr_unavailable: bool,
    /// Rough, deterministic estimate of the peak bytes held for this document, not RSS:
    /// `input_bytes + max(sizes.est_work_bytes, sizes.text_bytes)`.
    ///
    /// Usually a lower estimate: the measured RSS growth over an empty run (macOS, release, PDF)
    /// was about 10x this value for a 201-page text document and about 46x for a one-page
    /// document with tables (fixed parser overhead dominates), but about 0.7x for an image-only
    /// page (measured without OCR models).
    pub est_peak_bytes: u64,
}

pub(crate) fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

impl ParseStats {
    /// Adds `d` to the busy time of `stage`.
    pub fn add_stage(&mut self, stage: Stage, d: Duration) {
        let t = self.stages_us.entry(stage).or_default();
        *t = t.saturating_add(micros(d));
    }

    /// Counts one hit of `kind` and emits the `limit_hit` event (page index when page-level).
    pub fn hit(&mut self, kind: LimitKind, page: Option<usize>) {
        let n = self.limits.entry(kind).or_default();
        *n = n.saturating_add(1);
        crate::telemetry::limit_hit(kind, page);
    }

    /// Post-hoc counters from the final document: pages total/processed/ocr/mixed, sizes
    /// (blocks, words, text bytes, tables, cells) and warnings. Idempotent: replaces these fields,
    /// leaves the parser's in-flight counters alone.
    pub fn fill_from_document(&mut self, doc: &Document) {
        let mut t = Tally::default();
        for s in &doc.sections {
            t.blocks(&s.blocks);
        }
        self.sizes.blocks = t.blocks;
        self.sizes.words = t.words;
        self.sizes.text_bytes = t.text_bytes;
        self.sizes.tables = t.tables;
        self.sizes.cells = t.cells;
        self.pages.processed = doc.sections.len();
        self.pages.total = doc.metadata.page_count.unwrap_or(doc.sections.len());
        let src = |k: TextSource| doc.sections.iter().filter(|s| s.source == k).count();
        self.pages.ocr = src(TextSource::Ocr);
        self.pages.mixed = src(TextSource::Mixed);
        self.warnings.clear();
        let all = doc
            .warnings
            .iter()
            .chain(doc.sections.iter().flat_map(|s| &s.warnings));
        for w in all {
            let c = self.warnings.entry(w.kind).or_default();
            let n = match w.level {
                Level::Info => &mut c.info,
                Level::Warn => &mut c.warn,
                Level::Loss => &mut c.loss,
            };
            *n = n.saturating_add(1);
        }
    }

    /// Final step of the registry: outcome, post-hoc counters or the document-level limit of the
    /// error, memory estimate and wall time.
    pub fn finish(&mut self, result: &Result<Document>, wall: Duration) {
        self.outcome = Outcome::of(result);
        match result {
            Ok(doc) => self.fill_from_document(doc),
            Err(ParseError::TooLarge { .. }) => self.hit(LimitKind::InputSize, None),
            Err(ParseError::Limit { .. }) => self.hit(LimitKind::ParserLimit, None),
            Err(ParseError::Timeout(_)) => self.hit(LimitKind::DocumentTimeout, None),
            Err(_) => {}
        }
        self.est_peak_bytes = self
            .input_bytes
            .saturating_add(self.sizes.est_work_bytes.max(self.sizes.text_bytes));
        self.wall_us = micros(wall);
    }

    /// Takes the parser's in-flight counters, keeping the registry's `format` and `input_bytes`.
    pub fn absorb(&mut self, parser: ParseStats) {
        let (format, input_bytes) = (self.format, self.input_bytes);
        *self = parser;
        self.format = format;
        self.input_bytes = input_bytes;
    }

    /// Copy with every duration zeroed (`wall_us`, `stages_us` values; the stage keys stay):
    /// equal across runs and thread counts.
    #[must_use]
    pub fn without_timings(&self) -> Self {
        let mut s = self.clone();
        s.wall_us = 0;
        s.stages_us.values_mut().for_each(|v| *v = 0);
        s
    }

    /// One-line JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("ParseStats is always serializable")
    }
}

#[derive(Default)]
struct Tally {
    blocks: u64,
    words: u64,
    text_bytes: u64,
    tables: u64,
    cells: u64,
}

impl Tally {
    fn text(&mut self, t: &str) {
        self.words = self
            .words
            .saturating_add(t.split_whitespace().count() as u64);
        self.text_bytes = self.text_bytes.saturating_add(t.len() as u64);
    }

    fn blocks(&mut self, blocks: &[Block]) {
        for b in blocks {
            self.blocks = self.blocks.saturating_add(1);
            match b {
                Block::Heading { text, .. }
                | Block::Paragraph { text, .. }
                | Block::Code { text, .. }
                | Block::Note { text, .. } => self.text(text),
                Block::List { items, .. } => {
                    for i in items {
                        self.blocks(&i.blocks);
                    }
                }
                Block::Table { rows, .. } => {
                    self.tables = self.tables.saturating_add(1);
                    for c in rows.iter().flatten() {
                        self.cells = self.cells.saturating_add(1);
                        self.blocks(&c.blocks);
                    }
                }
                Block::Image { ocr_text, .. } => {
                    if let Some(t) = ocr_text {
                        self.text(t);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "stats_test.rs"]
mod tests;
