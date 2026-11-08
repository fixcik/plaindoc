//! Extension point for table detection and the own detector (SP4).

use pdf_oxide::PdfDocument;
use pdf_oxide::structured::StructuredPage;
use plaindoc_core::{BBox, Block, HiddenText, LimitKind, ParseError, Result, Warning};

use super::hidden::{HiddenCounts, PageFacts, filter_words};
use super::table_input::{limit, page_input, page_origin};
use crate::tables::{
    DetectedGrid, InputSegment, InputWord, PageInput, Rect, TableConfig, detect_tables, guarded,
    to_bbox, to_block,
};

/// One OCR word for table detection. Coordinates: pt in the *aligned* frame of the visible page —
/// after `/Rotate` and after deskew/orientation, origin bottom-left, y up.
#[derive(Clone, Debug, PartialEq)]
pub struct ScanWord {
    pub text: String,
    pub bbox: BBox,
    /// Per-character x-ranges [x0, x1) in pt, from CTC steps (empty if unknown). SP4: `InputWord.char_x`.
    pub char_x: Vec<(f32, f32)>,
    /// Recognition confidence; informational (SP4 does not use it).
    pub conf: f32,
    /// Ink-based font size in pt (±20–30 % on scans).
    pub font_size: f32,
    pub line: u32,
    pub block: u32,
    /// Degrees; SP4 sets `InputWord.rotated = |angle| > 45`.
    pub angle: f32,
}

/// Raster rule, axis-aligned in the same aligned frame (pt, y up). SP4 sets `InputSegment.raster = true`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScanSegment {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

/// Table-detector input of one scanned page.
#[derive(Clone, Debug, PartialEq)]
pub struct ScanTableInputs {
    pub page_index: usize,
    /// Size of the aligned visible page in pt (SP4 needs the height for `top = page_height - y`).
    pub page_width: f32,
    pub page_height: f32,
    pub words: Vec<ScanWord>,
    pub segments: Vec<ScanSegment>,
    /// Skew correction applied during alignment, degrees (0 when below the deskew threshold; the
    /// residual skew is not measured), and the orientation that was undone.
    pub skew_deg: f32,
    pub rotation: u32,
    /// Table regions of the region model (task R2) in the same aligned frame, with scores; empty
    /// without the model. Declared here so that R2 adds no field to a public struct.
    pub model_regions: Vec<(BBox, f32)>,
}

/// Detects tables on a single page.
pub trait TableDetector: Send + Sync {
    /// Called once per page, on a PDF_POOL thread, possibly concurrently for different pages.
    fn detect(&self, doc: &PdfDocument, page: &StructuredPage) -> Result<Vec<DetectedTable>>;

    /// [`TableDetector::detect`] that also reports the page's detector input in `counts`. The
    /// default leaves `counts` empty; [`OwnTables`] fills it.
    fn detect_counted(
        &self,
        doc: &PdfDocument,
        page: &StructuredPage,
        counts: &mut TableInputCounts,
    ) -> Result<Vec<DetectedTable>> {
        let _ = counts;
        self.detect(doc, page)
    }

    /// [`TableDetector::detect_counted`] with the page's hidden-text filter for the detector's own
    /// text. The default ignores `words` (a detector reads the page its own way); [`OwnTables`]
    /// applies it to its words before detection. A detector wrapping [`OwnTables`] must forward
    /// this method, or its words are not filtered.
    #[doc(hidden)]
    fn detect_filtered(
        &self,
        doc: &PdfDocument,
        page: &StructuredPage,
        counts: &mut TableInputCounts,
        words: &mut WordFilter<'_>,
    ) -> Result<Vec<DetectedTable>> {
        let _ = words;
        self.detect_counted(doc, page, counts)
    }

    /// Tables of a scanned page; bboxes of the result are in the same aligned frame as `inputs`.
    fn detect_scan(&self, _inputs: &ScanTableInputs) -> Result<Vec<DetectedTable>> {
        Ok(Vec::new())
    }
}

/// The page's hidden-text mode and facts for [`TableDetector::detect_filtered`], and the counts of
/// what the filter found in the detector's words.
#[doc(hidden)]
pub struct WordFilter<'a> {
    mode: HiddenText,
    facts: &'a PageFacts,
    pub(crate) counts: HiddenCounts,
}

impl<'a> WordFilter<'a> {
    pub(crate) fn new(mode: HiddenText, facts: &'a PageFacts) -> Self {
        Self {
            mode,
            facts,
            counts: HiddenCounts::default(),
        }
    }
}

/// Size of one page's table-detector input and how detection ended, for `ParseStats`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TableInputCounts {
    /// Words handed to the detector.
    pub words: usize,
    /// Paths plus segments handed to the detector.
    pub prims: usize,
    /// The hostile-input limit that skipped the page.
    pub limit: Option<LimitKind>,
    /// The detector panicked (the page got "table detection failed").
    pub panicked: bool,
}

/// A table found on a page: its region, the ready-made block and its page warnings.
#[derive(Clone, Debug, PartialEq)]
pub struct DetectedTable {
    /// Table region on the page.
    pub bbox: BBox,
    /// Block placed among the page blocks.
    pub block: Block,
    /// Page warnings about this table (`TableUncertain`).
    pub warnings: Vec<Warning>,
    /// The part of `bbox` whose text the table holds: page text there is removed from the prose, text
    /// elsewhere in `bbox` stays (words the grid did not take; final review C1). `new` sets `bbox`.
    pub text_bbox: BBox,
}

impl DetectedTable {
    /// Creates a detected table from its region and block.
    pub fn new(bbox: BBox, block: Block) -> Self {
        Self {
            bbox,
            block,
            warnings: Vec::new(),
            text_bbox: bbox,
        }
    }
}

/// Detector that finds no tables.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoTables;

impl TableDetector for NoTables {
    fn detect(&self, _doc: &PdfDocument, _page: &StructuredPage) -> Result<Vec<DetectedTable>> {
        Ok(Vec::new())
    }
}

/// The own detector of `crate::tables` on the page words and vector primitives.
#[derive(Clone, Debug, Default)]
pub struct OwnTables {
    pub config: TableConfig,
}

impl TableDetector for OwnTables {
    fn detect(&self, doc: &PdfDocument, page: &StructuredPage) -> Result<Vec<DetectedTable>> {
        self.detect_counted(doc, page, &mut TableInputCounts::default())
    }

    fn detect_counted(
        &self,
        doc: &PdfDocument,
        page: &StructuredPage,
        counts: &mut TableInputCounts,
    ) -> Result<Vec<DetectedTable>> {
        self.detect_with(doc, page, counts, None)
    }

    fn detect_filtered(
        &self,
        doc: &PdfDocument,
        page: &StructuredPage,
        counts: &mut TableInputCounts,
        words: &mut WordFilter<'_>,
    ) -> Result<Vec<DetectedTable>> {
        self.detect_with(doc, page, counts, Some(words))
    }

    fn detect_scan(&self, i: &ScanTableInputs) -> Result<Vec<DetectedTable>> {
        let input = scan_page_input(i);
        let cfg = scan_config(&self.config);
        // `guarded` errs only on a detector panic.
        let found = guarded(|| detect_tables(&input, &cfg))?;
        if let Some(l) = found.skipped {
            return Err(limit(l.what, l.count, l.limit));
        }
        // Aligned frame, y up.
        Ok(found
            .tables
            .iter()
            .map(|t| to_detected(t, input.height))
            .collect())
    }
}

impl OwnTables {
    fn detect_with(
        &self,
        doc: &PdfDocument,
        page: &StructuredPage,
        counts: &mut TableInputCounts,
        filter: Option<&mut WordFilter<'_>>,
    ) -> Result<Vec<DetectedTable>> {
        let mut input =
            page_input(doc, page.page_index).inspect_err(|e| counts.limit = adapter_limit(e))?;
        let (ox, top_y) = page_origin(doc, page.page_index);
        if let Some(f) = filter {
            f.counts = filter_words(&mut input.words, f.mode, f.facts, (ox, top_y));
        }
        counts.words = input.words.len();
        counts.prims = input.paths.len().saturating_add(input.segments.len());
        // `guarded` errs only on a detector panic.
        let found = guarded(|| detect_tables(&input, &self.config))
            .inspect_err(|_| counts.panicked = true)?;
        if let Some(l) = found.skipped {
            counts.limit = Some(table_limit_kind(l.what));
            return Err(limit(l.what, l.count, l.limit));
        }
        Ok(found
            .tables
            .into_iter()
            .map(|mut t| {
                if ox != 0.0 {
                    shift_x(&mut t, ox);
                }
                to_detected(&t, top_y)
            })
            .collect())
    }
}

/// The detector config of `detect_scan`: `cfg` with the scan-only guards on.
pub(crate) fn scan_config(cfg: &TableConfig) -> TableConfig {
    TableConfig {
        scan_text_guards: true,
        ..cfg.clone()
    }
}

/// Detector input of a scanned page: top-left origin, raster rules (`raster: true`). `model_regions`
/// are not fed (task R2).
pub(crate) fn scan_page_input(i: &ScanTableInputs) -> PageInput {
    let h = f64::from(i.page_height);
    let rect = |b: &BBox| Rect::new(b.x.into(), h - f64::from(b.y + b.h), b.w.into(), b.h.into());
    PageInput {
        width: i.page_width.into(),
        height: h,
        words: i
            .words
            .iter()
            .map(|w| InputWord {
                rect: rect(&w.bbox),
                text: w.text.clone(),
                font_size: w.font_size.into(),
                char_x: w
                    .char_x
                    .iter()
                    .map(|&(a, b)| (a.into(), b.into()))
                    .collect(),
                rotated: w.angle.abs() > 45.0,
            })
            .collect(),
        segments: i
            .segments
            .iter()
            .map(|s| InputSegment {
                x1: s.x1.into(),
                y1: h - f64::from(s.y1),
                x2: s.x2.into(),
                y2: h - f64::from(s.y2),
                raster: true,
            })
            .collect(),
        ..Default::default()
    }
}

/// `LimitExceeded::what` of `detect_tables`, `grid`, `lattice` and `page_input_with` → `LimitKind`.
pub(crate) fn table_limit_kind(what: &str) -> LimitKind {
    if what.starts_with("words") {
        LimitKind::TableWords
    } else if what.starts_with("cells") {
        LimitKind::TableCells
    } else if what.starts_with("model regions") {
        LimitKind::TableModelRegions
    } else {
        LimitKind::TablePrims
    }
}

/// The limit of a `page_input` error (`table_input::limit` prefixes "table detector: ").
fn adapter_limit(e: &ParseError) -> Option<LimitKind> {
    match e {
        ParseError::Limit { kind, .. } => {
            kind.strip_prefix("table detector: ").map(table_limit_kind)
        }
        _ => None,
    }
}

/// Page-local → PDF x (nested grids included); y is handled by `page_height` = top edge of the MediaBox.
fn shift_x(t: &mut DetectedGrid, dx: f64) {
    t.bbox[0] += dx;
    t.bbox[2] += dx;
    t.cells[0] += dx;
    t.cells[2] += dx;
    for n in t.rows.iter_mut().flatten().flat_map(|c| &mut c.nested) {
        shift_x(n, dx);
    }
}

pub(crate) fn to_detected(t: &DetectedGrid, page_height: f64) -> DetectedTable {
    let (bbox, block, warnings) = to_block(t, page_height);
    DetectedTable {
        bbox,
        block,
        warnings,
        text_bbox: to_bbox(t.cells, page_height),
    }
}

fn columns(b: &Block) -> u32 {
    match b {
        Block::Table { rows, .. } => rows
            .iter()
            .map(|r| r.iter().map(|c| c.colspan).sum())
            .max()
            .unwrap_or(0),
        _ => 0,
    }
}

/// Share of the page height near the bottom (previous page) and the top (next page) where a table
/// fragment counts as continued.
const CONTINUE_EDGE: f32 = 0.25;

/// A multi-page table fragment: the last table of the previous page ends near its bottom, the first table
/// of this page starts near its top, and both have the same column count. Fragments are not merged.
pub(crate) fn continues(prev: &DetectedTable, next: &DetectedTable, page_height: f32) -> bool {
    let c = columns(&prev.block);
    c > 0
        && c == columns(&next.block)
        && prev.bbox.y <= CONTINUE_EDGE * page_height
        && next.bbox.y + next.bbox.h >= (1.0 - CONTINUE_EDGE) * page_height
}

#[cfg(test)]
#[path = "tables_test.rs"]
mod tests;
