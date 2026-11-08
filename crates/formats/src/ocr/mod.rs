//! OCR glue shared by the PDF and image parsers: settings, pixel <-> point geometry, OCR result
//! -> blocks, stage-2 warnings.

pub(crate) mod images;
#[cfg(feature = "ocr-paddle")]
mod lazy;

pub use images::{ImageLimits, ImageParser};
#[cfg(feature = "ocr-paddle")]
pub use lazy::paddle_engine;
/// Model manifest and resolver (for `models list`).
#[cfg(feature = "ocr-paddle")]
pub use plaindoc_ocr::models;

use std::sync::Arc;

use plaindoc_core::{BBox, Block, Level, Warning, WarningKind};
#[cfg(test)]
use plaindoc_ocr::OcrWord;
use plaindoc_ocr::{OcrEngine, OcrPage, OcrParams};

/// OCR settings of a parser. `engine: None` means OCR is unavailable in this setup.
#[non_exhaustive]
#[derive(Clone)]
pub struct OcrConfig {
    pub engine: Option<Arc<dyn OcrEngine>>,
    /// Render resolution for page OCR (spike: 200 dpi, ~17 ms render).
    pub page_dpi: u32,
    /// Images on a page smaller than this (shorter side, px at `page_dpi`) are not OCRed.
    pub min_region_px: u32,
    pub params: OcrParams,
    /// Mean confidence below this gives `LowOcrConfidence`. Calibration (`tests/ocr_eval.rs`, 51 synthetic
    /// pages per variant, criteria: c200 0 %, s150 <= 30 % of pages below): page means c200 min 0.873,
    /// median 0.971, 0 pages below 0.75; s150 min 0.722, p10 0.903, median 0.949, 1 page (2 %) below 0.75
    /// (1 below 0.80, 3 below 0.85). Kept at 0.75.
    pub low_confidence: f32,
    /// Decoding limits for standalone and embedded images.
    pub limits: ImageLimits,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            engine: None,
            page_dpi: 200,
            min_region_px: 24,
            params: OcrParams::default(),
            low_confidence: 0.75,
            limits: ImageLimits::default(),
        }
    }
}

impl OcrConfig {
    /// Settings with the build's default engine: PaddleOCR under `ocr-paddle`, none otherwise.
    pub fn default_engine() -> Self {
        #[cfg(feature = "ocr-paddle")]
        return Self::with_engine(lazy::paddle_engine(None, None));
        #[cfg(not(feature = "ocr-paddle"))]
        Self::default()
    }

    pub fn with_engine(engine: Arc<dyn OcrEngine>) -> Self {
        Self {
            engine: Some(engine),
            ..Self::default()
        }
    }
}

/// Raster of a page and its PDF geometry: maps raster pixels (top-left, y down) to PDF user space
/// (bottom-left, y up), inverting `pdf_oxide` `page_base_transform` incl. `/Rotate`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PageGeom {
    /// Rendered box (CropBox ∩ MediaBox) `[x0, y0, x1, y1]` in points.
    pub media: [f32; 4],
    /// `/Rotate` normalised to 0, 90, 180 or 270.
    pub rotation: u32,
    /// Pixels per point of the actual raster.
    pub scale: f32,
}

impl PageGeom {
    /// Geometry of a standalone image: 1 px = 1 pt, origin at the bottom-left corner.
    pub fn image(w: u32, h: u32) -> Self {
        Self {
            media: [0.0, 0.0, w as f32, h as f32],
            rotation: 0,
            scale: 1.0,
        }
    }

    fn wh(&self) -> (f32, f32) {
        (self.media[2] - self.media[0], self.media[3] - self.media[1])
    }

    /// Raster size in points after rotation.
    pub fn rotated_extent(&self) -> (f32, f32) {
        let (w, h) = self.wh();
        if self.rotation % 180 == 90 {
            (h, w)
        } else {
            (w, h)
        }
    }

    /// Raster size in pixels at `scale` (the rendered page, before alignment).
    #[cfg(test)]
    pub(crate) fn image_size(&self) -> (u32, u32) {
        let (w, h) = self.rotated_extent();
        (
            (w * self.scale).round() as u32,
            (h * self.scale).round() as u32,
        )
    }

    pub fn px_to_pt(&self, dx: f32, dy: f32) -> (f32, f32) {
        let ([mx, my, ..], (w, h), s) = (self.media, self.wh(), self.scale);
        let (u, v) = (dx / s, dy / s);
        match self.rotation {
            90 => (mx + v, my + u),
            180 => (mx + w - u, my + v),
            270 => (mx + w - v, my + h - u),
            _ => (mx + u, my + h - v),
        }
    }

    pub fn pt_to_px(&self, x: f32, y: f32) -> (f32, f32) {
        let ([mx, my, ..], (w, h), s) = (self.media, self.wh(), self.scale);
        let (x, y) = (x - mx, y - my);
        let (u, v) = match self.rotation {
            90 => (y, x),
            180 => (w - x, y),
            270 => (h - y, w - x),
            _ => (x, h - y),
        };
        (u * s, v * s)
    }

    /// Pixel rect `[x0, y0, x1, y1]` -> PDF bbox.
    pub fn bbox(&self, r: [f32; 4]) -> BBox {
        let pts = [(r[0], r[1]), (r[2], r[1]), (r[2], r[3]), (r[0], r[3])]
            .map(|(x, y)| self.px_to_pt(x, y));
        let (x0, x1) = minmax(pts.iter().map(|p| p.0));
        let (y0, y1) = minmax(pts.iter().map(|p| p.1));
        BBox::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// PDF rect (`x, y` = bottom-left, `w, h`) -> pixel rect `[x0, y0, x1, y1]`.
    pub fn px_rect(&self, x: f32, y: f32, w: f32, h: f32) -> [f32; 4] {
        let pts =
            [(x, y), (x + w, y), (x + w, y + h), (x, y + h)].map(|(a, b)| self.pt_to_px(a, b));
        let (x0, x1) = minmax(pts.iter().map(|p| p.0));
        let (y0, y1) = minmax(pts.iter().map(|p| p.1));
        [x0, y0, x1, y1]
    }
}

fn minmax(it: impl Iterator<Item = f32>) -> (f32, f32) {
    it.fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v), b.max(v)))
}

/// Smallest rect `[x0, y0, x1, y1]` covering `a` and `b`.
fn union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// Drops the alpha channel of RGBA8 pixels.
pub(crate) fn rgba_to_rgb(rgba: &[u8]) -> Vec<u8> {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

/// One `Paragraph` per OCR block (lines joined by `\n`), boxes in PDF points. `offset` shifts
/// crop-local pixels into page pixels. `keep` filters words (dedup against the text layer).
pub(crate) fn ocr_blocks(
    page: &OcrPage,
    geom: &PageGeom,
    offset: (f32, f32),
    keep: &dyn Fn(&[f32; 4]) -> bool,
) -> Vec<Block> {
    ocr_blocks_aligned(page, geom, offset, keep)
        .into_iter()
        .map(|(b, _)| b)
        .collect()
}

/// [`ocr_blocks`] with each block's rect `[x0, y0, x1, y1]` in pixels of the aligned raster of `page`
/// (the frame of its reading order).
pub(crate) fn ocr_blocks_aligned(
    page: &OcrPage,
    geom: &PageGeom,
    offset: (f32, f32),
    keep: &dyn Fn(&[f32; 4]) -> bool,
) -> Vec<(Block, [f32; 4])> {
    type Cur = (u32, Vec<String>, [f32; 4], [f32; 4]);
    let mut blocks: Vec<(Block, [f32; 4])> = Vec::new();
    let mut cur: Option<Cur> = None;
    let shift = |b: [f32; 4]| {
        let b = page.frame.to_source(b);
        [
            b[0] + offset.0,
            b[1] + offset.1,
            b[2] + offset.0,
            b[3] + offset.1,
        ]
    };
    let flush = |c: Option<Cur>, out: &mut Vec<(Block, [f32; 4])>| {
        if let Some((_, lines, r, a)) = c
            && !lines.is_empty()
        {
            out.push((Block::paragraph(lines.join("\n"), Some(geom.bbox(r))), a));
        }
    };
    for (li, line) in page.lines.iter().enumerate() {
        let words: Vec<_> = page
            .words
            .iter()
            .filter(|w| w.line == li as u32 && keep(&shift(w.bbox)))
            .collect();
        if words.is_empty() {
            continue;
        }
        let text = words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let empty = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        let r = words.iter().fold(empty, |a, w| union(a, shift(w.bbox)));
        let al = words.iter().fold(empty, |a, w| union(a, w.bbox));
        match &mut cur {
            Some((blk, lines, acc, aacc)) if *blk == line.block => {
                lines.push(text);
                *acc = union(*acc, r);
                *aacc = union(*aacc, al);
            }
            _ => {
                flush(cur.take(), &mut blocks);
                cur = Some((line.block, vec![text], r, al));
            }
        }
    }
    flush(cur, &mut blocks);
    blocks
}

/// Stage-2 warnings for one OCRed section, and its quality (mean confidence).
pub(crate) fn ocr_warnings(pages: &[&OcrPage], scope: &str, low: f32) -> (Vec<Warning>, f32) {
    let words: usize = pages.iter().map(|p| p.words.len()).sum();
    let chars: f32 = pages
        .iter()
        .flat_map(|p| &p.words)
        .map(|w| w.text.chars().count() as f32)
        .sum();
    let conf = if chars > 0.0 {
        pages
            .iter()
            .flat_map(|p| &p.words)
            .map(|w| w.conf * w.text.chars().count() as f32)
            .sum::<f32>()
            / chars
    } else {
        0.0
    };
    let has_text = pages.iter().any(|p| p.has_text(5, 0.8));
    // Pages the orientation check turned (clockwise degrees), named only when there are any.
    let turned: Vec<String> = pages
        .iter()
        .filter(|p| p.rotation != 0)
        .map(|p| format!("{}°", p.rotation))
        .collect();
    let turned = if turned.is_empty() {
        String::new()
    } else {
        format!("; turned by {}", turned.join(", "))
    };
    let mut w = vec![Warning::new(
        WarningKind::OcrUsed,
        Level::Info,
        format!("OCR ({scope}): {words} words, mean confidence {conf:.2}{turned}"),
    )];
    if !has_text {
        w.push(Warning::new(
            WarningKind::OcrEmpty,
            Level::Warn,
            format!("OCR ({scope}) found no text"),
        ));
        return (w, 0.0);
    }
    if conf < low {
        w.push(Warning::new(
            WarningKind::LowOcrConfidence,
            Level::Warn,
            format!("OCR mean confidence {conf:.2} < {low:.2}"),
        ));
    }
    (w, conf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn px_pt_roundtrip_all_rotations_with_offset_media() {
        for rotation in [0, 90, 180, 270] {
            let g = PageGeom {
                media: [10.0, 20.0, 622.0, 812.0],
                rotation,
                scale: 200.0 / 72.0,
            };
            for (x, y) in [(10.0, 20.0), (300.0, 400.0), (622.0, 812.0)] {
                let (px, py) = g.pt_to_px(x, y);
                let (bx, by) = g.px_to_pt(px, py);
                assert!(
                    (bx - x).abs() < 1e-3 && (by - y).abs() < 1e-3,
                    "rot {rotation}: {x},{y}"
                );
                let (ew, eh) = g.rotated_extent();
                assert!(
                    px >= -1e-3
                        && py >= -1e-3
                        && px <= ew * g.scale + 1e-2
                        && py <= eh * g.scale + 1e-2
                );
            }
        }
    }

    #[test]
    fn top_left_pixel_is_top_left_of_unrotated_page() {
        let g = PageGeom {
            media: [0.0, 0.0, 612.0, 792.0],
            rotation: 0,
            scale: 1.0,
        };
        let b = g.bbox([0.0, 0.0, 100.0, 10.0]);
        assert_eq!((b.x, b.y, b.w, b.h), (0.0, 782.0, 100.0, 10.0));
        // /Rotate 90: raster top-left = PDF (x0, y0)
        let r = PageGeom { rotation: 90, ..g };
        assert_eq!(r.px_to_pt(0.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn warnings_used_empty_and_low_confidence() {
        let page = |conf: f32, text: &str| OcrPage {
            words: vec![OcrWord {
                text: text.into(),
                bbox: [0.0; 4],
                conf,
                line: 0,
                block: 0,
                angle: 0.0,
                font_px: 10.0,
                char_x: vec![],
            }],
            ..Default::default()
        };
        let (w, q) = ocr_warnings(&[&page(0.95, "Договор")], "full page", 0.75);
        assert_eq!(
            w.iter().map(|w| (w.kind, w.level)).collect::<Vec<_>>(),
            vec![(WarningKind::OcrUsed, Level::Info)]
        );
        assert!((q - 0.95).abs() < 1e-6);
        assert!(!w[0].message.contains("turned"));
        let turned = OcrPage {
            rotation: 270,
            ..page(0.95, "Договор")
        };
        let (w, _) = ocr_warnings(&[&turned], "full page", 0.75);
        assert_eq!(
            w[0].message,
            "OCR (full page): 1 words, mean confidence 0.95; turned by 270°"
        );
        let (w, _) = ocr_warnings(&[&page(0.85, "Договор")], "x", 0.9);
        assert!(
            w.iter()
                .any(|w| w.kind == WarningKind::LowOcrConfidence && w.level == Level::Warn)
        );
        let (w, q) = ocr_warnings(&[&page(0.95, "ab")], "x", 0.75);
        assert!(w.iter().any(|w| w.kind == WarningKind::OcrEmpty) && q == 0.0);
    }
}
