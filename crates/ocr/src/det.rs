//! PP-OCRv5 det pre/post-processing (DB). Pure functions, no ONNX.

use crate::{Image, TextBox};

/// DB post-processing thresholds (PaddleX OCR.yaml defaults).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DbParams {
    pub thresh: f32,
    pub box_thresh: f32,
    pub unclip: f32,
    /// Boxes with a shorter side below this (input pixels) are dropped.
    pub min_side: f32,
}

impl Default for DbParams {
    fn default() -> Self {
        Self {
            thresh: 0.3,
            box_thresh: 0.6,
            unclip: 1.5,
            min_side: 5.0,
        }
    }
}

/// Detector input size: long side capped at `limit` (0 = no downscale, hard cap 4000),
/// both sides rounded to a multiple of 32 (min 32).
pub fn det_input_size(w: u32, h: u32, limit: u32) -> (u32, u32) {
    let lim = if limit == 0 { 4000 } else { limit.min(4000) };
    let long = w.max(h).max(1);
    let ratio = if long > lim {
        lim as f32 / long as f32
    } else {
        1.0
    };
    let r32 = |v: u32| ((((v as f32 * ratio) / 32.0).round() as u32).max(1)) * 32;
    (r32(w), r32(h))
}

/// NCHW f32 tensor data: BGR order, ImageNet mean/std, as in the model's inference.yml.
pub fn det_tensor(img: &Image, rw: u32, rh: u32) -> Vec<f32> {
    let r = image::imageops::resize(img, rw, rh, image::imageops::FilterType::Triangle);
    let plane = (rw * rh) as usize;
    let mean = [0.485f32, 0.456, 0.406];
    let std = [0.229f32, 0.224, 0.225];
    let mut data = vec![0f32; 3 * plane];
    for (i, p) in r.pixels().enumerate() {
        let bgr = [p[2], p[1], p[0]];
        for c in 0..3 {
            data[c * plane + i] = (bgr[c] as f32 / 255.0 - mean[c]) / std[c];
        }
    }
    data
}

/// DB post-process of a `rw x rh` probability map into oriented boxes in original pixels
/// (`sx = orig_w / rw`, `sy = orig_h / rh`). Components: 8-connectivity over `prob > thresh`,
/// score = mean prob of the component, oriented rectangle via PCA (approximates minAreaRect),
/// unclip by `area * unclip / perimeter`. Order: raster order of the component's first pixel.
pub fn db_boxes(
    prob: &[f32],
    rw: usize,
    rh: usize,
    sx: f32,
    sy: f32,
    orig: (u32, u32),
    p: &DbParams,
) -> Vec<TextBox> {
    // Malformed model output or zero-sized original: no boxes, never panic.
    if prob.len() != rw * rh || orig.0 == 0 || orig.1 == 0 {
        return Vec::new();
    }
    let mut lab = vec![false; rw * rh];
    let mut boxes = Vec::new();
    let mut stack = Vec::new();
    let mut pix: Vec<usize> = Vec::new();
    for start in 0..rw * rh {
        // NaN never starts a component (same predicate as the neighbour test below).
        if prob[start].partial_cmp(&p.thresh) != Some(std::cmp::Ordering::Greater) || lab[start] {
            continue;
        }
        lab[start] = true;
        stack.push(start);
        pix.clear();
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        let mut psum = 0f32;
        while let Some(k) = stack.pop() {
            pix.push(k);
            psum += prob[k];
            let (x, y) = (k % rw, k / rw);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            for ny in y.saturating_sub(1)..=(y + 1).min(rh - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(rw - 1) {
                    let nk = ny * rw + nx;
                    if !lab[nk] && prob[nk] > p.thresh {
                        lab[nk] = true;
                        stack.push(nk);
                    }
                }
            }
        }
        let (bw, bh) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
        let score = psum / pix.len() as f32;
        if bw.min(bh) < 3.0 || score < p.box_thresh {
            continue;
        }
        if let Some(quad) = oriented_box(&pix, rw, bw, bh, sx, sy, orig, p) {
            boxes.push(TextBox { quad, score });
        }
    }
    boxes
}

#[allow(clippy::too_many_arguments)]
fn oriented_box(
    pix: &[usize],
    rw: usize,
    bw: f32,
    bh: f32,
    sx: f32,
    sy: f32,
    orig: (u32, u32),
    p: &DbParams,
) -> Option<[[f32; 2]; 4]> {
    let n = pix.len() as f32;
    let at = |k: usize| ((k % rw) as f32 + 0.5, (k / rw) as f32 + 0.5);
    let (mut mx, mut my) = (0f32, 0f32);
    for &k in pix {
        let (x, y) = at(k);
        mx += x;
        my += y;
    }
    mx /= n;
    my /= n;
    let (mut cxx, mut cyy, mut cxy) = (0f32, 0f32, 0f32);
    for &k in pix {
        let (x, y) = at(k);
        let (dx, dy) = (x - mx, y - my);
        cxx += dx * dx;
        cyy += dy * dy;
        cxy += dx * dy;
    }
    let tr = cxx + cyy;
    let disc = (tr * tr / 4.0 - (cxx * cyy - cxy * cxy)).max(0.0).sqrt();
    let (l1, l2) = (tr / 2.0 + disc, (tr / 2.0 - disc).max(1e-6));
    let mut th = 0.5 * (2.0 * cxy).atan2(cxx - cyy);
    if l1 / l2 < 4.0 {
        // Nearly square blob: PCA angle is noise, use the bbox shape.
        th = if bh > bw * 1.5 {
            std::f32::consts::FRAC_PI_2
        } else {
            0.0
        };
    }
    let (ux, uy) = (th.cos(), th.sin());
    let (vx, vy) = (-uy, ux);
    let (mut umin, mut umax, mut vmin, mut vmax) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for &k in pix {
        let (x, y) = at(k);
        let (pu, pv) = ((x - mx) * ux + (y - my) * uy, (x - mx) * vx + (y - my) * vy);
        umin = umin.min(pu - 0.5);
        umax = umax.max(pu + 0.5);
        vmin = vmin.min(pv - 0.5);
        vmax = vmax.max(pv + 0.5);
    }
    let (lu, lv) = (umax - umin, vmax - vmin);
    let d = lu * lv * p.unclip / (2.0 * (lu + lv));
    let (umin, umax, vmin, vmax) = (umin - d, umax + d, vmin - d, vmax + d);
    let (w, h) = (orig.0 as f32, orig.1 as f32);
    let pt = |a: f32, b: f32| {
        [
            ((mx + a * ux + b * vx) * sx).clamp(0.0, w - 1.0),
            ((my + a * uy + b * vy) * sy).clamp(0.0, h - 1.0),
        ]
    };
    // quad[0..1] runs along the principal axis; its sign for vertical blobs is not normalised.
    let quad = [
        pt(umin, vmin),
        pt(umax, vmin),
        pt(umax, vmax),
        pt(umin, vmax),
    ];
    let b = TextBox { quad, score: 0.0 };
    let (e1, e2) = b.extent();
    (e1.min(e2) >= p.min_side).then_some(quad)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_size_rounds_to_32_and_caps_long_side() {
        assert_eq!(det_input_size(1653, 2339, 960), (672, 960));
        assert_eq!(det_input_size(1653, 2339, 0), (1664, 2336));
        assert_eq!(det_input_size(10, 10, 960), (32, 32));
        assert_eq!(det_input_size(5000, 100, 0), (4000, 96));
    }

    #[test]
    fn one_horizontal_blob_gives_one_unclipped_box() {
        let (rw, rh) = (64, 32);
        let mut prob = vec![0f32; rw * rh];
        for y in 10..16 {
            for x in 8..56 {
                prob[y * rw + x] = 0.9;
            }
        }
        let b = db_boxes(&prob, rw, rh, 2.0, 2.0, (128, 64), &DbParams::default());
        assert_eq!(b.len(), 1);
        let [x0, y0, x1, y1] = b[0].aabb();
        // 48x6 blob, unclip d = 48*6*1.5/(2*54) = 4 px each side, scaled by 2.
        assert!(
            (x0 - 8.0).abs() < 1.5 && (x1 - 120.0).abs() < 1.5,
            "{x0} {x1}"
        );
        assert!(
            (y0 - 12.0).abs() < 1.5 && (y1 - 40.0).abs() < 1.5,
            "{y0} {y1}"
        );
        assert!(!b[0].is_vertical() && (b[0].score - 0.9).abs() < 1e-4);
    }

    #[test]
    fn weak_and_tiny_components_are_dropped() {
        let (rw, rh) = (32, 32);
        let mut prob = vec![0f32; rw * rh];
        for y in 2..8 {
            for x in 2..30 {
                prob[y * rw + x] = 0.45; // above thresh, below box_thresh
            }
        }
        prob[20 * rw + 20] = 0.99; // 1 px
        assert!(db_boxes(&prob, rw, rh, 1.0, 1.0, (32, 32), &DbParams::default()).is_empty());
    }

    #[test]
    fn tall_blob_is_vertical() {
        let (rw, rh) = (32, 96);
        let mut prob = vec![0f32; rw * rh];
        for y in 8..88 {
            for x in 12..18 {
                prob[y * rw + x] = 0.9;
            }
        }
        let b = db_boxes(&prob, rw, rh, 1.0, 1.0, (32, 96), &DbParams::default());
        assert!(b.len() == 1 && b[0].is_vertical(), "{b:?}");
    }

    #[test]
    fn malformed_input_does_not_panic() {
        let p = DbParams::default();
        assert!(db_boxes(&[0.9; 10], 8, 8, 1.0, 1.0, (8, 8), &p).is_empty());
        let (rw, rh) = (64, 32);
        let mut prob = vec![0f32; rw * rh];
        for y in 10..16 {
            for x in 8..56 {
                prob[y * rw + x] = 0.9;
            }
        }
        let _ = db_boxes(&prob, rw, rh, 1.0, 1.0, (0, 0), &p);
        prob[0] = f32::NAN;
        assert!(
            db_boxes(&prob, rw, rh, 1.0, 1.0, (64, 32), &p)
                .iter()
                .all(|b| b.score.is_finite())
        );
        assert_eq!(det_input_size(u32::MAX, 10, u32::MAX).0 % 32, 0);
    }
}
