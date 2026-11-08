//! Candidate tables: lined regions (kind `L`) and text-aligned blocks (kind `T`) (proto.py
//! `detect_lined`, `detect_text`, `run`).

mod bounds;
mod fills;
mod flow;
mod gutter;
mod lined;
mod model;
mod rules;
mod text;

#[cfg(test)]
mod flow_test;
#[cfg(test)]
mod group_rows_test;
#[cfg(test)]
mod groups_test;
#[cfg(test)]
mod gutter_test;
#[cfg(test)]
mod helpers_test;
#[cfg(test)]
mod lined_test;
#[cfg(test)]
mod model_test;
#[cfg(test)]
mod text_test;

use crate::tables::block::UNCERTAIN_P;
use crate::tables::features::{Features, features};
use crate::tables::grid::text_lines;
use crate::tables::lattice::{Planar, lattice};
use crate::tables::prims::{PRect, Prims, Region, Word, regions, regions_with};
use crate::tables::select::{probability, use_planar};
use crate::tables::stats::median;
use crate::tables::{LimitExceeded, PageInput, TableConfig};
use bounds::region_of;
pub(crate) use bounds::within;
use fills::detect_fill_groups;
use flow::box_flow;
pub(crate) use gutter::page_gutters;
use gutter::{columns_above_at, columns_continue_above, prose_across_gutter, wide_gap_gutters};
use lined::{detect_lined, extend_first_column, split_bands};
use model::detect_model;
use rules::{FILL_MIN_H, FILL_MIN_W, detect_rules};
use text::detect_text;

/// Rulings group into one row/column bound when closer than this (proto.py `ycl_f`).
const RULE_MERGE: f64 = 2.0;
/// A row ruling must cover this share of the region width, a column ruling this share of its height.
const ROW_RULE_COVER: f64 = 0.35;
const COL_RULE_COVER: f64 = 0.3;
/// Valley width: at least this many points and this share of the font size.
const VALLEY_MIN: f64 = 4.0;
const VALLEY_FS: f64 = 0.7;
/// Share of lines allowed to cross a valley: lined regions / text blocks.
const VALLEY_TOL_LINED: f64 = 0.06;
const VALLEY_TOL_TEXT: f64 = 0.1;
/// Text blocks: a gap smaller than this many font sizes keeps words in one segment.
const SEG_GAP_FS: f64 = 1.0;
const SEG_MAX_WORDS: usize = 5;
const BLOCK_SKIP_GAP_FS: f64 = 2.2;
const BLOCK_MAX_GAP_FS: f64 = 3.0;
const TEXT_MIN_ROWS: usize = 3;
const TEXT_MAX_MEAN_WORDS: f64 = 6.0;
/// `gmin_adapt` (T14, proto2.py `adapt_valleys`, REPORT §4 (в)): retry gap max(this many points, this
/// share of the font size); kept when the median word-group count per line reaches this share of the
/// new column count.
const ADAPT_GMIN_MIN: f64 = 2.0;
const ADAPT_GMIN_FS: f64 = 0.35;
const ADAPT_CONFIRM: f64 = 0.8;
/// `dense_valleys` (A2, proto2.py `col_valleys`): valleys from lines of at least this many segments, when
/// at least this many lines and this share of all lines are such.
const DENSE_MIN_SEGS: usize = 3;
const DENSE_MIN_LINES: usize = 3;
const DENSE_LINE_SHARE: f64 = 0.5;
/// `rg_extend` (A2, proto2.py `extend_rows`): window in line steps, share of segments inside one column.
const EXTEND_PITCH: f64 = 0.9;
const EXTEND_ALIGNED_SHARE: f64 = 0.5;
/// `extend_first_column` (Task 12(b) brief): labels count within the larger of this many points and the
/// median column width left of the region; at least this share of the region's text lines must carry one;
/// at least this share of the page words in their x-band must lie inside the region's y-band (line numbers
/// in a margin run above and below it); the edge moves to the leftmost label minus this margin.
const FIRST_COL_MIN_REACH: f64 = 40.0;
const FIRST_COL_LINE_SHARE: f64 = 0.5;
const FIRST_COL_BAND_SHARE: f64 = 0.8;
const FIRST_COL_MARGIN: f64 = 2.0;
/// `band_split`: a text line of a band counts as a table row when it takes at least this share of the columns.
const BAND_FULL_SHARE: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateKind {
    /// Built from rulings/boxes of a region (`L`).
    Lined,
    /// Built from text alignment only (`T`).
    Text,
    /// A layout-model region snapped to the text lines inside it, columns from text valleys (`M`, SP4-R1).
    Model,
}

/// A raw candidate with its prototype grid, features and (for `L`) planar partition.
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub kind: CandidateKind,
    /// `[x0, top, x1, bottom]`.
    pub bbox: [f64; 4],
    /// The part of `bbox` its column/row bounds cover: words outside it are in no cell of `rows`.
    pub cells: [f64; 4],
    pub rows: Vec<Vec<String>>,
    pub features: Features,
    pub planar: Option<Planar>,
    /// Most of its words are not upright (rejected as [`super::RejectReason::Rotated`]).
    pub rotated: bool,
    /// Rows at or above the first rule or band of a rule group or fill stack (`header_split`); 0 with
    /// `header_split` off or nothing above it.
    pub head: usize,
    /// A `T` candidate (with `frame_prose` also a lined one whose rows come from the text) of prose across
    /// the layout gutter (`gutter_prose`, T15): rejected as [`super::RejectReason::TwoColumnProse`].
    pub gutter_prose: bool,
    /// A lined candidate of separate boxes in flow (`box_flow`): rejected as [`super::RejectReason::BoxFlow`].
    pub box_flow: bool,
    /// Built by the text-alignment pass (`detect_text`), not a rule group or fill stack: a model region
    /// replaces it (SP4-R1).
    pub text_block: bool,
}

/// Grid of one candidate before features.
pub(crate) struct Raw<'a> {
    pub kind: CandidateKind,
    pub bbox: [f64; 4],
    pub rows: Vec<Vec<String>>,
    pub words: Vec<&'a Word>,
    pub colb: Vec<f64>,
    pub rowb: Vec<f64>,
    /// Primitives of the region's own component (`comp_lines`), for the planar partition.
    pub sub: Option<Prims>,
    /// See [`Candidate::head`].
    pub head: usize,
    /// A lined candidate whose row bounds come from the text lines (its rulings split no rows):
    /// `frame_prose` gives it the `T` prose guard.
    pub text_rows: bool,
}

/// `[colb0, rowb0, colbN, rowbN]` within the bbox (`build_cells` drops words outside the bounds); the
/// bbox when a side has no bounds.
fn cell_extent(t: &Raw) -> [f64; 4] {
    let b = t.bbox;
    match (t.colb.first(), t.colb.last(), t.rowb.first(), t.rowb.last()) {
        (Some(&x0), Some(&x1), Some(&y0), Some(&y1)) if t.colb.len() > 1 && t.rowb.len() > 1 => {
            [b[0].max(x0), b[1].max(y0), b[2].min(x1), b[3].min(y1)]
        }
        _ => b,
    }
}

/// All candidates of a page in prototype order (lined first), with features and planar partition.
/// `rule_groups` adds hlines-only regions before the text pass (not in the prototype).
pub(crate) fn candidates(
    page: &PageInput,
    ws: &[Word],
    p: &Prims,
    cfg: &TableConfig,
) -> Result<Vec<Candidate>, LimitExceeded> {
    let mut regs = regions(p);
    if cfg.extend_first_column {
        extend_first_column(&mut regs, ws, p);
    }
    // Backgrounds leave only the regions of the lined detector; the other passes keep `regs`.
    let mut lined_regs = (cfg.lined_skip_background && p.rects.iter().any(|r| r.background))
        .then(|| regions_with(p, true));
    if cfg.extend_first_column
        && let Some(lr) = &mut lined_regs
    {
        extend_first_column(lr, ws, p);
    }
    let mut raws = detect_lined(ws, p, lined_regs.as_deref().unwrap_or(&regs), cfg)?;
    let hit = |a: [f64; 4], b: [f64; 4]| a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
    // Components that gave no lined candidate (before any filter, like `ts` of the prototype).
    let freed: Vec<[f64; 4]> = if cfg.used_lined_only {
        regs.iter()
            .map(|r| [r.x0, r.y0, r.x1, r.y1])
            .filter(|&q| !raws.iter().any(|t| hit(q, t.bbox)))
            .collect()
    } else {
        Vec::new()
    };
    if cfg.fill_groups {
        let fg = detect_fill_groups(ws, p, &regs, page.width, page.height, cfg)?;
        // Lined candidates and regions inside a stack (single bands with multi-line cells) give way to it.
        let in_stack = |b: [f64; 4]| fg.iter().any(|f| within(f.bbox, b, 3.0));
        raws.retain(|t| !in_stack(t.bbox));
        regs.retain(|r| !in_stack([r.x0, r.y0, r.x1, r.y1]));
        regs.extend(fg.iter().map(|t| region_of(t.bbox)));
        raws.extend(fg);
    }
    if cfg.rule_groups {
        let r = detect_rules(ws, p, &regs, page.height, cfg)?;
        regs.extend(r.iter().map(|t| region_of(t.bbox)));
        raws.extend(r);
    }
    let mut text_from = raws.len();
    raws.extend(detect_text(ws, &regs, cfg)?);
    if !freed.is_empty() {
        // `used_lined_only`: the rule and text passes again with only the candidates above as the zone;
        // what they find inside a freed component and away from every candidate is added.
        let mut zone: Vec<Region> = raws[..text_from]
            .iter()
            .map(|t| region_of(t.bbox))
            .collect();
        let rules = if cfg.rule_groups {
            detect_rules(ws, p, &zone, page.height, cfg)?
        } else {
            Vec::new()
        };
        zone.extend(rules.iter().map(|t| region_of(t.bbox)));
        let text = detect_text(ws, &zone, cfg)?;
        let new = |t: &Raw| {
            freed.iter().any(|&q| within(q, t.bbox, 3.0))
                && !raws.iter().any(|u| hit(u.bbox, t.bbox))
        };
        let (rules, text): (Vec<Raw>, Vec<Raw>) = (
            rules.into_iter().filter(|t| new(t)).collect(),
            text.into_iter().filter(|t| new(t)).collect(),
        );
        let n = rules.len();
        raws.splice(text_from..text_from, rules);
        text_from += n;
        raws.extend(text);
    }
    if let Some(m) = &cfg.region_model
        && !page.model_regions.is_empty()
    {
        let all: Vec<&Word> = ws.iter().collect();
        // SP4-R3: the filled boxes for zebra rows, only with `inner_bounds`.
        let fills: Option<Vec<&PRect>> = m.inner_bounds.then(|| {
            p.rects
                .iter()
                .filter(|r| r.fill && r.b - r.t >= FILL_MIN_H && r.x1 - r.x0 >= FILL_MIN_W)
                .collect()
        });
        raws.extend(detect_model(
            &text_lines(&all),
            &page.model_regions,
            fills.as_deref(),
            cfg,
        )?);
    }
    let page_fs = (!ws.is_empty()).then(|| median(&ws.iter().map(|w| w.fs).collect::<Vec<_>>()));
    // The `T` prose guard; with `frame_prose` also for lined candidates whose rows come from the text.
    let prose_guarded = |t: &Raw| t.kind == CandidateKind::Text || (cfg.frame_prose && t.text_rows);
    let gut = match page_fs {
        Some(fs)
            if (cfg.gutter_prose > 0
                || cfg.page_columns
                || cfg.uncertain_page_columns
                || cfg.wide_gap_columns)
                && raws.iter().any(prose_guarded) =>
        {
            page_gutters(ws, page.width, fs)
        }
        _ => Vec::new(),
    };
    raws.into_iter()
        .enumerate()
        .map(|(i, t)| {
            let features = features(&t, page, ws, page_fs, p);
            let planar = if t.kind == CandidateKind::Lined {
                lattice(ws, t.sub.as_ref().unwrap_or(p), t.bbox)?
            } else {
                None
            };
            let rotated = 2 * t.words.iter().filter(|w| w.rotated).count() > t.words.len();
            let fs = page_fs.unwrap_or(0.0);
            let uncertain = probability(&features, t.kind) < UNCERTAIN_P;
            let gutter_prose = prose_guarded(&t)
                && features.numcols == 0.0
                && (prose_across_gutter(&t.words, t.bbox, &gut, cfg.gutter_prose)
                    || (t.kind == CandidateKind::Text
                        && (cfg.page_columns || (cfg.uncertain_page_columns && uncertain))
                        && columns_continue_above(ws, t.bbox, fs, &t.colb, &gut))
                    || (t.kind == CandidateKind::Text
                        && cfg.wide_gap_columns
                        && columns_above_at(
                            ws,
                            t.bbox,
                            fs,
                            &wide_gap_gutters(&t.words, &t.colb, &gut, fs),
                        )));
            let box_flow = cfg.box_flow
                && t.kind == CandidateKind::Lined
                && uncertain
                && box_flow(&t.words, &p.rects, t.bbox);
            let cells = cell_extent(&t);
            let mut cand = Candidate {
                kind: t.kind,
                bbox: t.bbox,
                cells,
                rows: t.rows,
                features,
                planar,
                rotated,
                head: t.head,
                gutter_prose,
                box_flow,
                // Raws from `text_from` on are text blocks, then model regions.
                text_block: i >= text_from && t.kind == CandidateKind::Text,
            };
            if cfg.band_split
                && cand.kind == CandidateKind::Lined
                && !use_planar(&cand)
                && let Some(rows) = split_bands(&t.words, &t.colb, &t.rowb, cfg)?
            {
                cand.rows = rows;
                // Selection must not take the planar grid for the longer row list.
                cand.planar = None;
            }
            Ok(cand)
        })
        .collect()
}
