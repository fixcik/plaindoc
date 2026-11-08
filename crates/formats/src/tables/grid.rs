//! Text lines, column valleys, row bounds and the cell grid (proto.py `text_lines`, `valleys`,
//! `build_cells`, `row_bounds_from_text`, `finalize`).

use std::cmp::Ordering;

use super::prims::Word;
use super::stats::py_round;
use super::{LimitExceeded, MAX_CELLS};

/// Words whose centres differ by at most this share of the font size are one line.
const LINE_CY_TOL_FS: f64 = 0.5;
/// A text line farther than this many line heights from the previous one starts a new row.
const ROW_GAP_LH: f64 = 2.2;
/// `finalize`: a grid needs this share of non-empty cells.
const MIN_FILL: f64 = 0.35;
/// `finalize` (TOCF=1): a 2-column grid whose right column is >80% page numbers is a table of contents.
const TOC_NUM_SHARE: f64 = 0.8;

#[derive(Clone, Debug)]
pub(crate) struct Line<'a> {
    pub cy: f64,
    pub w: Vec<&'a Word>,
}

/// Groups words into lines: sort by (cy, x0), running-mean centre, words sorted by x0.
pub(crate) fn text_lines<'a>(ws: &[&'a Word]) -> Vec<Line<'a>> {
    text_lines_at(ws, None)
}

/// [`text_lines`] with the merge tolerance on font size `fs` (the region median, `fs_tolerances`) instead
/// of each word's own.
pub(crate) fn text_lines_at<'a>(ws: &[&'a Word], fs: Option<f64>) -> Vec<Line<'a>> {
    let mut s: Vec<&Word> = ws.to_vec();
    s.sort_by(|a, b| a.cy.total_cmp(&b.cy).then(a.x0.total_cmp(&b.x0)));
    let mut lines: Vec<Line> = Vec::new();
    for w in s {
        let tol = match fs {
            None => LINE_CY_TOL_FS * w.fs.max(1.0),
            Some(f) => LINE_CY_TOL_FS * f,
        };
        match lines.last_mut() {
            Some(l) if (l.cy - w.cy).abs() <= tol => {
                l.w.push(w);
                let n = l.w.len() as f64;
                l.cy = (l.cy * (n - 1.0) + w.cy) / n;
            }
            _ => lines.push(Line {
                cy: w.cy,
                w: vec![w],
            }),
        }
    }
    for l in &mut lines {
        l.w.sort_by(|a, b| a.x0.total_cmp(&b.x0));
    }
    lines
}

/// Slack of the coverage test of [`valleys`] and of the merge of adjacent gaps.
const VALLEY_EPS: f64 = 1e-6;

/// X positions of vertical white gaps at least `gmin` wide, covered by at most `tol_cnt` words.
///
/// O(n log n): a word covers the elementary intervals `i` with `x0 < xs[i+1] - eps` and `x1 > xs[i] + eps`,
/// both monotone in `i`, so the covered ones form a range found by binary search and counted with a
/// difference array. Words with a NaN edge cover nothing, as with a direct comparison.
pub(crate) fn valleys(ws: &[&Word], gmin: f64, tol_cnt: usize) -> Vec<f64> {
    let mut xs: Vec<f64> = ws.iter().flat_map(|w| [w.x0, w.x1]).collect();
    xs.sort_by(f64::total_cmp);
    xs.dedup();
    // NaN edges sort to both ends; intervals touching them have no coverage.
    let lo = xs.partition_point(|v| v.is_nan());
    let hi = lo + xs[lo..].partition_point(|v| !v.is_nan());
    let core = &xs[lo..hi];
    let m = core.len().saturating_sub(1);
    let mut diff = vec![0i64; m + 1];
    if m > 0 {
        for w in ws {
            if w.x0.is_nan() || w.x1.is_nan() {
                continue;
            }
            let first = core[1..].partition_point(|&b| b - VALLEY_EPS <= w.x0);
            let end = core[..m].partition_point(|&a| a + VALLEY_EPS < w.x1);
            if first < end {
                diff[first] += 1;
                diff[end] -= 1;
            }
        }
    }
    let mut gaps: Vec<(f64, f64)> = Vec::new();
    let mut c = 0i64;
    for (i, win) in xs.windows(2).enumerate() {
        let (a, b) = (win[0], win[1]);
        let cnt = match i.checked_sub(lo) {
            Some(j) if j < m => {
                c += diff[j];
                c
            }
            _ => 0,
        };
        if cnt <= tol_cnt as i64 {
            // Python keeps `cur` across covered intervals; contiguity makes that a plain "extend last".
            match gaps.last_mut() {
                Some(g) if (g.1 - a).abs() < VALLEY_EPS => g.1 = b,
                _ => gaps.push((a, b)),
            }
        }
    }
    gaps.into_iter()
        .filter(|(a, b)| b - a >= gmin)
        .map(|(a, b)| (a + b) / 2.0)
        .collect()
}

/// Index of the first interval `[b[i], b[i+1])` holding `v` (Python `next(...)`).
pub(crate) fn slot(bounds: &[f64], v: f64) -> Option<usize> {
    (0..bounds.len().saturating_sub(1)).find(|&i| bounds[i] <= v && v < bounds[i + 1])
}

/// Cell texts of a column/row grid; words go by centre, text in (round(cy/3), x0) order.
/// Over [`MAX_CELLS`](super::MAX_CELLS) nothing is allocated.
pub(crate) fn build_cells(
    ws: &[&Word],
    colb: &[f64],
    rowb: &[f64],
) -> Result<Vec<Vec<String>>, LimitExceeded> {
    let (nc, nr) = (colb.len().saturating_sub(1), rowb.len().saturating_sub(1));
    LimitExceeded::check("cells of a grid", nc.saturating_mul(nr), MAX_CELLS)?;
    let mut cell: Vec<Vec<Vec<&Word>>> = vec![vec![Vec::new(); nc]; nr];
    for &w in ws {
        if let (Some(ci), Some(ri)) = (slot(colb, w.cx), slot(rowb, w.cy)) {
            cell[ri][ci].push(w);
        }
    }
    Ok(cell
        .into_iter()
        .map(|r| {
            r.into_iter()
                .map(|mut c| {
                    c.sort_by(|a, b| {
                        py_round(a.cy / 3.0)
                            .cmp(&py_round(b.cy / 3.0))
                            .then(a.x0.total_cmp(&b.x0))
                    });
                    c.iter().map(|w| w.t.as_str()).collect::<Vec<_>>().join(" ")
                })
                .collect()
        })
        .collect())
}

/// `row_newcols` (T17, proto2.py): a line taking at least this many columns empty in the current row,
/// and at least this share of its own columns, starts a row (a sub-header under a merged cell).
const NEWCOLS_FRESH_MIN: usize = 2;
const NEWCOLS_FRESH_SHARE: f64 = 0.5;
/// Modes 2/3: a "full" line (at least this many columns and this share of all columns) starts a row.
const NEWCOLS_FULL_MIN: usize = 3;
const NEWCOLS_FULL_SHARE: f64 = 0.6;
/// Mode 3: a full line starts a row only farther than this many line heights from the previous line.
const NEWCOLS_FULL_GAP_LH: f64 = 1.25;

/// Row bounds from text lines: a line starts a row when it has a word left of the second column
/// bound or follows a big vertical gap. `newcols` (proto2.py `row_newcols`): 0 off; 1 also a line
/// taking new columns ([`NEWCOLS_FRESH_MIN`]); 2 also a full line; 3 a full line only after a gap of
/// [`NEWCOLS_FULL_GAP_LH`] line heights.
pub(crate) fn row_bounds_from_text(
    lines: &[Line],
    colb: &[f64],
    y0: f64,
    y1: f64,
    newcols: u8,
) -> Vec<f64> {
    let fc = colb[1];
    let nc = colb.len() - 1;
    // Columns of the current row: `occ[c] == row` (stamps instead of clearing a set).
    let mut occ = vec![0usize; if newcols > 0 { nc } else { 0 }];
    let mut row = 0usize;
    let mut cols: Vec<usize> = Vec::new();
    let mut starts = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let first = l.w.iter().any(|w| w.cx < fc);
        let gap = if i > 0 { l.cy - lines[i - 1].cy } else { 99.0 };
        let lh = l.w.iter().map(|w| w.fs).fold(f64::MIN, f64::max);
        let mut new = i == 0 || first || gap > ROW_GAP_LH * lh;
        if newcols == 0 {
            if new {
                starts.push(i);
            }
            continue;
        }
        // Python `bisect_right(colb, cx) - 1`, clamped to the columns.
        cols.clear();
        cols.extend(l.w.iter().map(|w| {
            let k = colb.partition_point(|&c| w.cx.partial_cmp(&c) != Some(Ordering::Less));
            k.saturating_sub(1).min(nc - 1)
        }));
        cols.sort();
        cols.dedup();
        if !new {
            let fresh = cols.iter().filter(|&&c| occ[c] != row).count();
            let full = cols.len() >= NEWCOLS_FULL_MIN
                && cols.len() as f64 >= NEWCOLS_FULL_SHARE * nc as f64;
            new = (fresh >= NEWCOLS_FRESH_MIN
                && fresh as f64 >= NEWCOLS_FRESH_SHARE * cols.len() as f64)
                || (newcols == 2 && full)
                || (newcols == 3 && full && gap > NEWCOLS_FULL_GAP_LH * lh);
        }
        if new {
            starts.push(i);
            row += 1;
        }
        for &c in &cols {
            occ[c] = row;
        }
    }
    let mut b = vec![y0];
    b.extend(
        starts
            .iter()
            .skip(1)
            .map(|&s| (lines[s - 1].cy + lines[s].cy) / 2.0),
    );
    b.push(y1 + 1.0);
    b
}

fn is_toc_number(s: &str) -> bool {
    !s.is_empty()
        && s.to_lowercase()
            .chars()
            .all(|c| c.is_ascii_digit() || "ivxlc-–".contains(c))
}

/// Drops empty rows/columns and rejects TOC-like or sparse grids (proto.py `finalize`, TOCF=1).
pub(crate) fn finalize(g: Vec<Vec<String>>) -> Option<Vec<Vec<String>>> {
    let g: Vec<Vec<String>> = g
        .into_iter()
        .filter(|r| r.iter().any(|c| !c.trim().is_empty()))
        .collect();
    if g.len() < 2 {
        return None;
    }
    let keep: Vec<usize> = (0..g[0].len())
        .filter(|&j| g.iter().any(|r| !r[j].trim().is_empty()))
        .collect();
    if keep.len() < 2 {
        return None;
    }
    let g: Vec<Vec<String>> = g
        .into_iter()
        .map(|mut r| keep.iter().map(|&j| std::mem::take(&mut r[j])).collect())
        .collect();
    if g[0].len() == 2 {
        let last: Vec<&str> = g
            .iter()
            .map(|r| r[1].trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !last.is_empty()
            && last.iter().filter(|s| is_toc_number(s)).count() as f64 / last.len() as f64
                > TOC_NUM_SHARE
        {
            return None;
        }
    }
    let filled = g.iter().flatten().filter(|c| !c.trim().is_empty()).count();
    if (filled as f64) / ((g.len() * g[0].len()) as f64) < MIN_FILL {
        return None;
    }
    Some(g)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::testkit::word;

    #[test]
    fn row_newcols_starts_rows_on_new_and_full_columns() {
        let colb = [0.0, 100.0, 200.0, 300.0, 400.0];
        let ws = [
            word("Name", 10.0, 100.0, 10.0),
            word("Temp", 110.0, 100.0, 10.0),
            // Sub-header under the merged cell: two columns empty in the row so far.
            word("max", 210.0, 112.0, 10.0),
            word("min", 310.0, 112.0, 10.0),
            // Full lines (3 of 4 columns) without a first-column word, 12 and 14 pt down.
            word("a", 110.0, 124.0, 10.0),
            word("b", 210.0, 124.0, 10.0),
            word("c", 310.0, 124.0, 10.0),
            word("d", 110.0, 138.0, 10.0),
            word("e", 210.0, 138.0, 10.0),
            word("f", 310.0, 138.0, 10.0),
        ];
        let refs: Vec<&Word> = ws.iter().collect();
        let lines = text_lines(&refs);
        let rows = |mode| row_bounds_from_text(&lines, &colb, 90.0, 150.0, mode).len() - 1;
        assert_eq!(rows(0), 1);
        assert_eq!(rows(1), 2, "new columns");
        assert_eq!(rows(2), 4, "and full lines");
        assert_eq!(rows(3), 3, "full lines only after 1.25 line heights");
    }

    #[test]
    fn lines_use_running_mean_and_sort_by_x() {
        let ws = [
            word("b", 60.0, 100.0, 10.0),
            word("a", 10.0, 101.0, 10.0),
            word("c", 10.0, 130.0, 10.0),
        ];
        let refs: Vec<&Word> = ws.iter().collect();
        let l = text_lines(&refs);
        assert_eq!(l.len(), 2);
        assert_eq!(
            l[0].w.iter().map(|w| w.t.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!((l[0].cy - (ws[0].cy + ws[1].cy) / 2.0).abs() < 1e-9);
    }

    #[test]
    fn valleys_find_column_gaps() {
        let ws = [
            word("aaaa", 10.0, 100.0, 10.0),
            word("bbbb", 100.0, 100.0, 10.0),
            word("cccc", 10.0, 120.0, 10.0),
        ];
        let refs: Vec<&Word> = ws.iter().collect();
        let v = valleys(&refs, 7.0, 0);
        assert_eq!(v.len(), 1);
        assert!(v[0] > ws[0].x1 && v[0] < ws[1].x0);
    }

    /// The original O(n^2) `valleys`, kept as the reference the fast one must match bit for bit.
    fn valleys_quadratic(ws: &[&Word], gmin: f64, tol_cnt: usize) -> Vec<f64> {
        let mut xs: Vec<f64> = ws.iter().flat_map(|w| [w.x0, w.x1]).collect();
        xs.sort_by(f64::total_cmp);
        xs.dedup();
        let mut gaps: Vec<(f64, f64)> = Vec::new();
        for win in xs.windows(2) {
            let (a, b) = (win[0], win[1]);
            let c = ws
                .iter()
                .filter(|w| w.x0 < b - 1e-6 && w.x1 > a + 1e-6)
                .count();
            if c <= tol_cnt {
                match gaps.last_mut() {
                    Some(g) if (g.1 - a).abs() < 1e-6 => g.1 = b,
                    _ => gaps.push((a, b)),
                }
            }
        }
        gaps.into_iter()
            .filter(|(a, b)| b - a >= gmin)
            .map(|(a, b)| (a + b) / 2.0)
            .collect()
    }

    fn bits(v: &[f64]) -> Vec<u64> {
        v.iter().map(|x| x.to_bits()).collect()
    }

    #[test]
    fn valleys_match_the_quadratic_reference() {
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        // Edge values: grid points, touching/overlapping, sub-epsilon offsets, signed zeros, NaN, infinities.
        let special = [
            0.0,
            -0.0,
            1e-7,
            -1e-7,
            1e-6,
            2e-6,
            f64::NAN,
            -f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        let mut checked = 0;
        for case in 0..3000 {
            let n = 1 + next(if case % 50 == 0 { 120 } else { 14 }) as usize;
            let (step, shift) =
                [(1.0, 0.0), (0.5, 0.25), (1e-6, 3.0), (7.3, -20.0)][next(4) as usize];
            let nan_ok = case % 7 == 0;
            let coord = |next: &mut dyn FnMut(u64) -> u64| {
                if next(12) == 0 {
                    let sp = special[next(special.len() as u64) as usize];
                    if sp.is_nan() && !nan_ok { 0.0 } else { sp }
                } else {
                    shift
                        + step * next(10) as f64
                        + [0.0, 0.0, 5e-7, 1e-6, 1.5e-6][next(5) as usize]
                }
            };
            let ws: Vec<Word> = (0..n)
                .map(|_| {
                    let a = coord(&mut next);
                    // Width: zero, positive, sometimes negative (x1 < x0).
                    let b = match next(6) {
                        0 => a,
                        1 => coord(&mut next),
                        _ => a + step * next(5) as f64,
                    };
                    let mut w = word("x", 0.0, 0.0, 10.0);
                    (w.x0, w.x1) = (a, b);
                    w
                })
                .collect();
            let refs: Vec<&Word> = ws.iter().collect();
            for tol in [0usize, 1, 2, 5, 1000] {
                for gmin in [0.0, 7.0, 1e-6, 2.0] {
                    assert_eq!(
                        bits(&valleys(&refs, gmin, tol)),
                        bits(&valleys_quadratic(&refs, gmin, tol)),
                        "case {case} tol {tol} gmin {gmin}: {:?}",
                        ws.iter().map(|w| (w.x0, w.x1)).collect::<Vec<_>>()
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 10_000);
    }

    #[test]
    fn valleys_keep_up_on_a_hostile_page() {
        // 20 000 words of one block: the quadratic version needs seconds here.
        let ws: Vec<Word> = (0..20_000)
            .map(|i| {
                word(
                    "ab",
                    10.0 + 20.0 * (i % 200) as f64,
                    10.0 * (i / 200) as f64,
                    10.0,
                )
            })
            .collect();
        let refs: Vec<&Word> = ws.iter().collect();
        let t = std::time::Instant::now();
        let v = valleys(&refs, 1.0, 0);
        let ms = t.elapsed().as_secs_f64() * 1e3;
        assert_eq!(v.len(), 199);
        eprintln!("valleys 20000 words: {ms:.2} ms");
        if !cfg!(debug_assertions) {
            // Measured ~1 ms (gate 5 ms); loose for a loaded CI box.
            assert!(ms <= 50.0, "valleys took {ms} ms");
        }
    }

    #[test]
    fn finalize_drops_empty_and_rejects_toc_and_sparse() {
        let s = |v: &[&[&str]]| {
            v.iter()
                .map(|r| r.iter().map(|c| c.to_string()).collect())
                .collect::<Vec<Vec<String>>>()
        };
        assert_eq!(
            finalize(s(&[&["a", "", "b"], &["", "", ""], &["c", "", "d"]])),
            Some(s(&[&["a", "b"], &["c", "d"]]))
        );
        assert_eq!(
            finalize(s(&[&["Intro", "1"], &["Methods", "iv"], &["End", "12"]])),
            None,
            "toc"
        );
        assert_eq!(
            finalize(s(&[
                &["a", "", "", ""],
                &["", "", "", "b"],
                &["", "c", "", ""],
                &["", "", "d", ""]
            ])),
            None,
            "sparse"
        );
    }
}
