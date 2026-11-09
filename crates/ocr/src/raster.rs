//! Raster helpers on pages (no ML): rotation and skew estimate, background normalisation and the separable
//! filters behind it.

use image::GrayImage;

use crate::Image;

/// Long side of the downscaled copy the skew is measured on.
const SKEW_SIDE: u32 = 600;
/// Skew grid: `-SKEW_STEPS..=SKEW_STEPS` steps of `SKEW_STEP_DEG` (±3°).
const SKEW_STEPS: i32 = 30;
const SKEW_STEP_DEG: f32 = 0.1;
/// Shift (px, both axes) added after the trial rotation, see `estimate_skew`.
const SKEW_SHIFT_PX: f64 = 0.25;

/// Grayscale copy of an RGB page.
pub fn gray(img: &Image) -> GrayImage {
    image::imageops::grayscale(img)
}

/// Rotates `img` by `deg` around `(w/2, h/2)`, size kept, bicubic (Keys a = -0.75, as cv2 `INTER_CUBIC`, the
/// research deskew); taps outside the image take `fill` (cv2 `BORDER_CONSTANT`).
/// cv2 convention (`getRotationMatrix2D(c, deg, 1)` + `warpAffine`, y down, `deg > 0` turns the content
/// counter-clockwise on screen): a source point lands at
/// `x' = cx + (x − cx)·cos φ + (y − cy)·sin φ`, `y' = cy − (x − cx)·sin φ + (y − cy)·cos φ`.
pub fn rotate(img: &Image, deg: f32, fill: [u8; 3]) -> Image {
    if deg == 0.0 {
        return img.clone();
    }
    let (w, h) = img.dimensions();
    let (wu, hu) = (w as usize, h as usize);
    let src = img.as_raw();
    let mut out = vec![0u8; src.len()];
    warp_at(wu, hu, deg, 0.0, |i, x, y| {
        let mut v = [0f32; 3];
        if let Some((first, wx, wy)) = cubic_inside(x, y, wu, hu) {
            // All 16 taps inside: the same weights and summation order as `cubic_taps`.
            for (j, wy) in wy.iter().enumerate() {
                let row = 3 * (first + j * wu);
                for (k, wx) in wx.iter().enumerate() {
                    let (wt, s) = (wx * wy, row + 3 * k);
                    for c in 0..3 {
                        v[c] += wt * f32::from(src[s + c]);
                    }
                }
            }
        } else {
            for (j, wt) in cubic_taps(x, y, wu, hu) {
                let px = j.map_or(fill, |j| [src[3 * j], src[3 * j + 1], src[3 * j + 2]]);
                for c in 0..3 {
                    v[c] += wt * f32::from(px[c]);
                }
            }
        }
        for c in 0..3 {
            out[3 * i + c] = v[c].round().clamp(0.0, 255.0) as u8;
        }
    });
    Image::from_raw(w, h, out).expect("buffer size matches the image")
}

/// Gray variant of [`rotate`] (same convention and interpolation).
pub fn rotate_gray(g: &GrayImage, deg: f32, fill: u8) -> GrayImage {
    if deg == 0.0 {
        return g.clone();
    }
    let (w, h) = g.dimensions();
    let src = g.as_raw();
    let mut out = vec![0u8; src.len()];
    warp(w as usize, h as usize, deg, 0.0, cubic_taps, |i, taps| {
        let v: f32 = taps
            .iter()
            .map(|&(j, wt)| wt * f32::from(j.map_or(fill, |j| src[j])))
            .sum();
        out[i] = v.round().clamp(0.0, 255.0) as u8;
    });
    GrayImage::from_raw(w, h, out).expect("buffer size matches the image")
}

/// Skew of the text lines in degrees, as the correction angle: `rotate(img, estimate_skew(..))` levels the
/// lines (sign as `cv2.getRotationMatrix2D`). Port of `ocr_page.py::estimate_skew`: ink of a copy
/// downscaled to 600 px long side (gray below 0.8 of its 9×9 closing), turned by -3..3° in 0.1° steps,
/// scored by the sharpness of the row profiles of three vertical strips. A page without ink gives 0.
/// The 0.1° grid is searched coarse to fine: every `SKEW_COARSE`-th step, then the steps within
/// `SKEW_REFINE` of the two best coarse ones (at most 29 trial rotations instead of 61). Near 0° the
/// score has narrow side peaks (sampling phase, e.g. ±0.3° above 0° on clean pages), so refining a
/// single 0.3° coarse winner picked 0.3° instead of -0.1° on 171 of 904 evaluation pages; this search
/// returns the full scan's angle on all 904 pages and the synthetic sweep (either margin alone suffices
/// there: radius 2 with two winners, radius 4 with one).
pub fn estimate_skew(g: &GrayImage) -> f32 {
    let Some(mut s) = SkewScore::new(g) else {
        return 0.0;
    };
    let mut coarse: Vec<Trial> = (-SKEW_STEPS..=SKEW_STEPS)
        .step_by(SKEW_COARSE as usize)
        .map(|i| Trial::at(s.score(i), i))
        .collect();
    coarse.sort_by(|a, b| b.cmp(a));
    let mut fine: Vec<i32> = coarse
        .iter()
        .take(2)
        .flat_map(|c| c.step - SKEW_REFINE..=c.step + SKEW_REFINE)
        .filter(|i| i.abs() <= SKEW_STEPS && i % SKEW_COARSE != 0)
        .collect();
    fine.sort_unstable();
    fine.dedup();
    let best = fine
        .into_iter()
        .map(|i| Trial::at(s.score(i), i))
        .fold(coarse[0], Trial::max);
    best.step as f32 * SKEW_STEP_DEG
}

/// Coarse step and refine radius of the skew search, in `SKEW_STEP_DEG` steps (0.5° and ±0.4°);
/// `SKEW_COARSE` divides `SKEW_STEPS`.
const SKEW_COARSE: i32 = 5;
const SKEW_REFINE: i32 = 4;
const _: () = assert!(SKEW_STEPS % SKEW_COARSE == 0);

/// A scored skew step. The order is total (higher score, then smaller |step|, then the lower step), so
/// the maximum does not depend on the order the steps are tried in; it equals the research's ascending
/// scan that keeps the first of equal scores unless a later one has a smaller |angle|.
#[derive(Clone, Copy, Debug)]
struct Trial {
    score: f64,
    step: i32,
}

impl Trial {
    fn at(score: f64, step: i32) -> Self {
        Self { score, step }
    }
}

impl Ord for Trial {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.score
            .total_cmp(&o.score)
            .then((o.step.abs(), o.step).cmp(&(self.step.abs(), self.step)))
    }
}
impl PartialOrd for Trial {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl PartialEq for Trial {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o).is_eq()
    }
}
impl Eq for Trial {}

/// Ink of the downscaled page and the buffers that score one trial rotation (see [`estimate_skew`]).
struct SkewScore {
    ink: Vec<f32>,
    strip: Vec<usize>,
    ww: usize,
    hh: usize,
    prof: Vec<f64>,
}

impl SkewScore {
    fn new(g: &GrayImage) -> Option<Self> {
        let (w, h) = g.dimensions();
        if w == 0 || h == 0 {
            return None;
        }
        let long = w.max(h);
        let small = if long > SKEW_SIDE {
            let s = f64::from(SKEW_SIDE) / f64::from(long);
            let dim = |v: u32| ((f64::from(v) * s) as u32).max(1);
            area_resize(g, dim(w), dim(h))
        } else {
            g.clone()
        };
        let bg = close_gray(&small, 9);
        let ink: Vec<f32> = small
            .as_raw()
            .iter()
            .zip(bg.as_raw())
            .map(|(&v, &b)| {
                if f32::from(v) / f32::from(b).max(1.0) < 0.8 {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        let (ww, hh) = (small.width() as usize, small.height() as usize);
        // Strip k holds columns `k*ww/3 .. (k+1)*ww/3`.
        let strip: Vec<usize> = (0..ww)
            .map(|x| (1..3).filter(|&k| x >= k * ww / 3).count())
            .collect();
        Some(Self {
            ink,
            strip,
            ww,
            hh,
            prof: vec![0f64; 3 * hh],
        })
    }

    /// Profile sharpness after turning the ink by `step · SKEW_STEP_DEG`.
    fn score(&mut self, step: i32) -> f64 {
        let a = step as f32 * SKEW_STEP_DEG;
        let Self {
            ink,
            strip,
            ww,
            hh,
            prof,
        } = self;
        let (ww, hh) = (*ww, *hh);
        prof.iter_mut().for_each(|p| *p = 0.0);
        // Sub-pixel shift: without it a = 0 (no interpolation blur) always wins. The research's 0.5 px
        // over-corrects (every row splits 50/50 at a = 0 only, so small angles win: 0.2° on a level
        // page); a quarter pixel blurs a = 0 about as much as the phases a tilted row sweeps.
        warp_at(ww, hh, a, SKEW_SHIFT_PX, |j, x, y| {
            // All 4 taps inside: the same products and summation order as `taps`.
            let v: f32 = if let Some(t) = bilinear_inside(x, y, ww, hh) {
                t.iter().map(|&(s, wt)| ink[s] * wt).sum()
            } else {
                taps(x, y, ww, hh)
                    .iter()
                    .map(|&(s, wt)| s.map_or(0.0, |s| ink[s]) * wt)
                    .sum()
            };
            prof[strip[j % ww] * hh + j / ww] += f64::from(v);
        });
        prof.chunks_exact(hh)
            .map(|p| p.windows(2).map(|d| (d[1] - d[0]).powi(2)).sum::<f64>())
            .sum()
    }
}

/// Source pixels (index, `None` outside the image) and weights of one interpolated sample.
type Taps<const N: usize> = [(Option<usize>, f32); N];

/// Inverse mapping of a cv2 rotation by `deg` about `(w/2, h/2)` followed by a shift of `shift` px:
/// calls `f(dst_index, taps)` for every destination pixel, `taps` = `sample` source pixels (index when
/// inside the image) and weights. Pixel centres sit at integer coordinates, as in cv2.
fn warp<const N: usize>(
    w: usize,
    h: usize,
    deg: f32,
    shift: f64,
    sample: fn(f64, f64, usize, usize) -> Taps<N>,
    mut f: impl FnMut(usize, Taps<N>),
) {
    warp_at(w, h, deg, shift, |i, x, y| f(i, sample(x, y, w, h)));
}

/// As [`warp`], but calls `f(dst_index, x, y)` with the source point itself.
fn warp_at(w: usize, h: usize, deg: f32, shift: f64, mut f: impl FnMut(usize, f64, f64)) {
    let (s, c) = f64::from(deg).to_radians().sin_cos();
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let u0 = -shift - cx;
    for y in 0..h {
        let v = y as f64 - shift - cy;
        let (mut sx, mut sy) = (cx + u0 * c - v * s, cy + u0 * s + v * c);
        for x in 0..w {
            f(y * w + x, sx, sy);
            sx += c;
            sy += s;
        }
    }
}

/// The taps of [`taps`] at `(x, y)` when all 4 lie inside the image (same indices and weights).
fn bilinear_inside(x: f64, y: f64, w: usize, h: usize) -> Option<[(usize, f32); 4]> {
    let (x0, y0) = (x.floor(), y.floor());
    // NaN fails every comparison; infinities fail the upper bounds.
    let inside = x0 >= 0.0 && y0 >= 0.0 && x0 + 1.0 < w as f64 && y0 + 1.0 < h as f64;
    inside.then(|| {
        let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
        let k = y0 as usize * w + x0 as usize;
        [
            (k, (1.0 - fx) * (1.0 - fy)),
            (k + 1, fx * (1.0 - fy)),
            (k + w, (1.0 - fx) * fy),
            (k + w + 1, fx * fy),
        ]
    })
}

/// When all 4×4 taps of [`cubic_taps`] at `(x, y)` lie inside the image: the index of the top-left
/// tap and the Keys weights along x and y (tap `4 * j + i` weighs `wx[i] * wy[j]`).
fn cubic_inside(x: f64, y: f64, w: usize, h: usize) -> Option<(usize, [f32; 4], [f32; 4])> {
    let (x0, y0) = (x.floor(), y.floor());
    // NaN fails every comparison; infinities fail the upper bounds.
    let inside = x0 >= 1.0 && y0 >= 1.0 && x0 + 2.0 < w as f64 && y0 + 2.0 < h as f64;
    inside.then(|| {
        let first = (y0 as usize - 1) * w + x0 as usize - 1;
        (first, keys((x - x0) as f32), keys((y - y0) as f32))
    })
}

/// Bilinear taps at `(x, y)`: the 4 neighbouring pixels (index, `None` outside) and their weights.
fn taps(x: f64, y: f64, w: usize, h: usize) -> Taps<4> {
    if !(x.is_finite() && y.is_finite()) {
        return [(None, 1.0), (None, 0.0), (None, 0.0), (None, 0.0)];
    }
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
    let at = |dx: f64, dy: f64| {
        let (xx, yy) = (x0 + dx, y0 + dy);
        (xx >= 0.0 && yy >= 0.0 && xx < w as f64 && yy < h as f64)
            .then(|| yy as usize * w + xx as usize)
    };
    [
        (at(0.0, 0.0), (1.0 - fx) * (1.0 - fy)),
        (at(1.0, 0.0), fx * (1.0 - fy)),
        (at(0.0, 1.0), (1.0 - fx) * fy),
        (at(1.0, 1.0), fx * fy),
    ]
}

/// Bicubic taps at `(x, y)`: the 4×4 neighbouring pixels (index, `None` outside) and their Keys weights
/// (a = -0.75, cv2 `INTER_CUBIC`).
fn cubic_taps(x: f64, y: f64, w: usize, h: usize) -> Taps<16> {
    let mut out = [(None, 0.0); 16];
    if !(x.is_finite() && y.is_finite()) {
        out[0].1 = 1.0;
        return out;
    }
    let (x0, y0) = (x.floor(), y.floor());
    let (wx, wy) = (keys((x - x0) as f32), keys((y - y0) as f32));
    // Far outside: keep the coordinates in a range where the integer casts are exact.
    let (xi, yi) = (
        x0.clamp(-4.0, w as f64 + 4.0) as isize,
        y0.clamp(-4.0, h as f64 + 4.0) as isize,
    );
    let cols = [-1, 0, 1, 2]
        .map(|d| xi + d)
        .map(|c| (c >= 0 && (c as usize) < w).then_some(c as usize));
    for (j, wy) in wy.iter().enumerate() {
        let r = yi + j as isize - 1;
        let row = (r >= 0 && (r as usize) < h).then(|| r as usize * w);
        for (i, wx) in wx.iter().enumerate() {
            out[4 * j + i] = (row.zip(cols[i]).map(|(r, c)| r + c), wx * wy);
        }
    }
    out
}

/// Keys cubic weights (a = -0.75) of the taps at offsets -1, 0, 1, 2 for a fraction `t` in `0..1`.
fn keys(t: f32) -> [f32; 4] {
    const A: f32 = -0.75;
    let near = |d: f32| ((A + 2.0) * d - (A + 3.0)) * d * d + 1.0;
    let far = |d: f32| ((A * d - 5.0 * A) * d + 8.0 * A) * d - 4.0 * A;
    [far(1.0 + t), near(t), near(1.0 - t), far(2.0 - t)]
}

/// Area-average downscale to `dw×dh` (cv2 `INTER_AREA` when shrinking).
fn area_resize(g: &GrayImage, dw: u32, dh: u32) -> GrayImage {
    let (w, h) = (g.width() as usize, g.height() as usize);
    let (dw, dh) = (dw as usize, dh as usize);
    let (wx, wy) = (area_weights(w, dw), area_weights(h, dh));
    let src = g.as_raw();
    let mut rows = vec![0f32; h * dw];
    for (row, out) in src.chunks_exact(w).zip(rows.chunks_exact_mut(dw)) {
        for (o, ws) in out.iter_mut().zip(&wx) {
            *o = ws.iter().map(|&(i, wt)| wt * f32::from(row[i])).sum();
        }
    }
    let mut out = vec![0u8; dw * dh];
    let mut acc = vec![0f32; dw];
    for (o, ws) in out.chunks_exact_mut(dw).zip(&wy) {
        acc.iter_mut().for_each(|a| *a = 0.0);
        for &(i, wt) in ws {
            for (a, &v) in acc.iter_mut().zip(&rows[i * dw..(i + 1) * dw]) {
                *a += wt * v;
            }
        }
        for (o, &a) in o.iter_mut().zip(&acc) {
            *o = a.round().clamp(0.0, 255.0) as u8;
        }
    }
    GrayImage::from_raw(dw as u32, dh as u32, out).expect("buffer size matches the image")
}

/// For each of `n` output cells over `len` input cells: covered input cells and weights (sum 1).
fn area_weights(len: usize, n: usize) -> Vec<Vec<(usize, f32)>> {
    let scale = len as f64 / n as f64;
    (0..n)
        .map(|d| {
            let (a, b) = (d as f64 * scale, (d + 1) as f64 * scale);
            (a.floor() as usize..(b.ceil() as usize).min(len))
                .filter_map(|i| {
                    let cover = b.min(i as f64 + 1.0) - a.max(i as f64);
                    (cover > 0.0).then_some((i, (cover / scale) as f32))
                })
                .collect()
        })
        .collect()
}

/// Grayscale closing (dilation, then erosion) with a `k×k` rectangle, centred at `k/2`.
/// Pixels outside the image are ignored, as with cv2's default morphology border.
/// Separable van Herk/Gil-Werman max/min filters: O(1) per pixel whatever `k` is.
pub fn close_gray(g: &GrayImage, k: u32) -> GrayImage {
    let (w, h) = (g.width() as usize, g.height() as usize);
    let k = k.max(1) as usize;
    if w == 0 || h == 0 {
        return g.clone();
    }
    let mut buf = g.as_raw().clone();
    for row in buf.chunks_exact_mut(w) {
        sliding_extremum(row, k, u8::max, 0);
    }
    let buf = cols_extremum(&buf, w, k, u8::max, 0);
    let mut buf = cols_extremum(&buf, w, k, u8::min, 255);
    for row in buf.chunks_exact_mut(w) {
        sliding_extremum(row, k, u8::min, 255);
    }
    GrayImage::from_raw(w as u32, h as u32, buf).expect("buffer size matches the image")
}

/// `k×k` mean filter with reflect-101 borders, rounded back to u8 (cv2 `blur` on a u8 image; parity with cv2
/// checked for odd `k`, ties may round differently for even `k`).
pub fn box_blur(g: &GrayImage, k: u32) -> GrayImage {
    let (w, h) = (g.width() as usize, g.height() as usize);
    let k = k.max(1) as usize;
    if w == 0 || h == 0 {
        return g.clone();
    }
    let mut sums: Vec<u32> = g.as_raw().iter().map(|&v| u32::from(v)).collect();
    for row in sums.chunks_exact_mut(w) {
        sliding_sum(row, k);
    }
    let area = (k * k) as f32;
    let buf = cols_sum(&sums, w, k, |s| {
        (s as f32 / area).round().clamp(0.0, 255.0) as u8
    });
    GrayImage::from_raw(w as u32, h as u32, buf).expect("buffer size matches the image")
}

/// Uneven-lighting normalisation: `gray / blur(close(gray))`, both with a `k×k` window, clipped to 0..1.5.
/// Background comes out near 1, ink well below it. Row-major, `width × height` values.
pub fn normalize_bg(g: &GrayImage, k: u32) -> Vec<f32> {
    let bg = box_blur(&close_gray(g, k), k);
    g.as_raw()
        .iter()
        .zip(bg.as_raw())
        .map(|(&v, &b)| (f32::from(v) / f32::from(b).max(1.0)).clamp(0.0, 1.5))
        .collect()
}

/// Reflect-101 index into `0..n` (`-1 -> 1`, `n -> n - 2`).
fn reflect101(mut j: isize, n: isize) -> usize {
    if n == 1 {
        return 0;
    }
    loop {
        if j < 0 {
            j = -j;
        } else if j >= n {
            j = 2 * n - 2 - j;
        } else {
            return j as usize;
        }
    }
}

/// In place `v[i] = op(v[i - k/2 ..= i - k/2 + k - 1])`, out-of-range samples replaced by `neutral`
/// (van Herk/Gil-Werman: block prefix and suffix extrema of size `k`).
fn sliding_extremum(v: &mut [u8], k: usize, op: impl Fn(u8, u8) -> u8, neutral: u8) {
    let n = v.len();
    let r = k / 2;
    let mut pre = vec![neutral; n + k - 1];
    pre[r..r + n].copy_from_slice(v);
    let mut suf = pre.clone();
    for block in pre.chunks_mut(k) {
        for j in 1..block.len() {
            block[j] = op(block[j - 1], block[j]);
        }
    }
    for block in suf.chunks_mut(k) {
        for j in (0..block.len().saturating_sub(1)).rev() {
            block[j] = op(block[j + 1], block[j]);
        }
    }
    for (i, d) in v.iter_mut().enumerate() {
        *d = op(suf[i], pre[i + k - 1]);
    }
}

/// `sliding_extremum` down the columns of a row-major buffer `w` wide, done on whole rows at a time
/// (element-wise over the row, so the compiler vectorises it).
fn cols_extremum(
    src: &[u8],
    w: usize,
    k: usize,
    op: impl Fn(u8, u8) -> u8,
    neutral: u8,
) -> Vec<u8> {
    let h = src.len() / w;
    let (r, len) = (k / 2, h + k - 1);
    let mut pre = vec![neutral; len * w];
    pre[r * w..(r + h) * w].copy_from_slice(src);
    let mut suf = pre.clone();
    for j in (0..len).filter(|j| j % k != 0) {
        let (done, rest) = pre.split_at_mut(j * w);
        for (c, &p) in rest[..w].iter_mut().zip(&done[(j - 1) * w..]) {
            *c = op(p, *c);
        }
    }
    for j in (0..len - 1).rev().filter(|j| (j + 1) % k != 0) {
        let (cur, done) = suf.split_at_mut((j + 1) * w);
        for (c, &p) in cur[j * w..].iter_mut().zip(&done[..w]) {
            *c = op(p, *c);
        }
    }
    let mut out = vec![0; h * w];
    for (i, row) in out.chunks_exact_mut(w).enumerate() {
        let (s, p) = (&suf[i * w..(i + 1) * w], &pre[(i + k - 1) * w..(i + k) * w]);
        for ((o, &a), &b) in row.iter_mut().zip(s).zip(p) {
            *o = op(a, b);
        }
    }
    out
}

/// In place `v[i] = sum(v[reflect101(j)])` for `j` in `i - k/2 ..= i - k/2 + k - 1` (running sum).
fn sliding_sum(v: &mut [u32], k: usize) {
    let n = v.len() as isize;
    let r = (k / 2) as isize;
    // Reflected samples once (`n + k` covers the last window and its trailing subtraction).
    let p: Vec<u32> = (-r..n - r + k as isize)
        .map(|j| v[reflect101(j, n)])
        .collect();
    let mut sum: u32 = p[..k].iter().sum();
    for (i, d) in v.iter_mut().enumerate() {
        *d = sum;
        sum = sum + p[i + k] - p[i];
    }
}

/// `sliding_sum` down the columns of a row-major buffer `w` wide, on whole rows at a time, each window sum
/// mapped through `f` as it is produced (no full-size sum buffer).
fn cols_sum(src: &[u32], w: usize, k: usize, f: impl Fn(u32) -> u8) -> Vec<u8> {
    let h = (src.len() / w) as isize;
    let r = (k / 2) as isize;
    let row = |j: isize| {
        let y = reflect101(j, h);
        &src[y * w..(y + 1) * w]
    };
    let mut sum = vec![0u32; w];
    for j in -r..k as isize - r {
        sum.iter_mut().zip(row(j)).for_each(|(s, &v)| *s += v);
    }
    let mut out = vec![0; src.len()];
    for (i, o) in out.chunks_exact_mut(w).enumerate() {
        o.iter_mut().zip(&sum).for_each(|(o, &s)| *o = f(s));
        let (add, sub) = (row(i as isize - r + k as isize), row(i as isize - r));
        for ((s, &a), &b) in sum.iter_mut().zip(add).zip(sub) {
            *s = *s + a - b;
        }
    }
    out
}

#[cfg(test)]
#[path = "raster_test.rs"]
pub(crate) mod tests;
