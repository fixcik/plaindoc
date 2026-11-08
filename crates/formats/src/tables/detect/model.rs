//! Layout-model regions (`M`, SP4-R1/R3): snapping to text lines, edge clusters, zebra rows.

use super::rules::{FILL_EDGE_TOL, FILL_ROW_TOL};
use super::text::text_grid;
use super::{CandidateKind, Raw};
use crate::tables::grid::Line;
use crate::tables::lattice::covered_len;
use crate::tables::prims::{PRect, Word};
use crate::tables::stats::median;
use crate::tables::{LimitExceeded, TableConfig};

/// SP4-R1 snap (region-spike `hybrid.py` `snap_region`): a word belongs to the region when the centre of its
/// text line lies inside the region's y range and its own centre inside the x range, both widened by this
/// many of its font sizes; the region becomes the box of those words.
const SNAP_TOL_FS: f64 = 0.5;
/// SP4-R3 (task brief): word edges within this many font sizes of a cluster's first edge are one cluster; a
/// cluster is a column bound when this share of the lines has an edge in it and no word crosses it. Used
/// instead of valleys when fewer than [`EDGE_MAX_VALLEYS`] valleys are found and the median line has at least
/// [`EDGE_MIN_WORDS`] words.
const EDGE_TOL_FS: f64 = 0.5;
const EDGE_LINE_SHARE: f64 = 0.6;
pub(super) const EDGE_MAX_VALLEYS: usize = 2;
pub(super) const EDGE_MIN_WORDS: f64 = 8.0;
/// SP4-R3: a band of fills (tops and bottoms within [`FILL_EDGE_TOL`]) bounds rows when it covers this share
/// of the block's width and is lower than this share of its height (not a table background); at least
/// [`FILL_ROWS_MIN_BANDS`] bands.
const FILL_ROWS_COVER: f64 = 0.9;
const FILL_ROWS_MAX_H: f64 = 0.5;
const FILL_ROWS_MIN_BANDS: usize = 2;
/// SP4-R3 zebra guard (review fix): the bands bound rows only when every unfilled gap between consecutive
/// bands is at most this many median band heights (alternating rows); a shaded header and total row with
/// several body rows between them fall back to text rows.
const FILL_ROWS_MAX_GAP_H: f64 = 1.5;

/// SP4-R3 column bounds of a model region from its lines: left edges and right edges of the words are
/// clustered separately (sorted, a cluster ends past [`EDGE_TOL_FS`] font sizes from its first edge); a
/// cluster supported by [`EDGE_LINE_SHARE`] of the lines gives a bound at its outer edge (the first left edge,
/// the last right edge) unless a word crosses it. A bound with no word centre between it and the previous
/// kept bound merges into it (the running midpoint of the pair, so a run of such bounds lands between them). O(n log n): sorts and binary searches over the words (the crossing test uses the running
/// maximum of `x1` over the words sorted by `x0`).
pub(super) fn edge_bounds(lines: &[Line], fs: f64) -> Vec<f64> {
    let tol = EDGE_TOL_FS * fs;
    let ws = || lines.iter().flat_map(|l| l.w.iter());
    let mut ext: Vec<(f64, f64)> = ws().map(|w| (w.x0, w.x1)).collect();
    ext.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut reach = Vec::with_capacity(ext.len());
    let mut m = f64::MIN;
    for e in &ext {
        m = m.max(e.1);
        reach.push(m);
    }
    let crossed = |c: f64| {
        let k = ext.partition_point(|e| e.0 < c);
        k > 0 && reach[k - 1] > c
    };
    let need = EDGE_LINE_SHARE * lines.len() as f64;
    let mut bounds = Vec::new();
    let mut seen = vec![usize::MAX; lines.len()];
    for right in [false, true] {
        let mut e: Vec<(f64, usize)> = lines
            .iter()
            .enumerate()
            .flat_map(|(i, l)| {
                l.w.iter()
                    .map(move |w| (if right { w.x1 } else { w.x0 }, i))
            })
            .collect();
        e.sort_by(|a, b| a.0.total_cmp(&b.0));
        seen.fill(usize::MAX);
        let mut k = 0;
        while k < e.len() {
            let (start, mut last, mut n) = (e[k].0, e[k].0, 0usize);
            let mut j = k;
            while j < e.len() && (j == k || e[j].0 - start <= tol) {
                if seen[e[j].1] != k {
                    seen[e[j].1] = k;
                    n += 1;
                }
                last = e[j].0;
                j += 1;
            }
            let b = if right { last } else { start };
            if n as f64 >= need && !crossed(b) {
                bounds.push(b);
            }
            k = j;
        }
    }
    bounds.sort_by(f64::total_cmp);
    let mut cx: Vec<f64> = ws().map(|w| w.cx).collect();
    cx.sort_by(f64::total_cmp);
    let mut out: Vec<f64> = Vec::with_capacity(bounds.len());
    for b in bounds {
        match out.last_mut() {
            Some(l) if cx.partition_point(|&c| c <= *l) == cx.partition_point(|&c| c < b) => {
                *l = (*l + b) / 2.0
            }
            _ => out.push(b),
        }
    }
    out
}

/// SP4-R3 row bounds of a model region's block `bx` (`[x0, top, x1, bottom]` of its words) from the filled
/// bands centred inside it, like the rulings of a rule group (zebra rows): `y0`, the band tops and bottoms
/// strictly inside `y0..y1`, `y1 + 1`, a bound within [`FILL_ROW_TOL`] of the previous one dropped; `None`
/// with fewer than [`FILL_ROWS_MIN_BANDS`] bands, when the bands are not zebra ([`FILL_ROWS_MAX_GAP_H`]) or
/// when they give more rows than the block has text lines (`nlines`; a hostile fill pattern cannot grow the
/// grid past the text rows). O(n log n) in the fills.
pub(super) fn fill_rows(
    fills: &[&PRect],
    bx: [f64; 4],
    y0: f64,
    y1: f64,
    nlines: usize,
) -> Option<Vec<f64>> {
    let (w, h) = (bx[2] - bx[0], bx[3] - bx[1]);
    let mut fr: Vec<&PRect> = fills
        .iter()
        .copied()
        .filter(|r| {
            let c = (r.t + r.b) / 2.0;
            bx[1] <= c && c <= bx[3] && r.x0 < bx[2] && bx[0] < r.x1
        })
        .collect();
    fr.sort_by(|a, b| a.t.total_cmp(&b.t).then(a.b.total_cmp(&b.b)));
    let mut bands: Vec<(f64, f64)> = Vec::new();
    let mut i = 0;
    while i < fr.len() {
        let mut j = i + 1;
        while j < fr.len()
            && (fr[j].t - fr[i].t).abs() <= FILL_EDGE_TOL
            && (fr[j].b - fr[i].b).abs() <= FILL_EDGE_TOL
        {
            j += 1;
        }
        let band = &fr[i..j];
        let cov = covered_len(
            band.iter()
                .map(|q| (q.x0.max(bx[0]), q.x1.min(bx[2])))
                .collect(),
            FILL_EDGE_TOL,
        );
        if cov >= FILL_ROWS_COVER * w && fr[i].b - fr[i].t < FILL_ROWS_MAX_H * h {
            bands.push((fr[i].t, fr[i].b));
        }
        i = j;
    }
    if bands.len() < FILL_ROWS_MIN_BANDS {
        return None;
    }
    let bh = median(&bands.iter().map(|b| b.1 - b.0).collect::<Vec<_>>());
    // Bands are in top order; a gap wider than the guard means rows between them are not alternating.
    if bands
        .windows(2)
        .any(|p| p[1].0 - p[0].1 > FILL_ROWS_MAX_GAP_H * bh)
    {
        return None;
    }
    let mut ys = vec![y0];
    ys.extend(
        bands
            .iter()
            .flat_map(|b| [b.0, b.1])
            .filter(|&y| y0 < y && y < y1),
    );
    ys.push(y1 + 1.0);
    ys.sort_by(f64::total_cmp);
    let rowb = drop_close_rows(ys);
    (rowb.len() - 1 <= nlines).then_some(rowb)
}

/// Sorted row bounds `ys` without those within [`FILL_ROW_TOL`] of the previous kept one.
pub(super) fn drop_close_rows(ys: Vec<f64>) -> Vec<f64> {
    let mut rowb: Vec<f64> = Vec::with_capacity(ys.len());
    for y in ys {
        if rowb.last().is_some_and(|&l| y - l <= FILL_ROW_TOL) {
            continue;
        }
        rowb.push(y);
    }
    rowb
}

/// Grids of the model regions of the page (SP4-R1), each snapped to the text lines inside it (`lines`: the
/// page's lines, built once); a region holding no line is dropped. A word goes to the first region that
/// takes it, so overlapping regions build at most one grid's worth of words in total: O(regions x words)
/// comparisons and one grid pass over the words.
pub(super) fn detect_model<'a>(
    lines: &[Line<'a>],
    regions: &[crate::tables::ModelRegion],
    fills: Option<&[&PRect]>,
    cfg: &TableConfig,
) -> Result<Vec<Raw<'a>>, LimitExceeded> {
    let mut out = Vec::new();
    let mut taken: Vec<Vec<bool>> = lines.iter().map(|l| vec![false; l.w.len()]).collect();
    for m in regions {
        let r = m.rect;
        let (x0, y0, x1, y1) = (r.x, r.top, r.x + r.w, r.top + r.h);
        let mut bl: Vec<Line<'a>> = Vec::new();
        for (l, tk) in lines.iter().zip(taken.iter_mut()) {
            let mut w = Vec::new();
            for (&x, t) in l.w.iter().zip(tk.iter_mut()) {
                let tol = SNAP_TOL_FS * x.fs;
                if !*t
                    && y0 - tol <= l.cy
                    && l.cy <= y1 + tol
                    && x0 - tol <= x.cx
                    && x.cx <= x1 + tol
                {
                    *t = true;
                    w.push(x);
                }
            }
            if !w.is_empty() {
                bl.push(Line { cy: l.cy, w });
            }
        }
        if bl.is_empty() {
            continue;
        }
        let fs = median(
            &bl.iter()
                .map(|l| median(&l.w.iter().map(|w| w.fs).collect::<Vec<_>>()))
                .collect::<Vec<_>>(),
        );
        let bw: Vec<&Word> = bl.iter().flat_map(|l| l.w.iter().copied()).collect();
        out.extend(text_grid(CandidateKind::Model, bw, &bl, fs, fills, cfg)?);
    }
    Ok(out)
}
