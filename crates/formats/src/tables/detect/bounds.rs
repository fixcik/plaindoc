//! Shared bounds of the detectors: rulings and text valleys to column/row bounds, text segments and
//! line steps, `rg_extend`, region geometry.

use super::{
    ADAPT_CONFIRM, ADAPT_GMIN_FS, ADAPT_GMIN_MIN, COL_RULE_COVER, DENSE_LINE_SHARE,
    DENSE_MIN_LINES, DENSE_MIN_SEGS, EXTEND_ALIGNED_SHARE, EXTEND_PITCH, RULE_MERGE, SEG_GAP_FS,
    TEXT_MAX_MEAN_WORDS, VALLEY_FS, VALLEY_MIN, VALLEY_TOL_LINED,
};
use crate::tables::grid::{Line, text_lines, text_lines_at, valleys};
use crate::tables::prims::{Region, VLine, Word};
use crate::tables::stats::{mean, median};
use crate::tables::{MAX_HEADER_LINES, TableConfig};

pub(super) fn rule_bounds(mut v: Vec<(f64, f64)>, min_len: f64) -> Vec<f64> {
    v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut cl: Vec<(f64, f64)> = Vec::new();
    for (p, l) in v {
        match cl.last_mut() {
            Some(c) if p - c.0 <= RULE_MERGE => {
                c.1 += l;
                c.0 = (c.0 + p) / 2.0;
            }
            _ => cl.push((p, l)),
        }
    }
    cl.into_iter()
        .filter(|c| c.1 >= min_len)
        .map(|c| c.0)
        .collect()
}

/// Column bounds of a region from text valleys (lined fallback, rule groups, fill stacks): region edges
/// plus the valleys more than 5 pt inside them. `dense` (A2, proto2.py `col_valleys`) takes the valleys of
/// the lines with at least [`DENSE_MIN_SEGS`] segments only, when there are enough of them.
pub(super) fn valley_bounds(
    rw: &[&Word],
    lines: &[Line],
    r: &Region,
    dense: bool,
    adapt: bool,
    fst: bool,
) -> Vec<f64> {
    let fs = median(&rw.iter().map(|w| w.fs).collect::<Vec<_>>());
    let v = col_valleys(rw, lines, fs, VALLEY_TOL_LINED, dense, adapt, fst);
    let m = fs_pt(5.0, fst, fs);
    std::iter::once(r.x0)
        .chain(v.into_iter().filter(|&c| r.x0 + m < c && c < r.x1 - m))
        .chain(std::iter::once(r.x1))
        .collect()
}

/// `fs_tolerances` (`fst`): a tolerance of `pt` points tuned on digital text (median font size 10 pt)
/// becomes `pt / 10` of the region's median font size `fs`; without the flag `pt` itself.
pub(super) fn fs_pt(pt: f64, fst: bool, fs: f64) -> f64 {
    if fst { pt / 10.0 * fs } else { pt }
}

/// `fs_tolerances`: the median font size of `ws` (at least 1 pt) for the word-grouping tolerances of a
/// region; `None` without the flag or without words.
pub(super) fn region_fs(ws: &[&Word], cfg: &TableConfig) -> Option<f64> {
    (cfg.fs_tolerances && !ws.is_empty())
        .then(|| median(&ws.iter().map(|w| w.fs).collect::<Vec<_>>()).max(1.0))
}

/// [`text_lines`] of a region's words, merged on the region's median font size with `fs_tolerances`.
pub(super) fn region_lines<'a>(ws: &[&'a Word], cfg: &TableConfig) -> Vec<Line<'a>> {
    text_lines_at(ws, region_fs(ws, cfg))
}

/// proto2.py `col_valleys(rw, lines, fs, tol_frac)`: the valleys of `rw` (of the dense lines with `dense`),
/// tolerating `tol_frac` of the lines, retried narrower with `adapt`; `fst` is `fs_tolerances`.
pub(super) fn col_valleys(
    rw: &[&Word],
    lines: &[Line],
    fs: f64,
    tol_frac: f64,
    dense: bool,
    adapt: bool,
    fst: bool,
) -> Vec<f64> {
    let gmin = fs_pt(VALLEY_MIN, fst, fs).max(VALLEY_FS * fs);
    let dl: Vec<&Line> = if dense {
        lines
            .iter()
            .filter(|l| line_segs_at(l, fst.then_some(fs)).len() >= DENSE_MIN_SEGS)
            .collect()
    } else {
        Vec::new()
    };
    let dw: Option<Vec<&Word>> = (dense
        && dl.len() >= DENSE_MIN_LINES
        && dl.len() as f64 >= DENSE_LINE_SHARE * lines.len() as f64)
        .then(|| dl.iter().flat_map(|l| l.w.iter().copied()).collect());
    let (vw, n) = match &dw {
        Some(d) => (d.as_slice(), dl.len()),
        None => (rw, lines.len()),
    };
    let tol = (tol_frac * n as f64) as usize;
    let v = valleys(vw, gmin, tol);
    if adapt {
        adapt_valleys(v, vw, lines, fs, tol, fst)
    } else {
        v
    }
}

/// `gmin_adapt` (T14, proto2.py `adapt_valleys`): valleys again with the narrower minimum gap; taken when
/// there are more of them and the median count of word groups per line (gaps of at least that width) is
/// at least [`ADAPT_CONFIRM`] of the new column count. `lines` are all lines of the region, `ws`/`tol`
/// those of the first pass. At most one more `valleys` call.
pub(super) fn adapt_valleys(
    v: Vec<f64>,
    ws: &[&Word],
    lines: &[Line],
    fs: f64,
    tol: usize,
    fst: bool,
) -> Vec<f64> {
    let g2 = fs_pt(ADAPT_GMIN_MIN, fst, fs).max(ADAPT_GMIN_FS * fs);
    let v2 = valleys(ws, g2, tol);
    if v2.len() <= v.len() {
        return v;
    }
    let cnt: Vec<f64> = lines
        .iter()
        .map(|l| {
            let (mut n, mut last) = (0usize, None::<f64>);
            for w in &l.w {
                if last.is_none_or(|x| w.x0 - x >= g2) {
                    n += 1;
                }
                last = Some(last.map_or(w.x1, |x| x.max(w.x1)));
            }
            n as f64
        })
        .collect();
    if !cnt.is_empty() && median(&cnt) >= ADAPT_CONFIRM * (v2.len() + 1) as f64 {
        v2
    } else {
        v
    }
}

/// Segments `(x0, x1)` of a text line: words closer than [`SEG_GAP_FS`] median font sizes join
/// (proto2.py `line_segs`).
pub(super) fn line_segs(l: &Line) -> Vec<(f64, f64)> {
    line_segs_at(l, None)
}

/// [`line_segs`] on font size `fs` (the region median, `fs_tolerances`) instead of the line's median.
fn line_segs_at(l: &Line, fs: Option<f64>) -> Vec<(f64, f64)> {
    let fs = fs.unwrap_or_else(|| median(&l.w.iter().map(|w| w.fs).collect::<Vec<_>>()));
    let mut segs: Vec<(f64, f64)> = Vec::new();
    for w in &l.w {
        match segs.last_mut() {
            Some(s) if w.x0 - s.1 < SEG_GAP_FS * fs => s.1 = w.x1,
            _ => segs.push((w.x0, w.x1)),
        }
    }
    segs
}

/// Median step between text lines more than 2 pt apart, else `fallback` (proto2.py `text_pitch`).
pub(super) fn text_pitch(lines: &[Line], fallback: f64) -> f64 {
    let g: Vec<f64> = lines
        .windows(2)
        .map(|p| p[1].cy - p[0].cy)
        .filter(|&d| d > 2.0)
        .collect();
    if g.is_empty() { fallback } else { median(&g) }
}

/// `rg_extend` (proto2.py `extend_rows`): takes one text line right above/below the region (centre within
/// [`EXTEND_PITCH`] line steps) when its segments sit in at least two columns and at least half of them
/// inside one column; a caption (one segment across columns) is not taken. Above, up to `above_lines`
/// (T17 `header_lines`, at most [`MAX_HEADER_LINES`]) lines in turn, each within the step of the previous
/// one; after the first, a line whose every segment lies inside one column is taken too (`Па` under
/// `Давление,`). Returns the new `(y0, y1)`.
pub(super) fn extend_rows(
    ws: &[Word],
    r: &Region,
    pitch: f64,
    colb: &[f64],
    above_lines: usize,
) -> (f64, f64) {
    let col = |v: f64| colb.partition_point(|&c| c <= v) as isize - 1;
    let aligned = |segs: &[(f64, f64)]| {
        if segs.len() < 2 {
            return false;
        }
        let (mut cols, mut inside) = (Vec::new(), 0usize);
        for &(a, b) in segs {
            let (ia, ib) = (col(a), col(b - 1e-6));
            if !cols.contains(&ia) {
                cols.push(ia);
            }
            if ia == ib {
                inside += 1;
            }
        }
        cols.len() >= 2 && inside as f64 >= EXTEND_ALIGNED_SHARE * segs.len() as f64
    };
    let in_columns = |segs: &[(f64, f64)]| {
        !segs.is_empty() && segs.iter().all(|&(a, b)| col(a) == col(b - 1e-6))
    };
    let (mut y0, mut y1) = (r.y0, r.y1);
    for above in [true, false] {
        let iters = if above {
            above_lines.min(MAX_HEADER_LINES)
        } else {
            1
        };
        for k in 0..iters {
            let cand: Vec<&Word> = ws
                .iter()
                .filter(|w| {
                    r.x0 - 2.0 <= w.cx
                        && w.cx <= r.x1 + 2.0
                        && if above {
                            y0 - EXTEND_PITCH * pitch <= w.cy && w.cy < y0 - 1.0
                        } else {
                            y1 + 1.0 < w.cy && w.cy <= y1 + EXTEND_PITCH * pitch
                        }
                })
                .collect();
            if cand.is_empty() {
                break;
            }
            let ls = text_lines(&cand);
            let l = if above { &ls[ls.len() - 1] } else { &ls[0] };
            let segs = line_segs(l);
            if !(aligned(&segs) || (k > 0 && in_columns(&segs))) {
                break;
            }
            if above {
                y0 = l.w.iter().map(|w| w.top).fold(f64::MAX, f64::min) - 1.0;
            } else {
                y1 = l.w.iter().map(|w| w.bot).fold(f64::MIN, f64::max) + 1.0;
            }
        }
    }
    (y0, y1)
}

/// Mean words per filled cell above [`TEXT_MAX_MEAN_WORDS`]: prose, not a table.
pub(super) fn too_wordy(rows: &[Vec<String>]) -> bool {
    let wc: Vec<f64> = rows
        .iter()
        .flatten()
        .filter(|c| !c.trim().is_empty())
        .map(|c| c.split_whitespace().count() as f64)
        .collect();
    mean(&wc) > TEXT_MAX_MEAN_WORDS
}

pub(super) fn region_of(b: [f64; 4]) -> Region {
    Region {
        x0: b[0],
        y0: b[1],
        x1: b[2],
        y1: b[3],
        members: Vec::new(),
    }
}

pub(super) fn inside(w: &Word, r: &Region, m: f64) -> bool {
    r.x0 - m <= w.cx && w.cx <= r.x1 + m && r.y0 - m <= w.cy && w.cy <= r.y1 + m
}

/// Box `inner` lies within `outer` widened by `m` (`[x0, top, x1, bottom]`).
pub(crate) fn within(outer: [f64; 4], inner: [f64; 4], m: f64) -> bool {
    outer[0] - m <= inner[0]
        && inner[2] <= outer[2] + m
        && outer[1] - m <= inner[1]
        && inner[3] <= outer[3] + m
}

/// A vertical ruling within 3 pt of a region.
pub(super) fn v_near(l: &VLine, r: &Region) -> bool {
    r.x0 - 3.0 <= l.x && l.x <= r.x1 + 3.0 && l.a >= r.y0 - 3.0 && l.b <= r.y1 + 3.0
}

/// Column bounds of a region from the vertical rulings within 3 pt of it.
pub(super) fn col_rule_bounds(vs: &[VLine], r: &Region) -> Vec<f64> {
    let xs: Vec<(f64, f64)> = vs
        .iter()
        .filter(|l| v_near(l, r))
        .map(|l| (l.x, l.b - l.a))
        .collect();
    rule_bounds(xs, COL_RULE_COVER * (r.y1 - r.y0))
}
