//! Data types shared by engines, the pipeline and callers. Pixel coordinates: origin top-left, y down.

use crate::lines::Segment;

/// RGB page or crop image.
pub type Image = image::RgbImage;

/// A detected text line: oriented rectangle `quad` = [start-top, end-top, end-bottom, start-bottom] in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextBox {
    pub quad: [[f32; 2]; 4],
    /// Mean DB probability over the component.
    pub score: f32,
}

impl TextBox {
    /// Axis-aligned box from pixel bounds.
    pub fn from_aabb(x0: f32, y0: f32, x1: f32, y1: f32, score: f32) -> Self {
        Self {
            quad: [[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
            score,
        }
    }

    /// `[x0, y0, x1, y1]` hull of the quad.
    pub fn aabb(&self) -> [f32; 4] {
        let xs = self.quad.map(|p| p[0]);
        let ys = self.quad.map(|p| p[1]);
        [
            xs.iter().copied().fold(f32::MAX, f32::min),
            ys.iter().copied().fold(f32::MAX, f32::min),
            xs.iter().copied().fold(f32::MIN, f32::max),
            ys.iter().copied().fold(f32::MIN, f32::max),
        ]
    }

    /// Length of the text direction edge and of the line-height edge.
    pub fn extent(&self) -> (f32, f32) {
        let d = |a: [f32; 2], b: [f32; 2]| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        (d(self.quad[0], self.quad[1]), d(self.quad[0], self.quad[3]))
    }

    /// Angle of the text direction in degrees, (-180, 180], 0 = left to right.
    pub fn angle_deg(&self) -> f32 {
        let [a, b] = [self.quad[0], self.quad[1]];
        (b[1] - a[1]).atan2(b[0] - a[0]).to_degrees()
    }

    /// Text runs closer to vertical than horizontal.
    pub fn is_vertical(&self) -> bool {
        let a = self.angle_deg().abs();
        (45.0..135.0).contains(&a)
    }
}

/// Recognition result for one box. `steps[i]` is the CTC time step of the i-th char of `text`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recognized {
    pub text: String,
    /// Mean max-probability over emitted chars, 0 for empty text.
    pub conf: f32,
    pub steps: Vec<u32>,
    /// CTC sequence length T of the model output.
    pub total_steps: u32,
    /// Fraction of the model input width covered by the crop (crop width / padded width).
    pub width_frac: f32,
    /// The crop was read rotated by 180 degrees.
    pub flipped: bool,
}

/// One word in page pixels. `line`/`block` index into [`OcrPage::lines`] / block order.
#[derive(Clone, Debug, PartialEq)]
pub struct OcrWord {
    pub text: String,
    /// `[x0, y0, x1, y1]` in pixels of the input image.
    pub bbox: [f32; 4],
    pub conf: f32,
    pub line: u32,
    pub block: u32,
    /// Text direction in degrees (skew of the line), 0 = horizontal.
    pub angle: f32,
    /// Estimated font size in pixels (ink height / 0.8).
    pub font_px: f32,
    /// Per-char `(x0, x1)` spans in pixels of the input image.
    pub char_x: Vec<(f32, f32)>,
}

/// One text line in reading order.
#[derive(Clone, Debug, PartialEq)]
pub struct OcrLine {
    pub text: String,
    pub bbox: [f32; 4],
    pub conf: f32,
    pub block: u32,
}

/// How the aligned raster (where words and lines live) was made from the source image:
/// aligned = `rotation` (clockwise, as `imageops::rotate90/180/270`) of `raster::rotate(src, skew_deg)`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AlignFrame {
    /// 0, 90, 180 or 270.
    pub rotation: u32,
    pub skew_deg: f32,
    /// Size of the source image.
    pub src_w: u32,
    pub src_h: u32,
}

impl AlignFrame {
    /// `[x0, y0, x1, y1]` in the aligned raster -> enclosing rectangle in the source image
    /// (4 corners: inverse 90/180/270, then the skew rotation with `-skew_deg`).
    pub fn to_source(&self, r: [f32; 4]) -> [f32; 4] {
        if self.rotation == 0 && self.skew_deg == 0.0 {
            return r;
        }
        let (w, h) = (self.src_w as f32, self.src_h as f32);
        let (cx, cy) = (w / 2.0, h / 2.0);
        let (s, c) = (-self.skew_deg).to_radians().sin_cos();
        let map = |[x, y]: [f32; 2]| {
            let (x, y) = match self.rotation {
                90 => (y, h - x),
                180 => (w - x, h - y),
                270 => (w - y, x),
                _ => (x, y),
            };
            let (dx, dy) = (x - cx, y - cy);
            [cx + dx * c + dy * s, cy - dx * s + dy * c]
        };
        let pts = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]].map(map);
        pts[1..]
            .iter()
            .fold([pts[0][0], pts[0][1], pts[0][0], pts[0][1]], |a, p| {
                [
                    a[0].min(p[0]),
                    a[1].min(p[1]),
                    a[2].max(p[0]),
                    a[3].max(p[1]),
                ]
            })
    }
}

/// OCR result of one image.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OcrPage {
    /// Size of the aligned raster (the system of word and line boxes; the source size is in `frame`).
    pub width: u32,
    pub height: u32,
    /// Words in reading order.
    pub words: Vec<OcrWord>,
    /// Lines in reading order.
    pub lines: Vec<OcrLine>,
    /// Char-weighted mean confidence, 0 when empty.
    pub mean_conf: f32,
    /// Skipped by the blank-page rule (no detector run).
    pub blank: bool,
    /// Recognition stopped early: deadline passed or the job was cancelled.
    pub timed_out: bool,
    /// Detector boxes beyond `max_boxes` that were not recognised.
    pub boxes_dropped: usize,
    /// Page turn found by the orientation check: 0, 90, 180 or 270 (clockwise).
    pub rotation: u32,
    /// Applied skew correction in degrees (0 when below the threshold or `deskew` is off).
    pub skew_deg: f32,
    /// Aligned raster -> source image mapping of the boxes.
    pub frame: AlignFrame,
    /// Aligned gray raster, kept only with `OcrParams.keep_aligned`.
    pub aligned_gray: Option<image::GrayImage>,
    /// Raster ruling lines of the aligned page (`OcrParams.table_lines`, filled by task 22).
    pub segments: Vec<Segment>,
    /// Det ran at the raised R-12 resolution (task 22; long side capped at 1760 px).
    pub full_res_det: bool,
}

impl OcrPage {
    /// The page has text: at least `min_chars` non-space chars in words with `conf >= min_conf`
    /// (spike: 5 chars at 0.8 separates text from photo noise).
    pub fn has_text(&self, min_chars: usize, min_conf: f32) -> bool {
        self.words
            .iter()
            .filter(|w| w.conf >= min_conf)
            .map(|w| w.text.chars().filter(|c| !c.is_whitespace()).count())
            .sum::<usize>()
            >= min_chars
    }

    /// Lines joined by `\n`, blocks separated by an empty line.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (i, l) in self.lines.iter().enumerate() {
            if i > 0 {
                out.push_str(if self.lines[i - 1].block != l.block {
                    "\n\n"
                } else {
                    "\n"
                });
            }
            out.push_str(&l.text);
        }
        out
    }
}
