//! Raster ruling lines (no ML).

/// Line extraction parameters. Lengths are given at 150 dpi and scale as `dpi / 150`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct LinesParams {
    pub dpi: f32,
    pub line_thr: f32,
    pub band_thr: f32,
    pub open_h_px: f32,
    pub open_v_px: f32,
    pub max_thick_px: f32,
    pub merge_gap_px: f32,
    pub merge_drift_px: f32,
    pub band_near_px: f32,
    pub band_far_px: f32,
}

impl Default for LinesParams {
    fn default() -> Self {
        Self {
            dpi: 150.0,
            line_thr: 0.80,
            band_thr: 0.35,
            open_h_px: 40.0,
            open_v_px: 30.0,
            max_thick_px: 9.0,
            merge_gap_px: 40.0,
            merge_drift_px: 4.0,
            band_near_px: 2.0,
            band_far_px: 4.0,
        }
    }
}

/// A ruling line in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub horizontal: bool,
    pub a: f32,
    pub b: f32,
    pub at: f32,
    pub width: f32,
}

/// Long horizontal and vertical rules of an aligned page, `norm` = `raster::normalize_bg` output (`w×h`, row-major).
/// Threshold, opening with long line kernels (kept runs), 8-connected components, thickness limit,
/// clean-band check on both sides (drops text lines merged into a run), then collinear merge of broken pieces.
/// Horizontal segments come first, each group in merge order. A `norm` that is not `w×h` long gives no lines.
pub fn extract_lines(norm: &[f32], w: u32, h: u32, p: &LinesParams) -> Vec<Segment> {
    let (w, h) = (w as usize, h as usize);
    if norm.len() != w * h {
        debug_assert!(false, "norm must hold w*h values");
        return Vec::new();
    }
    let s = p.dpi / 150.0;
    let ink: Vec<bool> = norm.iter().map(|&v| v < p.line_thr).collect();
    let cfg = Cfg {
        l: ((p.open_h_px * s) as usize).max(16),
        lv: ((p.open_v_px * s) as usize).max(12),
        // 8s+1 for the default 9 px: merged double lines blur into ~7 px.
        tmax: (p.max_thick_px - 1.0) * s + 1.0,
        d1: (p.band_near_px * s).round_ties_even() as isize,
        d2: (p.band_far_px * s).round_ties_even() as isize,
        band_thr: p.band_thr,
    };
    let page = Page { ink: &ink, w, h };
    let (gap, tol) = (p.merge_gap_px * s, p.merge_drift_px * s);
    let mut out = merge(
        components(&page, runs(&page, cfg.l, true), true, &cfg),
        gap,
        tol,
    );
    out.extend(merge(
        components(&page, runs(&page, cfg.lv, false), false, &cfg),
        gap,
        tol,
    ));
    out
}

struct Cfg {
    l: usize,
    lv: usize,
    tmax: f32,
    d1: isize,
    d2: isize,
    band_thr: f32,
}

struct Page<'a> {
    ink: &'a [bool],
    w: usize,
    h: usize,
}

/// Binary opening with an `L×1` (or `1×L`) rectangle: keeps the pixels of runs at least `len` long,
/// reproducing cv2's `morphologyEx(MORPH_OPEN)` used by the research (thresholds were calibrated on it).
/// cv2 anchors the kernel at `len / 2` and erodes and dilates with the same, unreflected kernel, so for an
/// even `len` the opened run `[s, e)` comes out shifted by +1 px along the line, `[s + 1, e + 1)` (clipped).
/// cv2's erode border is not emulated: there runs touching the image edge survive from about `len / 2` px and
/// a run touching the left/top edge is not shifted (no difference on the research pages).
fn runs(pg: &Page, len: usize, horizontal: bool) -> Vec<bool> {
    let (w, h) = (pg.w, pg.h);
    let mut out = vec![false; w * h];
    let shift = usize::from(len.is_multiple_of(2));
    let (lines, n) = if horizontal { (h, w) } else { (w, h) };
    let idx = |line: usize, i: usize| {
        if horizontal {
            line * w + i
        } else {
            i * w + line
        }
    };
    for line in 0..lines {
        let mut i = 0;
        while i < n {
            if !pg.ink[idx(line, i)] {
                i += 1;
                continue;
            }
            let start = i;
            while i < n && pg.ink[idx(line, i)] {
                i += 1;
            }
            if i - start >= len {
                (start + shift..(i + shift).min(n)).for_each(|j| out[idx(line, j)] = true);
            }
        }
    }
    out
}

/// 8-connected components of `mask` (consumed: visited pixels are cleared), filtered by thickness and the
/// clean-band check, as segments. Per component only its extent and the per-position `count`/`sum` along the
/// line are kept, no pixel list. A page of more than `u32::MAX` pixels gives no lines.
fn components(pg: &Page, mut mask: Vec<bool>, horizontal: bool, cfg: &Cfg) -> Vec<Segment> {
    let (w, h) = (pg.w, pg.h);
    if u32::try_from(w * h).is_err() {
        return Vec::new();
    }
    // Per position along the line: pixel count across it and the sum of their across coordinates
    // (page-wide scratch, zeroed again over each component's extent).
    let along_n = if horizontal { w } else { h };
    let mut count = vec![0u32; along_n];
    let mut sum = vec![0u64; along_n];
    let mut stack: Vec<u32> = Vec::new();
    let mut out = Vec::new();
    for start in 0..w * h {
        if !mask[start] {
            continue;
        }
        mask[start] = false;
        stack.push(start as u32);
        let (mut a0, mut a1, mut c0, mut c1) = (usize::MAX, 0, usize::MAX, 0);
        while let Some(i) = stack.pop() {
            let (x, y) = (i as usize % w, i as usize / w);
            // Along = coordinate along the line, across = perpendicular one.
            let (a, c) = if horizontal { (x, y) } else { (y, x) };
            (a0, a1, c0, c1) = (a0.min(a), a1.max(a), c0.min(c), c1.max(c));
            count[a] += 1;
            sum[a] += c as u64;
            for ny in y.saturating_sub(1)..(y + 2).min(h) {
                for nx in x.saturating_sub(1)..(x + 2).min(w) {
                    let j = ny * w + nx;
                    if mask[j] {
                        mask[j] = false;
                        stack.push(j as u32);
                    }
                }
            }
        }
        let ext = (a0, c0, c1 - c0 + 1);
        let (cnt, sm) = (&mut count[a0..=a1], &mut sum[a0..=a1]);
        if let Some(seg) = component_segment(pg, ext, cnt, sm, horizontal, cfg) {
            out.push(seg);
        }
        cnt.fill(0);
        sm.fill(0);
    }
    out
}

/// One component as a segment, or `None` if it is too thick or text touches it on too much of its length.
/// `(a0, c0, thick)`: first position along the line, first across and the bbox thickness; `count`/`sum` cover
/// the component's positions along the line from `a0`.
fn component_segment(
    pg: &Page,
    (a0, c0, thick): (usize, usize, usize),
    count: &[u32],
    sum: &[u64],
    horizontal: bool,
    cfg: &Cfg,
) -> Option<Segment> {
    let len = count.len();
    // Thickness measured per column (per row for verticals): the bbox grows with residual skew.
    if count.iter().copied().max().unwrap_or(0) as f32 > cfg.tmax {
        return None;
    }
    if !band_clear(pg, a0, count, sum, horizontal, cfg) {
        return None;
    }
    Some(Segment {
        horizontal,
        a: a0 as f32,
        b: (a0 + len) as f32,
        at: c0 as f32 + thick as f32 / 2.0,
        width: thick as f32,
    })
}

/// Bands `d1..=d2` px off both sides of the line are free of text, checked in pieces of `L` px with a local
/// line centre (robust to residual skew). Band rows with density >= 0.8 (a parallel line) are ignored.
fn band_clear(
    pg: &Page,
    a0: usize,
    count: &[u32],
    sum: &[u64],
    horizontal: bool,
    cfg: &Cfg,
) -> bool {
    let len = count.len();
    let n = (len / cfg.l).max(1);
    let (along_n, across_n) = if horizontal {
        (pg.w, pg.h)
    } else {
        (pg.h, pg.w)
    };
    let at = |along: usize, across: usize| {
        if horizontal {
            pg.ink[across * pg.w + along]
        } else {
            pg.ink[along * pg.w + across]
        }
    };
    // Mean density of the band lines `lo..hi` (across) over `p0..p1` (along), lines with density >= 0.8 skipped.
    let density = |lo: isize, hi: isize, p0: usize, p1: usize| -> f32 {
        let (lo, hi) = (
            lo.clamp(0, across_n as isize) as usize,
            hi.clamp(0, across_n as isize) as usize,
        );
        let p1 = p1.min(along_n);
        if lo >= hi || p0 >= p1 {
            return 0.0;
        }
        let (mut total, mut kept) = (0.0f32, 0usize);
        for c in lo..hi {
            let r = (p0..p1).filter(|&a| at(a, c)).count() as f32 / (p1 - p0) as f32;
            if r < 0.8 {
                total += r;
                kept += 1;
            }
        }
        if kept == 0 { 0.0 } else { total / kept as f32 }
    };
    let mut clear = 0usize;
    for c in 0..n {
        let (q0, q1) = (c * len / n, (c + 1) * len / n);
        let cnt: u64 = count[q0..q1].iter().map(|&v| u64::from(v)).sum();
        if cnt == 0 {
            continue;
        }
        let centre = sum[q0..q1].iter().sum::<u64>() as f32 / cnt as f32;
        let t = (count[q0..q1].iter().copied().max().unwrap_or(0) as f32)
            .round_ties_even()
            .max(1.0);
        let lo = (centre - t / 2.0) as isize;
        let hi = (centre + t / 2.0) as isize + 1;
        let da = density(
            (lo - cfg.d2).max(0),
            (lo - cfg.d1 + 1).max(0),
            a0 + q0,
            a0 + q1,
        );
        let db = density(hi + cfg.d1 - 1, hi + cfg.d2, a0 + q0, a0 + q1);
        if da.max(db) < cfg.band_thr {
            clear += 1;
        }
    }
    if horizontal && len >= 5 * cfg.l {
        // Long horizontal rules: text may touch them on part of the length.
        return clear as f32 >= 0.3 * n as f32;
    }
    if !horizontal && len >= 3 * cfg.lv {
        // Long vertical rules: text does not produce them, but may touch them.
        return true;
    }
    clear as f32 >= 0.6 * n as f32
}

/// Joins collinear pieces: sorted by `(round(at / tol), a)`, a piece continues the previous one if its line
/// drifts by at most `tol` and the gap is at most `gap`. The joined `at` is the running mean, `width` the first's.
fn merge(mut segs: Vec<Segment>, gap: f32, tol: f32) -> Vec<Segment> {
    let key = |s: &Segment| ((s.at / tol).round_ties_even() as i64, s.a);
    segs.sort_by(|x, y| {
        let (kx, ky) = (key(x), key(y));
        kx.0.cmp(&ky.0).then(kx.1.total_cmp(&ky.1))
    });
    let mut out: Vec<Segment> = Vec::with_capacity(segs.len());
    for q in segs {
        if let Some(o) = out.last_mut()
            && (o.at - q.at).abs() <= tol
            && q.a - o.b <= gap
        {
            o.b = o.b.max(q.b);
            o.at = (o.at + q.at) / 2.0;
        } else {
            out.push(q);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::normalize_bg;
    use image::GrayImage;

    fn grid(w: u32, h: u32, rows: &[u32], cols: &[u32], noise_text: bool) -> GrayImage {
        let mut g = GrayImage::from_pixel(w, h, image::Luma([245]));
        for &y in rows {
            for x in 50..w - 50 {
                for t in 0..2 {
                    g.put_pixel(x, y + t, image::Luma([30]));
                }
            }
        }
        for &x in cols {
            for y in rows[0]..*rows.last().unwrap() {
                for t in 0..2 {
                    g.put_pixel(x + t, y, image::Luma([30]));
                }
            }
        }
        if noise_text {
            // text-like dashes between the rules must not become lines
            for y in (rows[0] + 10..rows[1] - 10).step_by(12) {
                for x in (80..w - 80).step_by(9) {
                    for dx in 0..6 {
                        g.put_pixel(x + dx, y, image::Luma([40]));
                    }
                }
            }
        }
        g
    }

    #[test]
    fn lines_opening_shifts_even_kernels_like_cv2() {
        // One row: runs [10, 50) and [60, 100), both 40 px, the second touching the right edge.
        let w = 100;
        let mut ink = vec![false; w];
        ink[10..50].iter_mut().for_each(|v| *v = true);
        ink[60..100].iter_mut().for_each(|v| *v = true); // 40 px touching the edge
        let pg = Page { ink: &ink, w, h: 1 };
        let kept = |m: Vec<bool>| {
            m.iter()
                .enumerate()
                .filter(|(_, v)| **v)
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        };
        // Even kernel (40): shifted by +1, clipped at the edge.
        let even = kept(runs(&pg, 40, true));
        assert_eq!(even, (11..51).chain(61..100).collect::<Vec<_>>());
        // Odd kernel (39): no shift.
        assert_eq!(
            kept(runs(&pg, 39, true)),
            (10..50).chain(60..100).collect::<Vec<_>>()
        );
        // Shorter than the kernel: dropped.
        assert!(kept(runs(&pg, 41, true)).is_empty());
        // Columns shift the same way (down).
        let pg = Page {
            ink: &ink,
            w: 1,
            h: w,
        };
        assert_eq!(kept(runs(&pg, 40, false)), even);
    }

    #[test]
    fn lined_grid_yields_exact_segments() {
        let g = grid(
            1240,
            900,
            &[100, 200, 300, 400],
            &[60, 400, 800, 1180],
            true,
        );
        let norm = normalize_bg(&g, 31);
        let s = extract_lines(&norm, 1240, 900, &LinesParams::default());
        let h: Vec<_> = s.iter().filter(|s| s.horizontal).collect();
        let v: Vec<_> = s.iter().filter(|s| !s.horizontal).collect();
        assert_eq!((h.len(), v.len()), (4, 4), "{s:?}");
        assert!(h.iter().all(|s| (s.b - s.a) > 1000.0 && s.width <= 9.0));
    }

    #[test]
    fn broken_line_is_merged_across_small_gaps() {
        let mut g = grid(1240, 400, &[100, 300], &[], false);
        for x in 600..630 {
            for t in 0..2 {
                g.put_pixel(x, 100 + t, image::Luma([245]));
            }
        } // 30 px gap < 40
        let s = extract_lines(&normalize_bg(&g, 31), 1240, 400, &LinesParams::default());
        assert_eq!(s.iter().filter(|s| s.horizontal).count(), 2);
    }
}
