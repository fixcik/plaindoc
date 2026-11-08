//! PP-OCRv5 rec pre/post-processing: oriented crop, input tensor, greedy CTC with char positions.

use crate::{Image, Recognized, TextBox};

/// Model input height.
pub const REC_H: u32 = 48;
/// Minimum padded input width (PaddleOCR rec_image_shape 3x48x320).
pub const REC_MIN_W: u32 = 320;
/// Maximum input width: one extreme crop must not allocate unbounded memory; wider ones are
/// squeezed down to this width.
pub const REC_MAX_W: u32 = 3200;

/// Character set of a rec model: class 0 = CTC blank, 1..=n = `chars`, n+1 = space.
#[derive(Clone, Debug, PartialEq)]
pub struct Charset {
    chars: Vec<String>,
}

/// `cyrillic_PP-OCRv5_mobile_rec` dictionary (850 symbols, Apache-2.0, from the model's inference.yml).
pub const CYRILLIC_DICT: &str = include_str!("../assets/dict_cyrillic.txt");

impl Charset {
    /// One symbol per line.
    pub fn from_lines(s: &str) -> Self {
        Self {
            chars: s.lines().map(str::to_string).collect(),
        }
    }

    pub fn cyrillic() -> Self {
        Self::from_lines(CYRILLIC_DICT)
    }

    /// Number of model output classes this charset expects.
    pub fn classes(&self) -> usize {
        self.chars.len() + 2
    }

    fn symbol(&self, class: usize) -> &str {
        self.chars.get(class - 1).map_or(" ", String::as_str)
    }
}

/// Warps the oriented `b.quad` into an upright crop (bilinear); `flip` rotates it by 180 degrees.
pub fn crop_quad(img: &Image, b: &TextBox, flip: bool) -> Image {
    let [c0, c1, _, c3] = b.quad;
    if img.width() == 0 || img.height() == 0 || b.quad.iter().flatten().any(|v| !v.is_finite()) {
        return Image::new(1, 1);
    }
    let (ex, ey) = (c1[0] - c0[0], c1[1] - c0[1]);
    let (fx, fy) = (c3[0] - c0[0], c3[1] - c0[1]);
    // Output size is bounded by the source size so hostile coordinates cannot force huge allocations.
    let cap = img.width().max(img.height());
    let ow = ((ex * ex + ey * ey).sqrt().round() as u32).clamp(1, cap);
    let oh = ((fx * fx + fy * fy).sqrt().round() as u32).clamp(1, cap);
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let mut out = Image::new(ow, oh);
    for y in 0..oh {
        for x in 0..ow {
            let a = (x as f32 + 0.5) / ow as f32;
            let c = (y as f32 + 0.5) / oh as f32;
            let px = (c0[0] + a * ex + c * fx - 0.5).clamp(0.0, (iw - 1.001).max(0.0));
            let py = (c0[1] + a * ey + c * fy - 0.5).clamp(0.0, (ih - 1.001).max(0.0));
            let (x0, y0) = (px.floor() as u32, py.floor() as u32);
            let (tx, ty) = (px - x0 as f32, py - y0 as f32);
            let (x1, y1) = (
                (x0 + 1).min(img.width() - 1),
                (y0 + 1).min(img.height() - 1),
            );
            let (p00, p10, p01, p11) = (
                img.get_pixel(x0, y0),
                img.get_pixel(x1, y0),
                img.get_pixel(x0, y1),
                img.get_pixel(x1, y1),
            );
            let mut v = [0u8; 3];
            for ch in 0..3 {
                let top = p00[ch] as f32 * (1.0 - tx) + p10[ch] as f32 * tx;
                let bot = p01[ch] as f32 * (1.0 - tx) + p11[ch] as f32 * tx;
                v[ch] = (top * (1.0 - ty) + bot * ty).round() as u8;
            }
            out.put_pixel(x, y, image::Rgb(v));
        }
    }
    if flip {
        image::imageops::rotate180(&out)
    } else {
        out
    }
}

/// One crop as an NCHW tensor `[1, 3, 48, W]`: resized to height 48 keeping aspect, padded with 0
/// to `W = max(320, resized width)`, BGR, `(x/255 - 0.5)/0.5`. Returns (data, W, resized width).
pub fn rec_tensor(crop: &Image) -> (Vec<f32>, u32, u32) {
    let ratio = crop.width() as f32 / crop.height().max(1) as f32;
    let rw = ((REC_H as f32 * ratio).ceil() as u32).clamp(1, REC_MAX_W);
    let iw = rw.max(REC_MIN_W);
    let r = image::imageops::resize(crop, rw, REC_H, image::imageops::FilterType::Triangle);
    let (h, w) = (REC_H as usize, iw as usize);
    let mut data = vec![0f32; 3 * h * w];
    for (x, y, p) in r.enumerate_pixels() {
        let bgr = [p[2], p[1], p[0]];
        for ch in 0..3 {
            data[(ch * h + y as usize) * w + x as usize] = (bgr[ch] as f32 / 255.0 - 0.5) / 0.5;
        }
    }
    (data, iw, rw)
}

/// Greedy CTC over `probs` (`t x c`, row-major). Emits a class when it differs from the previous
/// arg-max and is not blank; records its time step. Trims spaces at both ends. Empty text has
/// `conf = 0`; a shape mismatch (`c != classes` or `probs.len() < t * c`) gives an empty result.
pub fn ctc_greedy(probs: &[f32], t: usize, c: usize, charset: &Charset) -> Recognized {
    // Malformed model output (class count or length mismatch) yields an empty result, never a panic.
    if c != charset.classes() || t.checked_mul(c).is_none_or(|n| probs.len() < n) {
        return Recognized::default();
    }
    let mut text = String::new();
    let mut steps = Vec::new();
    let mut confs = Vec::new();
    let mut last = 0usize;
    for step in 0..t {
        let row = &probs[step * c..(step + 1) * c];
        let (mut am, mut mv) = (0usize, f32::MIN);
        for (k, &v) in row.iter().enumerate() {
            if v > mv {
                mv = v;
                am = k;
            }
        }
        if am != 0 && am != last {
            let s = charset.symbol(am);
            for ch in s.chars() {
                text.push(ch);
                steps.push(step as u32);
                confs.push(mv);
            }
        }
        last = am;
    }
    // Trim spaces, keeping steps aligned with chars.
    let chars: Vec<char> = text.chars().collect();
    let lo = chars.iter().position(|c| *c != ' ').unwrap_or(chars.len());
    let hi = chars.iter().rposition(|c| *c != ' ').map_or(lo, |i| i + 1);
    let text: String = chars[lo..hi].iter().collect();
    let conf_slice = &confs[lo..hi];
    Recognized {
        text,
        conf: if conf_slice.is_empty() {
            0.0
        } else {
            conf_slice.iter().sum::<f32>() / conf_slice.len() as f32
        },
        steps: steps[lo..hi].to_vec(),
        total_steps: t as u32,
        width_frac: 1.0,
        flipped: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn onehot(seq: &[usize], c: usize) -> Vec<f32> {
        let mut v = vec![0.01f32; seq.len() * c];
        for (t, &k) in seq.iter().enumerate() {
            v[t * c + k] = 0.9;
        }
        v
    }

    #[test]
    fn rec_tensor_width_is_capped() {
        let (data, iw, rw) = rec_tensor(&Image::new(100_000, 20));
        assert_eq!((iw, rw), (REC_MAX_W, REC_MAX_W));
        assert_eq!(data.len(), 3 * REC_H as usize * REC_MAX_W as usize);
    }

    #[test]
    fn cyrillic_charset_has_850_symbols_and_russian_letters() {
        let cs = Charset::cyrillic();
        assert_eq!(cs.classes(), 852);
        assert!(cs.chars.iter().any(|c| c == "Ё") && cs.chars.iter().any(|c| c == "№"));
    }

    #[test]
    fn ctc_collapses_repeats_drops_blanks_and_keeps_steps() {
        let cs = Charset::from_lines("a\nb");
        let space = cs.classes() - 1;
        // a a _ a b space space b
        let p = onehot(&[1, 1, 0, 1, 2, space, space, 2, 0], cs.classes());
        let r = ctc_greedy(&p, 9, cs.classes(), &cs);
        assert_eq!(r.text, "aab b");
        assert_eq!(r.steps, vec![0, 3, 4, 5, 7]);
        assert!((r.conf - 0.9).abs() < 1e-6);
        assert_eq!(r.total_steps, 9);
    }

    #[test]
    fn ctc_trims_spaces_and_empty_has_zero_conf() {
        let cs = Charset::from_lines("a");
        let sp = cs.classes() - 1;
        let r = ctc_greedy(&onehot(&[sp, 1, sp], cs.classes()), 3, cs.classes(), &cs);
        assert_eq!((r.text.as_str(), r.steps.as_slice()), ("a", &[1u32][..]));
        let e = ctc_greedy(&onehot(&[0, 0], cs.classes()), 2, cs.classes(), &cs);
        assert_eq!((e.text.as_str(), e.conf), ("", 0.0));
    }

    #[test]
    fn rec_tensor_pads_to_min_width() {
        let crop = Image::from_pixel(100, 24, image::Rgb([255, 255, 255]));
        let (d, iw, rw) = rec_tensor(&crop);
        assert_eq!((iw, rw), (320, 200));
        assert_eq!(d.len(), 3 * 48 * 320);
        assert_eq!(d[0], 1.0);
        assert_eq!(d[250], 0.0, "padding");
    }

    #[test]
    fn crop_of_axis_box_is_the_region() {
        let mut img = Image::from_pixel(40, 20, image::Rgb([255, 255, 255]));
        img.put_pixel(10, 5, image::Rgb([0, 0, 0]));
        let c = crop_quad(&img, &TextBox::from_aabb(10.0, 5.0, 30.0, 15.0, 1.0), false);
        assert_eq!(c.dimensions(), (20, 10));
        assert_eq!(c.get_pixel(0, 0)[0], 0);
        let f = crop_quad(&img, &TextBox::from_aabb(10.0, 5.0, 30.0, 15.0, 1.0), true);
        assert_eq!(f.get_pixel(19, 9)[0], 0);
    }

    #[test]
    fn crop_degenerate_sources_and_quads_do_not_panic_or_blow_up() {
        let b = TextBox::from_aabb(0.0, 0.0, 1.0, 10.0, 1.0);
        assert_eq!(crop_quad(&Image::new(0, 0), &b, false).dimensions(), (1, 1));
        assert_eq!(
            crop_quad(&Image::new(1, 10), &b, false).dimensions(),
            (1, 10)
        );
        assert_eq!(
            crop_quad(&Image::new(10, 1), &b, true).dimensions(),
            (1, 10)
        );
        let inf = TextBox::from_aabb(0.0, 0.0, f32::INFINITY, 10.0, 1.0);
        assert_eq!(
            crop_quad(&Image::new(8, 8), &inf, false).dimensions(),
            (1, 1)
        );
        let huge = TextBox::from_aabb(0.0, 0.0, 1e9, 1e9, 1.0);
        assert_eq!(
            crop_quad(&Image::new(8, 6), &huge, false).dimensions(),
            (8, 8)
        );
    }

    #[test]
    fn ctc_shape_mismatch_gives_empty_result() {
        let cs = Charset::from_lines("a");
        let p = onehot(&[1, 1], cs.classes());
        assert_eq!(
            ctc_greedy(&p, 2, cs.classes() + 1, &cs),
            Recognized::default()
        );
        assert_eq!(ctc_greedy(&p, 3, cs.classes(), &cs), Recognized::default());
        assert_eq!(
            ctc_greedy(&p, usize::MAX, cs.classes(), &cs),
            Recognized::default()
        );
    }
}
