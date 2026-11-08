//! Rule groups (booktabs, hlines-only tables): grouping, `rg_split`, `header_split`, `rg_extend`.

use super::bounds::{
    extend_rows, inside, line_segs, region_lines, region_of, text_pitch, too_wordy, valley_bounds,
};
use super::{CandidateKind, RULE_MERGE, Raw};
use crate::tables::grid::{Line, build_cells, finalize, row_bounds_from_text, text_lines};
use crate::tables::prims::{HLine, Prims, Region, Word};
use crate::tables::stats::median;
use crate::tables::{LimitExceeded, TableConfig};

/// Horizontal rulings this long and aligned within this tolerance form one rule group.
const RULE_MIN_LEN: f64 = 50.0;
const RULE_ALIGN: f64 = 6.0;
const RULE_GROUP_MIN_H: f64 = 15.0;
/// Rules are row bounds when they cut the region into at least this share of its text lines (a rule under
/// every row, `hlines_all`); otherwise (booktabs top/header/bottom) rows come from the text.
const RULE_ROW_SHARE: f64 = 0.8;
/// A rule group taller than this share of the page is a page frame (journal header + footer rules).
pub(super) const RULE_GROUP_MAX_H_FRAC: f64 = 0.6;
/// `rg_split` (A2, proto2.py): a group is cut at a rule gap over this many text line steps holding a caption
/// line (one segment wider than this share of the group).
const SPLIT_GAP_PITCH: f64 = 2.5;
const SPLIT_CAPTION_W: f64 = 0.4;
/// Line step of a group without two text lines (proto2.py `rg_split`).
const SPLIT_PITCH_FALLBACK: f64 = 12.0;
/// Over this many rulings (after merging) a page gets no rule groups, over this many filled boxes no fill
/// stacks: illustration pages, the grouping is O(n·groups) (A2 limits, REPORT §2/§7, ODL `LINES > 5000`).
/// The page is not skipped: regions and text blocks still run.
const MAX_GROUP_RULINGS: usize = 5000;
pub(super) const MAX_STACK_FILLS: usize = 2000;
/// Fill stacks (A2, proto2.py `detect_fill_groups`): a filled box at least this high and wide, lower than
/// this share of the page; boxes with top and bottom within [`FILL_EDGE_TOL`] form a band.
pub(super) const FILL_MIN_H: f64 = 4.0;
pub(super) const FILL_MIN_W: f64 = 3.0;
pub(super) const FILL_MAX_H_FRAC: f64 = 0.25;
pub(super) const FILL_EDGE_TOL: f64 = 1.5;
/// A band is at least this wide and its fills cover this share of it (pieces closer than
/// [`FILL_EDGE_TOL`] join).
pub(super) const BAND_MIN_W: f64 = 60.0;
pub(super) const BAND_COVER: f64 = 0.8;
/// Bands with x edges within this tolerance and a gap of at most this many band heights stack.
pub(super) const STACK_ALIGN: f64 = 6.0;
pub(super) const STACK_GAP_H: f64 = 3.0;
/// Fill x edges within this distance of a cluster's first edge are one column bound; a bound needs this
/// share of the bands (at least two edges).
pub(super) const FILL_COL_TOL: f64 = 2.0;
pub(super) const FILL_COL_SHARE: f64 = 0.5;
/// Row bounds of a stack closer than this to the previous one are dropped.
pub(super) const FILL_ROW_TOL: f64 = 2.0;

type Group<'p> = Vec<&'p HLine>;

/// `rg_split` (rule groups) and `fill_split` (fill stacks): cuts a sequence of at least three items (`x`:
/// their `(x0, x1)`, `span`: their `(top, bottom)`) at a gap over [`SPLIT_GAP_PITCH`] text line steps that
/// holds a caption line (two tables of one width with a caption between them).
pub(super) fn split_at_captions<T>(
    ws: &[Word],
    groups: Vec<Vec<T>>,
    x: impl Fn(&T) -> (f64, f64),
    span: impl Fn(&T) -> (f64, f64),
    multiseg_guard: bool,
) -> Vec<Vec<T>> {
    let mut out = Vec::new();
    for g in groups {
        let x0 = g.iter().map(|t| x(t).0).fold(f64::MAX, f64::min);
        let x1 = g.iter().map(|t| x(t).1).fold(f64::MIN, f64::max);
        let spans: Vec<(f64, f64)> = g.iter().map(&span).collect();
        out.extend(cut_at(g, &caption_cuts(ws, x0, x1, &spans, multiseg_guard)));
    }
    out
}

/// Splits `items` before every index where `cuts` is set.
fn cut_at<T>(items: Vec<T>, cuts: &[bool]) -> Vec<Vec<T>> {
    let mut out: Vec<Vec<T>> = Vec::new();
    for (i, x) in items.into_iter().enumerate() {
        match out.last_mut() {
            Some(cur) if !cuts[i] => cur.push(x),
            _ => out.push(vec![x]),
        }
    }
    out
}

/// The `rg_split` rule over a vertical sequence of `spans` `(top, bottom)` (rules: `(y, y)`; fill bands)
/// across `x0..x1`: `cuts[i]` when the gap before span `i` is over [`SPLIT_GAP_PITCH`] text line steps and
/// holds a caption line (one segment wider than [`SPLIT_CAPTION_W`] of the width). Sequences shorter than
/// three are not cut. `multiseg_guard` (`rg_split_multiseg`): no cut when the gap holds a line of two or
/// more segments.
fn caption_cuts(
    ws: &[Word],
    x0: f64,
    x1: f64,
    spans: &[(f64, f64)],
    multiseg_guard: bool,
) -> Vec<bool> {
    let mut cuts = vec![false; spans.len()];
    if spans.len() < 3 {
        return cuts;
    }
    let (gy0, gy1) = (spans[0].0, spans[spans.len() - 1].1);
    let rw: Vec<&Word> = ws
        .iter()
        .filter(|w| x0 <= w.cx && w.cx <= x1 && gy0 <= w.cy && w.cy <= gy1)
        .collect();
    let lines = text_lines(&rw);
    let pitch = if lines.len() > 1 {
        text_pitch(&lines, SPLIT_PITCH_FALLBACK)
    } else {
        SPLIT_PITCH_FALLBACK
    };
    for i in 1..spans.len() {
        let (a, b) = (spans[i - 1].1, spans[i].0);
        // Line centres increase (a new line starts more than half a font size below the last mean).
        let lo = lines.partition_point(|l| l.cy <= a);
        let hi = lines.partition_point(|l| l.cy < b).max(lo);
        cuts[i] = b - a > SPLIT_GAP_PITCH * pitch
            && (!multiseg_guard || lines[lo..hi].iter().all(|l| line_segs(l).len() == 1))
            && lines[lo..hi].iter().any(|l| {
                line_segs(l).len() == 1
                    && l.w[l.w.len() - 1].x1 - l.w[0].x0 > SPLIT_CAPTION_W * (x1 - x0)
            });
    }
    cuts
}

/// `header_split` (T17, proto2.py `split_header`): the header lines above `top` (the first rule or band)
/// split into rows by [`row_bounds_from_text`] with at least the new-columns rule (mode 1); one pass.
fn split_header(rw: &[&Word], colb: &[f64], rowb: &mut Vec<f64>, top: f64, newcols: u8) {
    let hw: Vec<&Word> = rw.iter().copied().filter(|w| w.cy < top - 1.0).collect();
    if hw.is_empty() {
        return;
    }
    let hl = text_lines(&hw);
    if hl.len() < 2 {
        return;
    }
    let lo = rowb[0];
    let hb = row_bounds_from_text(&hl, colb, lo, top, newcols.max(1));
    rowb.extend(
        hb[1..hb.len() - 1]
            .iter()
            .copied()
            .filter(|&y| lo + 1.0 < y && y < top - 1.0),
    );
    rowb.sort_by(f64::total_cmp);
    rowb.dedup();
}

type Rows = Vec<Vec<String>>;

/// [`split_header`] above `top` when `header_split` is on and the candidate reaches above `top`
/// (`room`); returns the `top` that [`cells_and_head`] counts header rows against.
pub(super) fn header_top(
    cfg: &TableConfig,
    room: bool,
    rw: &[&Word],
    colb: &[f64],
    rowb: &mut Vec<f64>,
    top: f64,
) -> Option<f64> {
    (cfg.header_split && room).then(|| {
        split_header(rw, colb, rowb, top, cfg.row_newcols);
        top
    })
}

/// Grid of a candidate and its header row count: the non-empty rows ending at or above `top` (from
/// [`header_top`]; `None`, with `header_split` off or nothing above the first rule or band: 0).
pub(super) fn cells_and_head(
    rw: &[&Word],
    colb: &[f64],
    rowb: &[f64],
    top: Option<f64>,
) -> Result<Option<(Rows, usize)>, LimitExceeded> {
    let g = build_cells(rw, colb, rowb)?;
    let head = top.map_or(0, |top| {
        g.iter()
            .zip(&rowb[1..])
            .filter(|&(r, &end)| end <= top && r.iter().any(|c| !c.trim().is_empty()))
            .count()
    });
    Ok(finalize(g).map(|rows| (rows, head)))
}

/// One rule-group candidate over `e` (taller than the rules of `g` after `rg_extend`) and its lines.
fn build_rule_group<'a>(
    ws: &'a [Word],
    g: &[&HLine],
    e: &Region,
    colb: Option<Vec<f64>>,
    cfg: &TableConfig,
) -> Result<Option<(Raw<'a>, Vec<Line<'a>>)>, LimitExceeded> {
    let rw: Vec<&Word> = ws.iter().filter(|w| inside(w, e, 0.0)).collect();
    if rw.len() < 4 {
        return Ok(None);
    }
    let lines = region_lines(&rw, cfg);
    let colb = colb.unwrap_or_else(|| {
        let (d, a, f) = (cfg.dense_valleys, cfg.gmin_adapt, cfg.fs_tolerances);
        valley_bounds(&rw, &lines, e, d, a, f)
    });
    if colb.len() < 3 {
        return Ok(None);
    }
    let mut top = None;
    let rowb = if (cfg.rg_regular_rows && regular_rows(&lines, g))
        || (g.len() - 1) as f64 >= RULE_ROW_SHARE * lines.len() as f64
    {
        let mut rb = Vec::with_capacity(g.len() + 2);
        if e.y0 < g[0].y - 1.0 {
            rb.push(e.y0);
        }
        rb.extend(g.iter().map(|h| h.y));
        top = header_top(cfg, e.y0 < g[0].y - 1.0, &rw, &colb, &mut rb, g[0].y);
        if e.y1 > g[g.len() - 1].y + 1.0 {
            rb.push(e.y1 + 1.0);
        }
        rb
    } else {
        row_bounds_from_text(&lines, &colb, e.y0, e.y1, cfg.row_newcols)
    };
    let Some((rows, head)) = cells_and_head(&rw, &colb, &rowb, top)? else {
        return Ok(None);
    };
    if too_wordy(&rows) {
        return Ok(None);
    }
    Ok(Some((
        Raw {
            kind: CandidateKind::Text,
            bbox: [e.x0, e.y0, e.x1, e.y1],
            rows,
            words: rw,
            colb,
            rowb,
            sub: None,
            head,
            text_rows: false,
        },
        lines,
    )))
}

/// `rg_extend` over a built candidate: rebuilds it over the extended region with the same columns and keeps
/// the rebuild when it has more rows and as many columns.
pub(super) fn extend_candidate<'a>(
    ws: &'a [Word],
    r: &Region,
    pitch: f64,
    above_lines: usize,
    c0: Raw<'a>,
    build: impl FnOnce(&Region, Vec<f64>) -> Result<Option<Raw<'a>>, LimitExceeded>,
) -> Result<Raw<'a>, LimitExceeded> {
    let (y0, y1) = extend_rows(ws, r, pitch, &c0.colb, above_lines);
    if (y0, y1) == (r.y0, r.y1) {
        return Ok(c0);
    }
    let e = region_of([r.x0, y0, r.x1, y1]);
    Ok(match build(&e, c0.colb.clone())? {
        Some(c1) if c1.rows.len() > c0.rows.len() && c1.rows[0].len() == c0.rows[0].len() => c1,
        _ => c0,
    })
}

/// Regions bounded by two or more aligned full rulings without verticals (booktabs, hlines-only tables):
/// the connected-primitive regions miss them because the rulings do not touch.
pub(crate) fn detect_rules<'a>(
    ws: &'a [Word],
    p: &Prims,
    regs: &[Region],
    page_h: f64,
    cfg: &TableConfig,
) -> Result<Vec<Raw<'a>>, LimitExceeded> {
    if p.hs.len() + p.vs.len() > MAX_GROUP_RULINGS {
        return Ok(Vec::new());
    }
    let mut hs: Vec<_> = p.hs.iter().filter(|h| h.b - h.a >= RULE_MIN_LEN).collect();
    hs.sort_by(|x, y| x.y.total_cmp(&y.y));
    let mut groups: Vec<Group> = Vec::new();
    for h in hs {
        match groups
            .iter_mut()
            .find(|g| (g[0].a - h.a).abs() <= RULE_ALIGN && (g[0].b - h.b).abs() <= RULE_ALIGN)
        {
            Some(g) => {
                if (g[g.len() - 1].y - h.y).abs() > RULE_MERGE {
                    g.push(h)
                }
            }
            None => groups.push(vec![h]),
        }
    }
    if cfg.rg_split {
        groups = split_at_captions(
            ws,
            groups,
            |h| (h.a, h.b),
            |h| (h.y, h.y),
            cfg.rg_split_multiseg,
        );
    }
    let claimed = |y0: f64, r: &Region| {
        regs.iter()
            .any(|q| q.x0 < r.x1 && r.x0 < q.x1 && q.y0 < r.y1 && y0 < q.y1)
    };
    let mut out = Vec::new();
    for g in groups.into_iter().filter(|g| g.len() >= 2) {
        let r = region_of([
            g.iter().map(|h| h.a).fold(f64::MAX, f64::min),
            g[0].y,
            g.iter().map(|h| h.b).fold(f64::MIN, f64::max),
            g[g.len() - 1].y,
        ]);
        if r.y1 - r.y0 < RULE_GROUP_MIN_H
            || (r.y1 - r.y0 > RULE_GROUP_MAX_H_FRAC * page_h
                && !(cfg.rg_tall_regular && regular_rule_group(ws, &g)))
            || claimed(r.y0, &r)
        {
            continue;
        }
        let mut r = r;
        if cfg.rg_fill_header
            && let Some(f) = p.rects.iter().find(|f| {
                f.fill
                    && (f.b - g[0].y).abs() <= FILL_EDGE_TOL
                    && (f.x0 - r.x0).abs() <= RULE_ALIGN
                    && (f.x1 - r.x1).abs() <= RULE_ALIGN
                    && f.b - f.t < FILL_MAX_H_FRAC * page_h
                    && !claimed(f.t, &r)
            })
        {
            r.y0 = f.t;
        }
        let Some((mut c0, lines)) = build_rule_group(ws, &g, &r, None, cfg)? else {
            continue;
        };
        if cfg.rg_extend {
            let gaps: Vec<f64> = g.windows(2).map(|p| p[1].y - p[0].y).collect();
            let pitch = text_pitch(&lines, median(&gaps));
            c0 = extend_candidate(ws, &r, pitch, cfg.header_lines, c0, |e, colb| {
                Ok(build_rule_group(ws, &g, e, Some(colb), cfg)?.map(|c| c.0))
            })?;
        }
        out.push(c0);
    }
    Ok(out)
}

/// `rg_tall_regular` and `rg_regular_rows`: a regular rule group has at least 4 rules, a text line in every
/// interval between neighbouring rules and no interval with more than twice the median count of lines.
fn regular_rows(lines: &[Line], g: &[&HLine]) -> bool {
    if g.len() < 4 {
        return false;
    }
    // `text_lines` returns the lines sorted by `cy`.
    let n: Vec<f64> = g
        .windows(2)
        .map(|p| {
            let lo = lines.partition_point(|l| l.cy <= p[0].y);
            lines[lo..].partition_point(|l| l.cy < p[1].y) as f64
        })
        .collect();
    n.iter().all(|&k| k >= 1.0) && n.iter().cloned().fold(0.0, f64::max) <= 2.0 * median(&n)
}

/// [`regular_rows`] over the words between the rules of `g`.
fn regular_rule_group(ws: &[Word], g: &[&HLine]) -> bool {
    let rw: Vec<&Word> = ws
        .iter()
        .filter(|w| g[0].a <= w.cx && w.cx <= g[0].b && g[0].y < w.cy && w.cy < g[g.len() - 1].y)
        .collect();
    regular_rows(&text_lines(&rw), g)
}
