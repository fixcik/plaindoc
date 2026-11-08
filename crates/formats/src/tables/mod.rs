//! Source-agnostic table detector.
//!
//! Port of the spike prototype `docs/design/spikes/tables-corpus/scripts/proto.py` (detector,
//! features, planar partition) and of the filters fixed by D11. Input is a page of words with boxes plus
//! optional vector primitives; the PDF adapter lives in `crate::pdf::table_input`, OCR/raster sources
//! (SP5) fill the same [`PageInput`].
//!
//! Coordinates: points, origin at the top-left corner, y grows downwards (as in the prototype).

mod block;
mod detect;
#[cfg(any(test, feature = "tables-eval"))]
pub mod dump;
#[cfg(feature = "tables-eval")]
pub mod eval;
mod features;
mod gate;
mod grid;
mod lattice;
mod nest;
mod prims;
mod select;
mod split;
mod stats;

pub use block::{guarded, to_block};
#[cfg(feature = "pdf")]
pub(crate) use block::{panic_message, to_bbox};
pub use detect::CandidateKind;
pub use features::Features;
#[cfg(all(test, feature = "pdf"))]
pub(crate) use gate::area_on_page;
pub use gate::{
    GateFeatures, LARGE_IMAGE_SHARE, gate_features, has_large_image, may_contain_table,
};
pub use select::{DetectedGrid, GridCell, PageTables, RejectReason, Rejected, Structure};

/// Hostile-input limits (SP4 review C1), checked before any allocation that grows with them; over a
/// limit the page gets no tables and [`PageTables::skipped`] says why. Worst real page of the corpora:
/// 20 518 primitives (PMC2375034, 131 ms), 1 711 words; real tables have fewer than 2 000 cells.
pub const MAX_WORDS: usize = 20_000;
/// Paths plus segments on the page after clipping.
pub const MAX_PRIMS: usize = 25_000;
/// Rows × columns of one grid (`build_cells`, `lattice`).
pub const MAX_CELLS: usize = 10_000;
/// Hard caps of a PDF page before its detector input is built (final review I3): words as extracted, and
/// paths plus segments with each stroked `re` of a composite path counted as 4. Over a cap the page is
/// skipped before the copies are made. 4x the detector limits: [`MAX_PRIMS`] applies after clipping to the page
/// (content far outside it may still be clipped away), [`MAX_WORDS`] before clipping. [`detect_tables`] also
/// checks paths and segments against [`ADAPTER_MAX_PRIMS`] before clipping. Corpus maximum on the PDF path (ODL,
/// ICDAR, PT1M, synthetic; 6 093 pages): 2 787 words (syn_437), 20 518 paths and segments (PMC2375034, page
/// index 18).
pub const ADAPTER_MAX_WORDS: usize = 4 * MAX_WORDS;
/// See [`ADAPTER_MAX_WORDS`].
pub const ADAPTER_MAX_PRIMS: usize = 4 * MAX_PRIMS;

/// [`TableConfig::text_max_cols`] for wide tables (T14, proto2.py `tx_maxcols=40`), the default; the
/// reference keeps the prototype's 15. A 40-column block reaches [`MAX_CELLS`] at 251 rows.
pub const WIDE_TEXT_MAX_COLS: usize = 40;
/// [`TableConfig::text_max_cols`] of the prototype (proto2.py `tx_maxcols` default), kept by the reference.
pub(crate) const PROTO_TEXT_MAX_COLS: usize = 15;

/// A limit hit by [`detect_tables`]: `count` of `what` is over `limit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LimitExceeded {
    pub what: &'static str,
    pub count: usize,
    pub limit: usize,
}

impl LimitExceeded {
    fn check(what: &'static str, count: usize, limit: usize) -> Result<(), Self> {
        if count > limit {
            Err(Self { what, count, limit })
        } else {
            Ok(())
        }
    }
}

/// Axis-aligned box: left, top, width, height (top-left origin).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub top: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, top: f64, w: f64, h: f64) -> Self {
        Self { x, top, w, h }
    }
}

/// A word of the page text layer (or an OCR word).
#[derive(Clone, Debug, PartialEq)]
pub struct InputWord {
    pub rect: Rect,
    pub text: String,
    /// Font size in points; `0` means unknown (the word height is used).
    pub font_size: f64,
    /// Left/right x of each char of `text`, when the source knows them. Used to split words glued across
    /// cell borders (`VALIDTO`). Empty or of a different length than `text.chars()` means unknown.
    pub char_x: Vec<(f64, f64)>,
    /// The word is not upright (PDF: `rotation_degrees != 0`; OCR: word angle beyond ±45°). A candidate
    /// made mostly of such words is rejected ([`RejectReason::Rotated`]).
    pub rotated: bool,
}

/// A painted path summarised by its rendered box (single-subpath paths and fills).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputPath {
    pub rect: Rect,
    pub stroke: bool,
    pub fill: bool,
}

/// A straight segment of a multi-subpath stroked path (a whole grid drawn as one path), top-left origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputSegment {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    /// Found in a page image, not drawn by a PDF path; SP5's scan path sets `raster: true`. The raster rules of
    /// [`TableConfig::short_vertical_stacks`] apply to such segments only (Task 12(c)).
    pub raster: bool,
}

/// A table region found by a layout model (SP4-R1; the runner is SP5-R2), top-left origin. Built with
/// [`ModelRegion::new`] (SP5-R2 may add fields).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelRegion {
    pub rect: Rect,
    /// Model confidence in `0..=1` (the oracle feeds 1).
    pub score: f32,
}

impl ModelRegion {
    pub fn new(rect: Rect, score: f32) -> Self {
        Self { rect, score }
    }
}

/// Most [`PageInput::model_regions`] of one page (SP4-R1): a page holds a few tables and the region model
/// keeps a handful of table boxes above its threshold; over it the page gets no tables ([`PageTables::skipped`]).
pub const MAX_MODEL_REGIONS: usize = 32;

/// Everything the detector sees of one page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageInput {
    pub width: f64,
    pub height: f64,
    pub words: Vec<InputWord>,
    pub paths: Vec<InputPath>,
    pub segments: Vec<InputSegment>,
    /// Table regions of a layout model; empty without a model. Used only with
    /// [`TableConfig::region_model`].
    pub model_regions: Vec<ModelRegion>,
    /// Boxes of the images painted on the page, for the pre-model gate ([`has_large_image`], T18); empty
    /// unless the caller fills them (SP5-R2, only where [`may_contain_table`] says no). The detector ignores
    /// them; no effect on the PDF path until SP5-R2 fills them.
    pub image_boxes: Vec<Rect>,
}

/// Region-model settings (SP4-R1; the runner, SP5-R2, adds its fields).
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RegionModelConfig {
    /// Run the model only on pages where [`may_contain_table`] (then [`has_large_image`]) says yes (T18).
    /// Off: optional optimisation; T18 pass-through budget (≤ 55 % of PT1M pages) not met: 0.586 with the
    /// image rule; table-free pages 0.503 (docs/tables-eval.md, "T18 pre-model gate"). Read by the SP5-R2
    /// runner: no effect on the PDF path until SP5-R2 calls the gate.
    pub page_gate: bool,
    /// SP4-R3 bounds inside a region's grid: filled bands across the region's width give its row bounds
    /// (zebra), and with fewer than 2 valleys at 8 or more words per line the column bounds come from
    /// clusters of word edges. Off by default: not accepted on the oracle (docs/tables-eval.md, "SP4-R3").
    pub inner_bounds: bool,
}

/// Detector switches. [`TableConfig::reference`] reproduces the Python prototype (D11 pipeline); the
/// default adds rule groups, merging of collinear rulings, the A2 additions (fill stacks with their
/// caption cut, header-row extension, dense valleys, rule-group split) and T14-T17 (proto2 `c14`, decided
/// jointly for T15 and T17, docs/tables-eval.md). Nested tables and `header_rows` are
/// always on (they do not change the candidates the parity tests compare; with `header_split`, T17, the
/// header rows of the split header set `header_rows`); glued-word splitting is off by default (ruling В-4).
#[derive(Clone, Debug, PartialEq)]
pub struct TableConfig {
    /// Rules chart+tiny+sparse+twocol and the logistic model (D11).
    pub filters: bool,
    /// Minimum logistic-model probability to keep a candidate (D11: 0.10).
    pub logit_threshold: f64,
    /// Use the planar partition (merged cells) for lined tables when the two-sided row guard holds (D11).
    pub hybrid: bool,
    /// Split words that cross a vertical ruling at char boundaries (not in the prototype). Off by default
    /// (ruling В-4: no effect on the corpora, +1 false page, a second `prims` + `regions` pass).
    pub split_glued_words: bool,
    /// Regions of aligned horizontal rulings without verticals (not in the prototype).
    pub rule_groups: bool,
    /// Merge collinear rulings (Chrome draws a table rule as a chain of per-cell pieces; A1, not in the
    /// prototype's reference pipeline).
    pub merge_lines: bool,
    /// Stacks of filled bands (zebra rows, filled header/cell rows) as `T` candidates; lined regions inside a
    /// stack are dropped (A2, proto2.py `detect_fill_groups`). Accepted together with `fill_split` (A2b).
    pub fill_groups: bool,
    /// Rule groups and fill stacks take one column-aligned text line right above/below them (a header
    /// over the first rule; A2, proto2.py `extend_rows`).
    pub rg_extend: bool,
    /// Column valleys of rule groups and fill stacks from lines of at least three segments only (a total
    /// row spanning columns keeps the column; A2, proto2.py `col_valleys`).
    pub dense_valleys: bool,
    /// A rule group spanning two tables is cut at a wide gap holding a caption line (A2, proto2.py
    /// `rg_split`).
    pub rg_split: bool,
    /// The `rg_split` rule for fill stacks: a stack is cut at a wide gap holding a caption line (two
    /// cell-filled tables with a caption between them; A2b, not in proto2.py).
    pub fill_split: bool,
    /// A lined region takes rows, columns and the planar partition from the rulings of its own connected
    /// component only, so a nested table in a cell does not split the outer one (A3, proto2.py
    /// `comp_lines`).
    pub comp_lines: bool,
    /// A region takes the labels standing left of its first column (a merged first column whose frame did
    /// not close; Task 12(b), REPORT §8 (б)). Off unless the scan gate passes.
    pub extend_first_column: bool,
    /// Raster rules (Task 12(c), REPORT §8 (в)) for segments with [`InputSegment::raster`]: classification
    /// by angle (within 1°) instead of |Δ| <= 2.5 pt, column bounds from stacks of short collinear
    /// vertical pieces (gaps counted), and in the planar partition only raster rulings of at least 0.3 of
    /// the region side with an 8-pt join gap. Non-raster input is untouched. Off: the Task 12(c) gate was not
    /// passed (`rounded_cells` c150 9/17 < 85 %, docs/tables-eval.md); the scan path keeps it off and feeds
    /// axis-aligned segments.
    pub short_vertical_stacks: bool,
    /// Most columns of a text-aligned (`T`) block; wider blocks are dropped (proto2.py `tx_maxcols`). The
    /// prototype's 15 cuts tables of 18-27 columns; T14 raises it to [`WIDE_TEXT_MAX_COLS`] (which still
    /// bounds the grid; default). The reference keeps 15.
    pub text_max_cols: usize,
    /// Rule groups and fill stacks retry their column valleys with a narrower minimum gap
    /// (max(2 pt, 0.35 font sizes)) and keep the result when it has more columns and the text lines confirm
    /// them (T14, proto2.py `gmin_adapt`: 3-pt gaps at 6.5-pt type). On by default.
    pub gmin_adapt: bool,
    /// A `T` candidate with more than 15 columns and no token of at least three letters is chart axis
    /// labels and is rejected as [`RejectReason::Chart`] (T14, proto2 `run.py` `reject`). On by
    /// default; a fully numeric table wider than 15 columns is rejected with it.
    pub axis_guard: bool,
    /// Text-block joining (T16, proto2.py `tx_join`): a block takes up to 6 non-candidate lines in a row
    /// (hanging cell lines: narrow, right under the previous line, starting at a segment start of the
    /// block; the old one-line skip counts towards the 6) and candidate
    /// lines up to 5 font sizes apart when their segment starts align; it breaks on an already found
    /// region. On (T16 gate passed).
    pub text_join: bool,
    /// Row rule of text rows (T17, proto2.py `row_newcols`): 0 only the first-column and gap rules; 1 also
    /// the new-columns line only (what `header_split` uses inside the header, not a shipped mode); 2 also a
    /// line taking at least two columns empty in the current row (a sub-header under a merged cell) or a
    /// full line (at least 3 columns and 60% of them); 3 the same with the full line only after a gap of
    /// 1.25 line heights (T17', checked, not shipped). 2 by default (T15 + T17 joint gate passed; T17 alone
    /// missed multihead shape, 97 < 100 of 140, docs/tables-eval.md).
    pub row_newcols: u8,
    /// Rule groups and fill stacks take up to this many column-aligned header lines above them (T17,
    /// proto2.py `hdr_ext`; at most [`MAX_HEADER_LINES`]); a line after the first one is also taken when each
    /// of its segments lies inside one column. 1 is the A2 `rg_extend`; [`MAX_HEADER_LINES`] by default
    /// (T15 + T17 joint gate).
    pub header_lines: usize,
    /// The header lines taken above a rule group or fill stack are split into rows by the new-columns rule,
    /// and `header_rows` of the table is their row count (T17, proto2.py `hdr_split`). On by default (T15 +
    /// T17 joint gate).
    pub header_split: bool,
    /// Candidate text lines: the at-most-5-words limit of a segment counts only non-numeric tokens, so
    /// digit groups (`32 000,00`) do not break a block (T15, proto2.py `tx_longseg=3`; modes 1 and 2 were
    /// rejected by the study). On by default (T15 + T17 joint gate; alone T15 drops ICDAR-train).
    pub text_longseg: bool,
    /// Column valleys of text blocks as for rule groups (`col_valleys`: lines of at least three segments
    /// when `dense_valleys`, the narrower retry when `gmin_adapt`; T15, proto2.py `tx_dense`). On by
    /// default (T15 + T17 joint gate).
    pub text_dense: bool,
    /// Guard "prose across the layout gutter" (T15, proto2.py `gutter_prose`): a `T` candidate crossing a
    /// page-wide valley in the middle half of the page and of itself, without numeric columns, is
    /// rejected as two-column prose when the median line has at least 4 words on each side (mode 1), or
    /// also when fewer than 20% of its lines have words on both sides (mode 3: column baselines shifted).
    /// 0 off; 3 by default (T15 + T17 joint gate); proto2's mode 2 is not ported (rejected by the study)
    /// and acts as mode 1.
    pub gutter_prose: u8,
    /// The `gutter_prose` guard also for a lined candidate whose rulings split no rows (its rows come from
    /// the text lines, as in a `T` block): a page frame with a rule between two columns of prose. On by
    /// default; off in the reference (the prototype applies the guard to `T` candidates only).
    /// Trade-off: it also rejects a text-heavy 2-column ruled table without row rules and without numeric
    /// columns whose column rule sits on a page-wide gutter, e.g. a frame-only term | definition glossary
    /// (mode 3 fires when long multi-line definitions leave words on both sides in < 20% of the lines).
    pub frame_prose: bool,
    /// Table regions of a layout model ([`PageInput::model_regions`]) as candidates (SP4-R1, opt-in): a
    /// region is snapped to the text lines inside it; an accepted lined table, rule group or fill stack
    /// overlapping it (IoU >= 0.5) wins and the region is dropped; an accepted text-block table with IoU >= 0.3
    /// to it, or inside it (5 pt margin), is replaced by the region's grid; the region is accepted with at
    /// least 3 rows and 2 columns, without the logistic model (rules chart, tiny and `Rotated` stay). `None`
    /// (default): the regions are ignored. No effect on the PDF path until SP5-R2 fills
    /// [`PageInput::model_regions`] (`OwnTables` leaves them empty).
    pub region_model: Option<RegionModelConfig>,
    /// Baseline: the text exclusion zone holds every ruling/box component of the page, so a component of
    /// invisible rectangles (stroke and fill off) or a lone box without a lined candidate silences the text
    /// detector inside it (idea-gap `usedL`, `heur_run.py`). With the flag, the components whose box touches
    /// no lined candidate are "freed"; when a page has any, the rule-group pass (if `rule_groups` is on) and
    /// the text pass run a second time with a zone of only the lined, fill-stack and rule candidates. A
    /// second-pass candidate is added only if it lies within one freed component (3 pt tolerance) and
    /// overlaps no existing candidate, so a `T` straddling several freed boxes is not built. First-pass
    /// results are unchanged. On by default (gate passed, docs/tables-eval.md).
    pub used_lined_only: bool,
    /// A lined candidate without a planar grid (booktabs: rules only above, under the header and below)
    /// splits a band between neighbouring horizontal rulings by its text when the band holds at least 2
    /// text lines, each filling at least half of the columns (at least 2): the rows come from
    /// `row_bounds_from_text` instead of one row per band (idea-gap `bandsplit`, `heur_run.py`). Off: gate not
    /// passed (docs/tables-eval.md).
    pub band_split: bool,
    /// SP4-R4 (A2): the height limit of a rule group (`RULE_GROUP_MAX_H_FRAC` of the page, a guard against
    /// page frames) does not apply to a regular group: at least 4 rules, every interval between neighbours
    /// holds a text line and none more than twice the median count. Off: gate not passed
    /// (docs/tables-eval.md).
    pub rg_tall_regular: bool,
    /// SP4-R4 (B): the rows of a regular rule group (see [`TableConfig::rg_tall_regular`]) are the rule
    /// intervals, whatever the share of intervals among the text lines (`RULE_ROW_SHARE`): wrapped cells
    /// stay in one row. On by default (gate passed, docs/tables-eval.md).
    pub rg_regular_rows: bool,
    /// SP4-R4 (C): a filled box with its bottom on the first rule of a rule group (within `FILL_EDGE_TOL`),
    /// its sides on the group's (within `RULE_ALIGN`) and lower than `FILL_MAX_H_FRAC` of the page is the
    /// header: the top of the region moves up to its top. On by default (gate passed, docs/tables-eval.md).
    pub rg_fill_header: bool,
    /// SP4-R4 (D): `rg_split` does not cut a rule group at a gap holding a line of two or more segments
    /// (a wrapped cell of a second column is not a caption). Rule groups only; fill stacks keep the rule.
    /// On by default (gate passed, docs/tables-eval.md).
    pub rg_split_multiseg: bool,
    /// SP4 T-cols: a `T` candidate with an inner column bound on a page gutter (within 3 pt) and, within
    /// 10 font sizes above it, at least 3 lines with words on each side of that gutter (lines crossing the
    /// gutter, such as one-column text, do not count) is rejected as [`RejectReason::TwoColumnProse`]: the
    /// aligned bottoms of two prose columns. Off: gate not passed (docs/tables-eval.md).
    pub page_columns: bool,
    /// The [`TableConfig::page_columns`] rule only for a candidate below the model probability 0.30 (the
    /// `TableUncertain` band): a sidebar beside the main column whose last lines align (a résumé). On by
    /// default (docs/tables-eval.md, "Uncertain page columns").
    pub uncertain_page_columns: bool,
    /// SP8b S3: a `T` candidate with an inner column gap of at least 12 font sizes holding a page gutter, and
    /// the two page columns continuing above it as in [`TableConfig::page_columns`], is rejected as
    /// [`RejectReason::TwoColumnProse`] at any probability: a résumé's main column beside its sidebar.
    pub wide_gap_columns: bool,
    /// SP8b S3: a lined candidate below the model probability 0.30 made of separate boxes, one per word
    /// group, in rows whose inner edges do not align (tag chips wrapped by width) is rejected as
    /// [`RejectReason::BoxFlow`].
    pub box_flow: bool,
    /// Background fills do not take part in the regions of the lined detector, so they no longer merge the
    /// rulings into one region without a candidate. A fill without stroke is a background when it holds at
    /// least 3 thin rulings and is at least twice as large as the box of those rulings, or when it belongs
    /// to a region that passes the same test (cell fills of a layout table chained to a ruled table) and is
    /// not fully inside the box of the region's rulings. The text detector and the other passes see the
    /// fills as before. On by default (gate passed, docs/tables-eval.md).
    pub lined_skip_background: bool,
    /// SP5 22.3, scans only: a text-block candidate is rejected as [`RejectReason::ScanDecoy`] when at
    /// least half of its rows (and 2) are table-of-contents lines (a page number last, after dot leaders
    /// that OCR kept only in pieces: `..`, `. .` or a `.12` page number) or when its last column holds only
    /// equation numbers (`(1)`, at least 2). A text layer keeps every leader dot, so the digital path does not
    /// need it. Off by default; `OwnTables::detect_scan` turns it on (docs/tables-eval.md, "SP5 Task 22").
    pub scan_text_guards: bool,
    /// SP5 22a (REPORT §8 (г)), scans: the word-grouping tolerances of the grids follow the median font
    /// size `fs` of the region's words (a scan's `fs` is estimated per word from its ink, ±20-30 %): text
    /// lines merge within 0.5 median `fs` instead of 0.5 of each word's own `fs`, text-block segments
    /// join below 1.0 median `fs` of the block's page words, and the pt values tuned on digital text
    /// become `pt / 10` median `fs` (valley floor 4 pt, retry floor 2 pt, valley-to-edge margins 5 pt
    /// and 3 pt). Off everywhere, `scan_config` included: the scan gate was not passed (m200 geometry
    /// below HEAD; docs/tables-eval.md, "Task 22a").
    pub fs_tolerances: bool,
}

/// Bound of [`TableConfig::header_lines`] (T17: `hdr_ext` ≤ 3 iterations).
pub const MAX_HEADER_LINES: usize = 3;

impl Default for TableConfig {
    fn default() -> Self {
        Self {
            filters: true,
            logit_threshold: select::LOGIT_THRESHOLD,
            hybrid: true,
            split_glued_words: false,
            rule_groups: true,
            merge_lines: true,
            fill_groups: true,
            rg_extend: true,
            dense_valleys: true,
            rg_split: true,
            fill_split: true,
            comp_lines: true,
            extend_first_column: false,
            short_vertical_stacks: false,
            text_max_cols: WIDE_TEXT_MAX_COLS,
            gmin_adapt: true,
            axis_guard: true,
            text_join: true,
            row_newcols: 2,
            header_lines: MAX_HEADER_LINES,
            header_split: true,
            text_longseg: true,
            text_dense: true,
            gutter_prose: 3,
            frame_prose: true,
            region_model: None,
            used_lined_only: true,
            band_split: false,
            rg_tall_regular: false,
            rg_regular_rows: true,
            rg_fill_header: true,
            rg_split_multiseg: true,
            page_columns: false,
            uncertain_page_columns: true,
            wide_gap_columns: true,
            box_flow: true,
            lined_skip_background: true,
            scan_text_guards: false,
            fs_tolerances: false,
        }
    }
}

impl TableConfig {
    /// Exactly the D11 pipeline as measured in Python (no glued-word splitting, no rule groups).
    pub fn reference() -> Self {
        Self {
            split_glued_words: false,
            rule_groups: false,
            merge_lines: false,
            fill_groups: false,
            rg_extend: false,
            dense_valleys: false,
            rg_split: false,
            fill_split: false,
            comp_lines: false,
            extend_first_column: false,
            short_vertical_stacks: false,
            text_max_cols: PROTO_TEXT_MAX_COLS,
            gmin_adapt: false,
            axis_guard: false,
            text_join: false,
            row_newcols: 0,
            header_lines: 1,
            header_split: false,
            text_longseg: false,
            text_dense: false,
            gutter_prose: 0,
            frame_prose: false,
            used_lined_only: false,
            rg_regular_rows: false,
            rg_fill_header: false,
            rg_split_multiseg: false,
            lined_skip_background: false,
            uncertain_page_columns: false,
            wide_gap_columns: false,
            box_flow: false,
            ..Self::default()
        }
    }
}

/// Keeps what lies within one page size around the page (content drawn far outside the MediaBox is
/// invisible but would still cost); `None` when nothing is outside or the page size is unusable.
/// [`PageInput::model_regions`] are copied only with `regions` and [`PageInput::image_boxes`] never (neither
/// is clipped: their callers read them from the unclipped page or only under the region-count limit).
fn clip_to_page(page: &PageInput, regions: bool) -> Option<PageInput> {
    let (w, h) = (page.width, page.height);
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return None;
    }
    let near = |x0: f64, top: f64, x1: f64, bot: f64| {
        [x0, top, x1, bot].iter().all(|v| v.is_finite())
            && x0.min(x1) <= 2.0 * w
            && x0.max(x1) >= -w
            && top.min(bot) <= 2.0 * h
            && top.max(bot) >= -h
    };
    let rect = |r: &Rect| near(r.x, r.top, r.x + r.w, r.top + r.h);
    let seg = |s: &InputSegment| near(s.x1, s.y1, s.x2, s.y2);
    if page.words.iter().all(|x| rect(&x.rect))
        && page.paths.iter().all(|x| rect(&x.rect))
        && page.segments.iter().all(seg)
    {
        return None;
    }
    Some(PageInput {
        width: w,
        height: h,
        words: page
            .words
            .iter()
            .filter(|x| rect(&x.rect))
            .cloned()
            .collect(),
        paths: page
            .paths
            .iter()
            .filter(|x| rect(&x.rect))
            .copied()
            .collect(),
        segments: page.segments.iter().filter(|x| seg(x)).copied().collect(),
        model_regions: if regions {
            page.model_regions.clone()
        } else {
            Vec::new()
        },
        image_boxes: Vec::new(),
    })
}

/// Detects tables on one page. A page over [`MAX_WORDS`], [`MAX_PRIMS`] or [`MAX_CELLS`] (paths and segments
/// also over [`ADAPTER_MAX_PRIMS`] before clipping) gets no tables and `skipped` set.
pub fn detect_tables(page: &PageInput, cfg: &TableConfig) -> PageTables {
    // Before clipping, which copies the words (the clipped page has no more of them) and the primitives
    // (bounded by the raw cap; the detector limit applies after clipping).
    let raw =
        LimitExceeded::check("words on the page", page.words.len(), MAX_WORDS).and_then(|()| {
            LimitExceeded::check(
                "paths and segments before clipping",
                page.paths.len() + page.segments.len(),
                ADAPTER_MAX_PRIMS,
            )
        });
    if let Err(e) = raw {
        return PageTables {
            skipped: Some(e),
            ..PageTables::default()
        };
    }
    // Clipping keeps the regions as they are; they are copied only when the detector may read them.
    let n_regions = page.model_regions.len();
    let use_regions = cfg.region_model.is_some();
    let clipped = clip_to_page(page, use_regions && n_regions <= MAX_MODEL_REGIONS);
    let page = clipped.as_ref().unwrap_or(page);
    let run = || {
        if use_regions {
            LimitExceeded::check("model regions on the page", n_regions, MAX_MODEL_REGIONS)?;
        }
        LimitExceeded::check(
            "paths and segments on the page",
            page.paths.len() + page.segments.len(),
            MAX_PRIMS,
        )?;
        // A stroked rectangle is a box plus four rulings: the region sweep sees up to 5x the input count.
        let mut pr = prims::prims_with(page, cfg.short_vertical_stacks);
        LimitExceeded::check(
            "primitives derived from the page",
            pr.hs.len() + pr.vs.len() + pr.rects.len(),
            MAX_PRIMS,
        )?;
        if cfg.merge_lines {
            prims::merge_collinear(&mut pr);
        }
        if cfg.lined_skip_background {
            prims::mark_backgrounds(&mut pr);
        }
        let words = if cfg.split_glued_words {
            split::split_glued(page, &pr)
        } else {
            prims::words_of(page)
        };
        let cands = detect::candidates(page, &words, &pr, cfg)?;
        Ok(select::select(cands, cfg))
    };
    run().unwrap_or_else(|e| PageTables {
        skipped: Some(e),
        ..PageTables::default()
    })
}

#[cfg(test)]
pub(crate) mod golden;
#[cfg(test)]
pub(crate) mod testkit;

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
