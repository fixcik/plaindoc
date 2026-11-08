//! Lined candidates (`L`): regions of connected rulings and boxes, `extend_first_column`, `band_split`.

use super::bounds::{col_rule_bounds, region_lines, rule_bounds, v_near, valley_bounds};
use super::{
    BAND_FULL_SHARE, CandidateKind, FIRST_COL_BAND_SHARE, FIRST_COL_LINE_SHARE, FIRST_COL_MARGIN,
    FIRST_COL_MIN_REACH, ROW_RULE_COVER, Raw,
};
use crate::tables::grid::{build_cells, finalize, row_bounds_from_text, text_lines};
use crate::tables::prims::{Prims, Region, VLine, Word, stack_raster_verticals};
use crate::tables::stats::median;
use crate::tables::{LimitExceeded, TableConfig};

/// Words ordered by centre x and by centre y, to find the words `inside` a region without scanning them all.
/// Words with a NaN centre are never inside anything and are left out.
pub(super) struct WordIndex {
    by_x: Vec<(f64, usize)>,
    by_y: Vec<(f64, usize)>,
}

impl WordIndex {
    pub(super) fn new(ws: &[Word]) -> Self {
        let sorted = |c: fn(&Word) -> f64| {
            let mut v: Vec<(f64, usize)> = ws
                .iter()
                .enumerate()
                .filter(|(_, w)| !c(w).is_nan())
                .map(|(i, w)| (c(w), i))
                .collect();
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
            v
        };
        Self {
            by_x: sorted(|w| w.cx),
            by_y: sorted(|w| w.cy),
        }
    }

    /// Indices (ascending) of the words with `inside(w, r, m)`; scans the shorter of the two bands.
    pub(super) fn inside(&self, ws: &[Word], r: &Region, m: f64) -> Vec<usize> {
        let (xl, xh, yl, yh) = (r.x0 - m, r.x1 + m, r.y0 - m, r.y1 + m);
        if xl.is_nan() || xh.is_nan() || yl.is_nan() || yh.is_nan() {
            return Vec::new();
        }
        let band = |v: &[(f64, usize)], lo: f64, hi: f64| {
            let a = v.partition_point(|e| e.0 < lo);
            let b = v.partition_point(|e| e.0 <= hi);
            a..b.max(a)
        };
        let (rx, ry) = (band(&self.by_x, xl, xh), band(&self.by_y, yl, yh));
        let mut out: Vec<usize> = if rx.len() <= ry.len() {
            self.by_x[rx].iter().map(|e| e.1).collect()
        } else {
            self.by_y[ry].iter().map(|e| e.1).collect()
        };
        out.retain(|&i| {
            let w = &ws[i];
            xl <= w.cx && w.cx <= xh && yl <= w.cy && w.cy <= yh
        });
        out.sort_unstable();
        out
    }
}

/// Moves `x0` of a region left over the row labels of a first column the frame did not enclose (Task 12(b)).
/// A label is a word left of the region, within [`FIRST_COL_MIN_REACH`] / the median column width, in a text
/// line that continues into the region; the labels must sit in at least [`FIRST_COL_LINE_SHARE`] of the
/// region's lines and, so that margin line numbers do not count, at least [`FIRST_COL_BAND_SHARE`] of the page
/// words in their x-band must lie inside the region's y-band.
pub(super) fn extend_first_column(regs: &mut [Region], ws: &[Word], p: &Prims) {
    let all: Vec<&Word> = ws.iter().collect();
    let page_lines = text_lines(&all);
    for r in regs.iter_mut() {
        let colb = col_rule_bounds(&p.vs, r);
        let widths: Vec<f64> = colb.windows(2).map(|c| c[1] - c[0]).collect();
        let reach = FIRST_COL_MIN_REACH.max(if widths.is_empty() {
            0.0
        } else {
            median(&widths)
        });
        let in_band = |w: &Word| r.y0 <= w.cy && w.cy <= r.y1;
        let in_region = |w: &Word| in_band(w) && r.x0 - 2.0 <= w.cx && w.cx <= r.x1 + 2.0;
        let mut labels: Vec<&Word> = Vec::new();
        let (mut lines, mut labelled) = (0usize, 0usize);
        for l in &page_lines {
            if !(r.y0 <= l.cy && l.cy <= r.y1 && l.w.iter().any(|w| in_region(w))) {
                continue;
            }
            lines += 1;
            let ls: Vec<&Word> =
                l.w.iter()
                    .copied()
                    .filter(|w| w.cx < r.x0 - 2.0 && r.x0 - reach <= w.x0)
                    .collect();
            labelled += usize::from(!ls.is_empty());
            labels.extend(ls);
        }
        if labels.is_empty() || (labelled as f64) < FIRST_COL_LINE_SHARE * lines as f64 {
            continue;
        }
        let (lo, hi) = labels
            .iter()
            .fold((f64::MAX, f64::MIN), |a, w| (a.0.min(w.x0), a.1.max(w.x1)));
        let band: Vec<&Word> = ws.iter().filter(|w| lo <= w.cx && w.cx <= hi).collect();
        let inside_band = band.iter().filter(|w| in_band(w)).count();
        if (inside_band as f64) < FIRST_COL_BAND_SHARE * band.len() as f64 {
            continue;
        }
        r.x0 = lo - FIRST_COL_MARGIN;
    }
}

pub(crate) fn detect_lined<'a>(
    ws: &'a [Word],
    p: &Prims,
    regs: &[Region],
    cfg: &TableConfig,
) -> Result<Vec<Raw<'a>>, LimitExceeded> {
    let mut out = Vec::new();
    let index = WordIndex::new(ws);
    // `short_vertical_stacks` (Task 12(c)): raster vertical pieces belonging to no region (each a stray
    // component, ends more than `REGION_TOL` apart) still join the stacks of the region they stand in.
    let stray: Vec<VLine> = if cfg.short_vertical_stacks && cfg.comp_lines {
        let nh = p.hs.len();
        let mut claimed = vec![false; p.vs.len()];
        for i in regs.iter().flat_map(|r| &r.members) {
            if let Some(c) = i.checked_sub(nh).and_then(|k| claimed.get_mut(k)) {
                *c = true;
            }
        }
        p.vs.iter()
            .zip(&claimed)
            .filter(|(l, c)| l.raster && !**c)
            .map(|(l, _)| *l)
            .collect()
    } else {
        Vec::new()
    };
    for r in regs {
        let rw: Vec<&Word> = index
            .inside(ws, r, 2.0)
            .into_iter()
            .map(|i| &ws[i])
            .collect();
        if rw.len() < 4 {
            continue;
        }
        let w = r.x1 - r.x0;
        // A3: only the rulings of the region's own component (a nested grid is not a row/column split).
        let sub = (cfg.comp_lines && !r.members.is_empty()).then(|| p.subset(&r.members));
        let q = sub.as_ref().unwrap_or(p);
        let yl: Vec<(f64, f64)> =
            q.hs.iter()
                .filter(|l| {
                    r.x0 - 3.0 <= l.a && l.b <= r.x1 + 3.0 && r.y0 - 3.0 <= l.y && l.y <= r.y1 + 3.0
                })
                .map(|l| (l.y, l.b - l.a))
                .collect();
        let rowb = rule_bounds(yl, ROW_RULE_COVER * w);
        let mut colb = if cfg.short_vertical_stacks {
            // Raster pieces |Δx| <= 2 pt, gaps <= 8 pt apart are one ruling, gaps counted (Task 12(c)).
            let pool: Vec<VLine> =
                (q.vs
                    .iter()
                    .chain(if sub.is_some() { &stray[..] } else { &[] }))
                .filter(|l| v_near(l, r))
                .copied()
                .collect();
            col_rule_bounds(&stack_raster_verticals(pool), r)
        } else {
            col_rule_bounds(&q.vs, r)
        };
        let lines = region_lines(&rw, cfg);
        if colb.len() < 3 {
            colb = valley_bounds(&rw, &lines, r, false, false, cfg.fs_tolerances);
        } else {
            if colb[0] > r.x0 + 8.0 {
                colb.push(r.x0);
                colb.push(r.x1);
                colb.sort_by(f64::total_cmp);
                colb.dedup();
            }
            if colb[colb.len() - 1] < r.x1 - 8.0 {
                colb.push(r.x1);
            }
        }
        let text_rows = rowb.len() < 3;
        let rowb = if !text_rows {
            let mut rb = rowb;
            rb.sort_by(f64::total_cmp);
            if rb[0] > r.y0 + 8.0 {
                rb.insert(0, r.y0);
            }
            if rb[rb.len() - 1] < r.y1 - 8.0 {
                rb.push(r.y1);
            }
            rb
        } else {
            row_bounds_from_text(&lines, &colb, r.y0, r.y1, cfg.row_newcols)
        };
        if let Some(rows) = finalize(build_cells(&rw, &colb, &rowb)?) {
            out.push(Raw {
                kind: CandidateKind::Lined,
                bbox: [r.x0, r.y0, r.x1, r.y1],
                rows,
                words: rw,
                colb,
                rowb,
                sub,
                head: 0,
                text_rows,
            });
        }
    }
    Ok(out)
}

/// `band_split`: the grid of a lined candidate with the bands between its row bounds cut into rows by their
/// text (`row_bounds_from_text`) where a band holds >= 2 text lines that each reach [`BAND_FULL_SHARE`] of
/// the columns (and at least 2); `None` when no band splits or the cut grid is rejected by `finalize`.
pub(super) fn split_bands(
    words: &[&Word],
    colb: &[f64],
    rowb: &[f64],
    cfg: &TableConfig,
) -> Result<Option<Vec<Vec<String>>>, LimitExceeded> {
    let nc = colb.len().saturating_sub(1);
    if nc < 2 || rowb.is_empty() {
        return Ok(None);
    }
    let (cl, cr) = (colb[0] - 2.0, colb[nc] + 2.0);
    let (rt, rb) = (rowb[0] - 2.0, rowb[rowb.len() - 1] + 2.0);
    let ws: Vec<&Word> = words
        .iter()
        .copied()
        .filter(|w| cl <= w.cx && w.cx <= cr && rt <= w.cy && w.cy <= rb)
        .collect();
    let need = 2.max((BAND_FULL_SHARE * nc as f64).ceil() as usize);
    let mut new = vec![rowb[0]];
    let mut changed = false;
    for ab in rowb.windows(2) {
        let (a, b) = (ab[0], ab[1]);
        let bw: Vec<&Word> = ws
            .iter()
            .copied()
            .filter(|w| a <= w.cy && w.cy < b)
            .collect();
        let lines = text_lines(&bw);
        let full = lines
            .iter()
            .filter(|l| {
                let mut cols: Vec<usize> =
                    l.w.iter()
                        .map(|w| {
                            let k = colb.partition_point(|&c| w.cx >= c);
                            k.saturating_sub(1).min(nc - 1)
                        })
                        .collect();
                cols.sort();
                cols.dedup();
                cols.len() >= need
            })
            .count();
        if lines.len() >= 2 && full >= 2 {
            let b = row_bounds_from_text(&lines, colb, a, b, cfg.row_newcols);
            let inner = &b[1..b.len() - 1];
            if !inner.is_empty() {
                new.extend_from_slice(inner);
                changed = true;
            }
        }
        new.push(b);
    }
    if !changed {
        return Ok(None);
    }
    Ok(finalize(build_cells(&ws, colb, &new)?))
}
