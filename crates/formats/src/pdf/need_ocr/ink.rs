//! Raster features at 50 dpi: glyph-like ink components outside the text layer
//! (port of ocr-need `pagefeat._ink_feats`; exact on the same gray image and boxes).

use super::InkFeatures;

/// numpy `percentile(g, 90)` (method "linear", including numpy's two-sided lerp) over u8 values.
fn p90(gray: &[u8]) -> f64 {
    if gray.is_empty() {
        return 0.0;
    }
    let mut hist = [0usize; 256];
    for &v in gray {
        hist[v as usize] += 1;
    }
    // k-th smallest value (0-based) from the histogram.
    let kth = |k: usize| {
        let mut acc = 0;
        for (v, &c) in hist.iter().enumerate() {
            acc += c;
            if acc > k {
                return v as f64;
            }
        }
        255.0
    };
    let pos = 0.9 * (gray.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let t = pos - lo as f64;
    let (a, b) = (kth(lo), kth((lo + 1).min(gray.len() - 1)));
    if t >= 0.5 {
        b - (b - a) * (1.0 - t)
    } else {
        a + (b - a) * t
    }
}

/// Component stats `(width, height, area)`, 8-connectivity.
fn components(mask: &[bool], w: usize, h: usize) -> Vec<(usize, usize, usize)> {
    let mut seen = vec![false; mask.len()];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for s in 0..mask.len() {
        if !mask[s] || seen[s] {
            continue;
        }
        seen[s] = true;
        stack.push(s);
        let (mut x0, mut y0, mut x1, mut y1, mut area) = (usize::MAX, usize::MAX, 0, 0, 0);
        while let Some(k) = stack.pop() {
            let (x, y) = (k % w, k / w);
            area += 1;
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    let nk = ny * w + nx;
                    if mask[nk] && !seen[nk] {
                        seen[nk] = true;
                        stack.push(nk);
                    }
                }
            }
        }
        out.push((x1 - x0 + 1, y1 - y0 + 1, area));
    }
    out
}

fn glyph_like(&(w, h, a): &(usize, usize, usize)) -> bool {
    (3..=25).contains(&h) && (2..=40).contains(&w) && (4..=500).contains(&a)
}

/// `gray` is `w x h` row-major; `boxes_pt` are text-layer boxes `[x0, y0, x1, y1]` in points with a
/// top-left origin on a page of `page_pt` points rendered at `dpi`.
pub(crate) fn ink_features(
    gray: &[u8],
    w: usize,
    h: usize,
    boxes_pt: &[[f32; 4]],
    page_pt: (f32, f32),
    dpi: f32,
) -> InkFeatures {
    if w == 0 || h == 0 || gray.len() != w * h {
        return InkFeatures::default();
    }
    let thr = (p90(gray) - 60.0).clamp(60.0, 200.0);
    let ink: Vec<bool> = gray.iter().map(|&v| f64::from(v) < thr).collect();
    let mut mask = vec![false; w * h];
    let sc = f64::from(dpi) / 72.0;
    // Python `int()` truncates; coordinates are clipped to the page, so they are >= 0.
    let px = |v: f32, add: i64| ((f64::from(v) * sc) as i64 + add).max(0) as usize;
    for b in boxes_pt {
        let (bx0, by0) = (b[0].max(0.0), b[1].max(0.0));
        let (bx1, by1) = (b[2].min(page_pt.0), b[3].min(page_pt.1));
        if bx1 <= bx0 || by1 <= by0 {
            continue;
        }
        let (x0, y0) = (px(bx0, -1), px(by0, -1));
        let (x1, y1) = (px(bx1, 2).min(w), px(by1, 2).min(h));
        for y in y0..y1 {
            mask[y * w + x0.min(x1)..y * w + x1].fill(true);
        }
    }
    let out: Vec<bool> = ink.iter().zip(&mask).map(|(&i, &m)| i && !m).collect();
    let cc_out = components(&out, w, h);
    let row_min = (w as f64 * 0.02).max(3.0);
    let rows: Vec<bool> = out
        .chunks_exact(w)
        .map(|r| r.iter().filter(|v| **v).count() as f64 > row_min)
        .collect();
    InkFeatures {
        glyph_cc_outside: cc_out.iter().filter(|c| glyph_like(c)).count() as u32,
        cc_outside: cc_out.len() as u32,
        ink_rows_outside: rows.windows(2).filter(|p| !p[0] && p[1]).count() as u32,
        glyph_cc_all: components(&ink, w, h)
            .iter()
            .filter(|c| glyph_like(c))
            .count() as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p90_matches_numpy_linear() {
        // numpy.percentile([0..=9], 90) == 8.1
        let v: Vec<u8> = (0..10).collect();
        assert!((p90(&v) - 8.1).abs() < 1e-5);
    }

    #[test]
    fn letters_outside_boxes_count_and_masked_ones_do_not() {
        let (w, h) = (100, 60);
        let mut g = vec![255u8; w * h];
        for k in 0..5 {
            for y in 10..18 {
                for x in 10 + k * 8..14 + k * 8 {
                    g[y * w + x] = 0;
                }
            }
        }
        let f = ink_features(&g, w, h, &[], (144.0, 86.4), 50.0);
        assert_eq!(
            (f.glyph_cc_outside, f.glyph_cc_all, f.ink_rows_outside),
            (5, 5, 1)
        );
        // Box covering the letters: x 7..53 px -> pt * 72 / 50.
        let b = [[7.0 * 1.44, 8.0 * 1.44, 53.0 * 1.44, 20.0 * 1.44]];
        let f = ink_features(&g, w, h, &b, (144.0, 86.4), 50.0);
        assert_eq!((f.glyph_cc_outside, f.glyph_cc_all), (0, 5));
    }
}
