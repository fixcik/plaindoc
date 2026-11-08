//! Planar cell partition of a lined region: elementary cells between all rulings, neighbours merged
//! when no ruling separates them, which yields colspan/rowspan (proto.py `lattice`).

use super::prims::{Prims, Word};
use super::stats::{Dsu, cluster, py_round};
use super::{LimitExceeded, MAX_CELLS};

/// Rulings closer than this are one grid line (proto.py `PTOL`).
const PTOL: f64 = 2.0;
/// A ruling belongs to a grid line within this distance (`PTOL * 1.5`).
const LINE_SNAP: f64 = PTOL * 1.5;
/// Collinear ruling pieces closer than this are one interval.
const JOIN_GAP: f64 = 1.0;
/// An edge counts as drawn when a ruling covers it up to this slack.
const COVER_TOL: f64 = 2.5;
/// Raster rulings (Task 12(c), REPORT §8: D11 hybrid on raster lines only after this length filter, else
/// TEDS drops 0.01-0.02): at least this share of the region side, and joined across gaps up to
/// [`RASTER_JOIN_GAP`] (rounded cell corners leave 4-8 pt, which [`COVER_TOL`] does not close).
const RASTER_MIN_SIDE: f64 = 0.3;
const RASTER_JOIN_GAP: f64 = 8.0;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PCell {
    pub text: String,
    pub cs: u32,
    pub rs: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Planar {
    /// Cells listed in the row where they start (HTML order).
    pub rows: Vec<Vec<PCell>>,
    /// Merged components that are not rectangles.
    pub nonrect: usize,
    pub ncols: usize,
    pub nrows: usize,
    pub ncells: usize,
    /// Share of elementary edges drawn by rulings.
    pub edge_cov: f64,
}

/// Total length of `iv` after [`merge_intervals`].
pub(crate) fn covered_len(iv: Vec<(f64, f64)>, tol: f64) -> f64 {
    merge_intervals(iv, tol)
        .iter()
        .fold(0.0, |s, &(a, b)| s + (b - a))
}

/// Sorted `(a, b)` intervals with those closer than `tol` joined.
pub(crate) fn merge_intervals(mut iv: Vec<(f64, f64)>, tol: f64) -> Vec<(f64, f64)> {
    iv.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (a, b) in iv {
        match out.last_mut() {
            Some(o) if a <= o.1 + tol => o.1 = o.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Join gap of one grid line: [`RASTER_JOIN_GAP`] when a raster ruling lies on it.
fn join_gap(raster: bool) -> f64 {
    if raster { RASTER_JOIN_GAP } else { JOIN_GAP }
}

fn covered(iv: &[(f64, f64)], a: f64, b: f64) -> bool {
    iv.iter()
        .any(|&(s, e)| s <= a + COVER_TOL && e >= b - COVER_TOL)
}

fn cell_text(mut ws: Vec<&Word>) -> String {
    ws.sort_by(|a, b| {
        py_round(a.cy / 4.0)
            .cmp(&py_round(b.cy / 4.0))
            .then(a.x0.total_cmp(&b.x0))
    });
    let mut lines: Vec<Vec<&str>> = Vec::new();
    let mut last: Option<f64> = None;
    for w in ws {
        match last {
            Some(l) if (w.cy - l).abs() <= 4.0 => lines.last_mut().expect("line").push(&w.t),
            _ => lines.push(vec![&w.t]),
        }
        last = Some(w.cy);
    }
    lines
        .iter()
        .map(|l| l.join(" "))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Planar partition of `bbox = [x0, top, x1, bottom]`; `None` when it has no cell. Over
/// [`MAX_CELLS`] elementary cells nothing is allocated. The raster rules apply to raster rulings (marked
/// only under `TableConfig::short_vertical_stacks`).
pub(crate) fn lattice(
    ws: &[Word],
    p: &Prims,
    bbox: [f64; 4],
) -> Result<Option<Planar>, LimitExceeded> {
    let [x0, y0, x1, y1] = bbox;
    let hs: Vec<_> =
        p.hs.iter()
            .filter(|h| y0 - 3.0 <= h.y && h.y <= y1 + 3.0 && h.b >= x0 - 3.0 && h.a <= x1 + 3.0)
            .filter(|h| !(h.raster && h.b - h.a < RASTER_MIN_SIDE * (x1 - x0)))
            .collect();
    let vs: Vec<_> =
        p.vs.iter()
            .filter(|v| x0 - 3.0 <= v.x && v.x <= x1 + 3.0 && v.b >= y0 - 3.0 && v.a <= y1 + 3.0)
            .filter(|v| !(v.raster && v.b - v.a < RASTER_MIN_SIDE * (y1 - y0)))
            .collect();
    let xs = cluster(
        &vs.iter().map(|v| v.x).chain([x0, x1]).collect::<Vec<_>>(),
        PTOL,
    );
    let ys = cluster(
        &hs.iter().map(|h| h.y).chain([y0, y1]).collect::<Vec<_>>(),
        PTOL,
    );
    if xs.len() < 2 || ys.len() < 2 {
        return Ok(None);
    }
    let (nc, nr) = (xs.len() - 1, ys.len() - 1);
    LimitExceeded::check("cells of a grid", nc.saturating_mul(nr), MAX_CELLS)?;
    let vcov: Vec<_> = xs
        .iter()
        .map(|&x| {
            let on = || vs.iter().filter(|v| (v.x - x).abs() <= LINE_SNAP);
            let gap = join_gap(on().any(|v| v.raster));
            merge_intervals(on().map(|v| (v.a, v.b)).collect(), gap)
        })
        .collect();
    let hcov: Vec<_> = ys
        .iter()
        .map(|&y| {
            let on = || hs.iter().filter(|h| (h.y - y).abs() <= LINE_SNAP);
            let gap = join_gap(on().any(|h| h.raster));
            merge_intervals(on().map(|h| (h.a, h.b)).collect(), gap)
        })
        .collect();
    let idx = |i: usize, j: usize| j * nc + i;
    let mut d = Dsu::new(nc * nr);
    let (mut nedge, mut ncov) = (0usize, 0usize);
    for j in 0..nr {
        for i in 0..nc {
            if i + 1 < nc {
                nedge += 1;
                if covered(&vcov[i + 1], ys[j], ys[j + 1]) {
                    ncov += 1;
                } else {
                    d.union(idx(i, j), idx(i + 1, j));
                }
            }
            if j + 1 < nr {
                nedge += 1;
                if covered(&hcov[j + 1], xs[i], xs[i + 1]) {
                    ncov += 1;
                } else {
                    d.union(idx(i, j), idx(i, j + 1));
                }
            }
        }
    }
    // Components: (i0, i1, j0, j1, members), keyed by root in first-seen order.
    let mut slot: Vec<Option<usize>> = vec![None; nc * nr];
    let mut comps: Vec<(usize, usize, usize, usize, usize)> = Vec::new();
    let mut cell_of = vec![0usize; nc * nr];
    for j in 0..nr {
        for i in 0..nc {
            let r = d.find(idx(i, j));
            let k = *slot[r].get_or_insert_with(|| {
                comps.push((i, i, j, j, 0));
                comps.len() - 1
            });
            let c = &mut comps[k];
            c.0 = c.0.min(i);
            c.1 = c.1.max(i);
            c.2 = c.2.min(j);
            c.3 = c.3.max(j);
            c.4 += 1;
            cell_of[idx(i, j)] = k;
        }
    }
    let mut words: Vec<Vec<&Word>> = vec![Vec::new(); comps.len()];
    for w in ws {
        if !(x0 - 2.0 <= w.cx && w.cx <= x1 + 2.0 && y0 - 2.0 <= w.cy && w.cy <= y1 + 2.0) {
            continue;
        }
        let i =
            (xs.partition_point(|&x| x <= w.cx) as isize - 1).clamp(0, nc as isize - 1) as usize;
        let j =
            (ys.partition_point(|&y| y <= w.cy) as isize - 1).clamp(0, nr as isize - 1) as usize;
        words[cell_of[idx(i, j)]].push(w);
    }
    let mut order: Vec<usize> = (0..comps.len()).collect();
    order.sort_by_key(|&k| (comps[k].2, comps[k].0));
    let mut rows: Vec<Vec<PCell>> = vec![Vec::new(); nr];
    let mut nonrect = 0;
    for k in order {
        let (i0, i1, j0, j1, n) = comps[k];
        let (cs, rs) = (i1 - i0 + 1, j1 - j0 + 1);
        if n != cs * rs {
            nonrect += 1;
        }
        rows[j0].push(PCell {
            text: cell_text(std::mem::take(&mut words[k])),
            cs: cs as u32,
            rs: rs as u32,
        });
    }
    Ok(Some(Planar {
        rows,
        nonrect,
        ncols: nc,
        nrows: nr,
        ncells: comps.len(),
        edge_cov: ncov as f64 / nedge.max(1) as f64,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::prims::{prims, prims_with, words_of};
    use crate::tables::testkit::{grid_page, hline, vline};

    #[test]
    fn full_grid_has_unit_cells_and_full_cover() {
        let page = grid_page(
            &[50.0, 150.0, 250.0],
            &[100.0, 120.0, 140.0],
            &[("a", 60.0, 105.0), ("b", 160.0, 125.0)],
        );
        let pl = lattice(&words_of(&page), &prims(&page), [50.0, 100.0, 250.0, 140.0])
            .unwrap()
            .unwrap();
        assert_eq!((pl.nrows, pl.ncols, pl.ncells, pl.nonrect), (2, 2, 4, 0));
        assert_eq!(pl.edge_cov, 1.0);
        assert_eq!(pl.rows[0][0].text, "a");
        assert_eq!(pl.rows[1][1].text, "b");
    }

    #[test]
    fn raster_lattice_takes_long_pieces_and_joins_corner_gaps() {
        // 2×2 grid; the middle vertical is two raster pieces 6 pt apart (a rounded corner) and a raster
        // 8-pt tick at x = 100 (under 0.3 of the 40-pt height) is noise.
        let page = |raster: bool| {
            let mut p = grid_page(&[], &[], &[("a", 60.0, 105.0), ("b", 160.0, 125.0)]);
            let mut seg = |x1: f64, y1: f64, x2: f64, y2: f64| {
                p.segments.push(crate::tables::InputSegment {
                    x1,
                    y1,
                    x2,
                    y2,
                    raster,
                })
            };
            for y in [100.0, 120.0, 140.0] {
                seg(50.0, y, 250.0, y);
            }
            for x in [50.0, 250.0] {
                seg(x, 100.0, x, 140.0);
            }
            seg(150.0, 100.0, 150.0, 117.0);
            seg(150.0, 123.0, 150.0, 140.0);
            seg(100.0, 100.0, 100.0, 108.0);
            p
        };
        let run = |raster: bool, rule: bool| {
            let pg = page(raster);
            let pl = lattice(
                &words_of(&pg),
                &prims_with(&pg, rule),
                [50.0, 100.0, 250.0, 140.0],
            )
            .unwrap()
            .unwrap();
            (pl.nrows, pl.ncols, pl.ncells, pl.edge_cov)
        };
        assert_eq!(run(true, true), (2, 2, 4, 1.0));
        let off = run(true, false);
        assert_eq!(off.1, 3, "the tick is a column without the rule");
        assert_eq!(run(false, true), off, "PDF segments are untouched");
    }

    #[test]
    fn missing_inner_ruling_gives_colspan() {
        // Header row without the middle vertical: one cell spanning two columns.
        let mut page = grid_page(
            &[],
            &[],
            &[
                ("Head", 120.0, 105.0),
                ("x", 60.0, 125.0),
                ("y", 160.0, 125.0),
            ],
        );
        for x in [50.0, 250.0] {
            page.paths.push(vline(x, 100.0, 140.0));
        }
        page.paths.push(vline(150.0, 120.0, 140.0));
        for y in [100.0, 120.0, 140.0] {
            page.paths.push(hline(50.0, 250.0, y));
        }
        let pl = lattice(&words_of(&page), &prims(&page), [50.0, 100.0, 250.0, 140.0])
            .unwrap()
            .unwrap();
        assert_eq!(pl.rows[0].len(), 1);
        assert_eq!(
            (
                pl.rows[0][0].cs,
                pl.rows[0][0].rs,
                pl.rows[0][0].text.as_str()
            ),
            (2, 1, "Head")
        );
        assert_eq!(pl.rows[1].len(), 2);
        assert_eq!(pl.nonrect, 0);
    }
}
