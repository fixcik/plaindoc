//! Engine-agnostic page OCR: blank rule -> deskew -> det -> orientation -> rec -> words -> reading order
//! -> fixes.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use image::imageops;

use crate::layout::{LayoutParams, Rect, lines_in_block, xy_cut};
use crate::lines::{LinesParams, Segment, extract_lines};
use crate::words::{InkMap, split_words};
use crate::{
    Image, OcrEngine, OcrError, OcrLine, OcrPage, OcrWord, Recognized, TextBox, blank, fix, raster,
};

/// Pipeline settings (engine settings live with the engine).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OcrParams {
    /// Boxes recognised below this confidence are dropped (spike: 0.5).
    pub min_box_conf: f32,
    /// Skip det on pages the blank rule calls empty.
    pub blank_check: bool,
    /// Latin->Cyrillic look-alike fix in Cyrillic context.
    pub homoglyphs: bool,
    /// `1 250.75` -> `1 250,75` when the line already uses the decimal comma.
    pub decimal_comma: bool,
    /// Ink = gray below this share of the page background (p90 gray).
    pub ink_ratio: f32,
    pub layout: LayoutParams,
    /// Recognition stops between chunks once this instant has passed (`OcrPage.timed_out`).
    pub deadline: Option<Instant>,
    /// At most this many detector boxes are recognised per image (hostile noise pages produce
    /// thousands); the rest are counted in `OcrPage.boxes_dropped`.
    pub max_boxes: usize,
    /// Estimate the skew and work on the deskewed raster (applied from `MIN_SKEW_DEG`).
    pub deskew: bool,
    /// Orientation check: 180° on horizontal pages, 90/270° on mostly vertical ones (`OcrPage.rotation`).
    pub orientation: bool,
    /// Keep the aligned gray raster in `OcrPage.aligned_gray` (raster lines, task 22).
    pub keep_aligned: bool,
    /// Raster ruling lines of the aligned page into `OcrPage.segments`. A ruled-table page (R-12: at
    /// least 2 horizontal rules of 200 px and 2 vertical of 90 px at 150 dpi) is detected at full
    /// resolution up to `R12_DET_MAX_SIDE` (`OcrPage.full_res_det`).
    pub table_lines: Option<LinesParams>,
}

/// Boxes recognised per engine call; deadline and cancel are checked between chunks.
pub const REC_CHUNK: usize = 16;

/// Det long-side cap of a ruled-table page (R-12): full resolution of a 150-dpi A4 page, where the
/// ocr-tables spike measured the table gain; bounds det memory on 200-dpi renders (0.8 GB per
/// session pair).
const R12_DET_MAX_SIDE: u32 = 1760;
/// Smaller skew estimates are not applied.
const MIN_SKEW_DEG: f32 = 0.25;
/// Boxes read per orientation variant (the widest ones).
const ORIENT_PROBES: usize = 5;
/// A turned variant must read better by at least this confidence.
const ORIENT_MARGIN: f32 = 0.1;
/// Share of box area that makes a page "vertical" (90/270° candidates).
const VERTICAL_SHARE: f32 = 0.6;

impl Default for OcrParams {
    fn default() -> Self {
        Self {
            min_box_conf: 0.5,
            blank_check: true,
            homoglyphs: true,
            decimal_comma: false,
            ink_ratio: 0.72,
            layout: LayoutParams::default(),
            deadline: None,
            max_boxes: 2000,
            deskew: true,
            orientation: true,
            keep_aligned: false,
            table_lines: None,
        }
    }
}

/// The `ORIENT_PROBES` boxes with the longest text edge (ties by index).
fn widest(boxes: &[TextBox]) -> Vec<TextBox> {
    let mut idx: Vec<usize> = (0..boxes.len()).collect();
    idx.sort_by(|&a, &b| {
        boxes[b]
            .extent()
            .0
            .total_cmp(&boxes[a].extent().0)
            .then(a.cmp(&b))
    });
    idx.into_iter()
        .take(ORIENT_PROBES)
        .map(|i| boxes[i])
        .collect()
}

fn mean_conf(r: &[Recognized]) -> f32 {
    if r.is_empty() {
        0.0
    } else {
        r.iter().map(|r| r.conf).sum::<f32>() / r.len() as f32
    }
}

/// `b` in an image of `w×h` turned by 180°, corners reordered so the text still starts at `quad[0]`.
fn turn180(b: &TextBox, w: f32, h: f32) -> TextBox {
    let m = |[x, y]: [f32; 2]| [w - x, h - y];
    let q = b.quad;
    TextBox {
        quad: [m(q[2]), m(q[3]), m(q[0]), m(q[1])],
        score: b.score,
    }
}

/// Orientation check on the (deskewed) page `img` with its detector `boxes`. Returns the clockwise turn
/// (90/180/270), the turned raster and its boxes when a turn reads clearly better than 0° (I9), else `None`.
/// At most 3 det and 3 rec calls of `ORIENT_PROBES` boxes.
fn orient(
    engine: &dyn OcrEngine,
    img: &Image,
    boxes: &[TextBox],
) -> Result<Option<(u32, Image, Vec<TextBox>)>, OcrError> {
    let area = |b: &TextBox| {
        let (l, t) = b.extent();
        l * t
    };
    let total: f32 = boxes.iter().map(area).sum();
    let vertical: f32 = boxes.iter().filter(|b| b.is_vertical()).map(area).sum();
    let probes = widest(boxes);
    let base = engine.recognize(img, &probes)?;
    if total > 0.0 && vertical >= VERTICAL_SHARE * total {
        // Mostly vertical text: 0° (vertical boxes are read both ways by the engine) vs 90° and 270°.
        let base = mean_conf(&base);
        let turn = |rotation| {
            if rotation == 90 {
                imageops::rotate90(img)
            } else {
                imageops::rotate270(img)
            }
        };
        // Only the winner's boxes are kept; its raster is rebuilt at the end (caps the peak at 3 rasters).
        let mut best: Option<(f32, u32, Vec<TextBox>)> = None;
        for rotation in [90, 270] {
            let turned = turn(rotation);
            let b = engine.detect(&turned)?;
            if b.is_empty() {
                continue;
            }
            let conf = mean_conf(&engine.recognize(&turned, &widest(&b))?);
            if conf >= base + ORIENT_MARGIN && best.as_ref().is_none_or(|x| conf > x.0) {
                best = Some((conf, rotation, b));
            }
        }
        return Ok(best.map(|(_, r, b)| (r, turn(r), b)));
    }
    // Horizontal text: the same boxes read upside down; 180° needs a clear win on at least 3 of 5 probes.
    let turned = imageops::rotate180(img);
    let (w, h) = (img.width() as f32, img.height() as f32);
    let mirrored: Vec<TextBox> = probes.iter().map(|b| turn180(b, w, h)).collect();
    let flipped = engine.recognize(&turned, &mirrored)?;
    let wins = base
        .iter()
        .zip(&flipped)
        .filter(|(a, f)| f.conf >= a.conf + ORIENT_MARGIN)
        .count();
    if wins * ORIENT_PROBES >= 3 * probes.len() {
        let b = engine.detect(&turned)?;
        return Ok(Some((180, turned, b)));
    }
    Ok(None)
}

/// Ruling lines of the aligned raster `img` and the R-12 verdict (high-resolution det wanted).
fn table_lines(img: &Image, lp: &LinesParams) -> (Vec<Segment>, bool) {
    let (w, h) = img.dimensions();
    let k = ((31.0 * lp.dpi / 150.0) as u32) | 1;
    let norm = raster::normalize_bg(&raster::gray(img), k);
    let segs = extract_lines(&norm, w, h, lp);
    let s = lp.dpi / 150.0;
    let count = |horizontal: bool, min: f32| {
        segs.iter()
            .filter(|g| g.horizontal == horizontal && g.b - g.a >= min * s)
            .count()
    };
    let full = count(true, 200.0) >= 2 && count(false, 90.0) >= 2;
    (segs, full)
}

fn ink_map(g: &image::GrayImage, ratio: f32) -> Vec<u8> {
    let mut hist = [0u64; 256];
    for p in g.pixels() {
        hist[p[0] as usize] += 1;
    }
    let target = (g.len() as f64 * 0.9) as u64;
    let mut acc = 0;
    let bg = hist
        .iter()
        .position(|&h| {
            acc += h;
            acc >= target
        })
        .unwrap_or(255) as f32;
    let thr = bg * ratio;
    g.pixels().map(|p| u8::from((p[0] as f32) < thr)).collect()
}

/// OCRs one image. Words and lines are in reading order with pixel boxes.
pub fn ocr_image(engine: &dyn OcrEngine, img: &Image, p: &OcrParams) -> Result<OcrPage, OcrError> {
    ocr_image_cancellable(engine, img, p, &AtomicBool::new(false))
}

/// [`ocr_image`] that also stops between recognition chunks when `cancel` is set (job timeout).
pub fn ocr_image_cancellable(
    engine: &dyn OcrEngine,
    img: &Image,
    p: &OcrParams,
    cancel: &AtomicBool,
) -> Result<OcrPage, OcrError> {
    let mut page = OcrPage {
        width: img.width(),
        height: img.height(),
        ..Default::default()
    };
    page.frame.src_w = img.width();
    page.frame.src_h = img.height();
    if img.width() == 0 || img.height() == 0 {
        return Ok(page);
    }
    if p.blank_check && blank::is_blank(img) {
        page.blank = true;
        return Ok(page);
    }
    let stopped =
        || cancel.load(Ordering::Relaxed) || p.deadline.is_some_and(|d| Instant::now() >= d);
    // Work on the aligned raster from here on: deskewed, then turned by the orientation check.
    let mut work = Cow::Borrowed(img);
    if p.deskew {
        let a = raster::estimate_skew(&raster::gray(img));
        if a.abs() >= MIN_SKEW_DEG {
            work = Cow::Owned(raster::rotate(img, a, [255; 3]));
            page.skew_deg = a;
        }
    }
    if let Some(lp) = &p.table_lines {
        (page.segments, page.full_res_det) = table_lines(&work, lp);
    }
    let mut boxes = if page.full_res_det {
        engine.detect_at(&work, R12_DET_MAX_SIDE)?
    } else {
        engine.detect(&work)?
    };
    if p.orientation
        && !boxes.is_empty()
        && !stopped()
        && let Some((rotation, turned, turned_boxes)) = orient(engine, &work, &boxes)?
    {
        work = Cow::Owned(turned);
        boxes = turned_boxes;
        page.rotation = rotation;
        if let Some(lp) = &p.table_lines {
            (page.segments, page.full_res_det) = table_lines(&work, lp);
            if page.full_res_det && !stopped() {
                boxes = engine.detect_at(&work, R12_DET_MAX_SIDE)?;
            }
        }
    }
    let img: &Image = &work;
    page.frame.rotation = page.rotation;
    page.frame.skew_deg = page.skew_deg;
    (page.width, page.height) = img.dimensions();
    if p.keep_aligned {
        page.aligned_gray = Some(raster::gray(img));
    }
    if boxes.is_empty() {
        return Ok(page);
    }
    if boxes.len() > p.max_boxes {
        page.boxes_dropped = boxes.len() - p.max_boxes;
        boxes.truncate(p.max_boxes);
    }
    let mut recs = Vec::with_capacity(boxes.len());
    for chunk in boxes.chunks(REC_CHUNK) {
        if stopped() {
            page.timed_out = true;
            break;
        }
        let r = engine.recognize(img, chunk)?;
        if r.len() != chunk.len() {
            return Err(OcrError::Engine(format!(
                "recognizer returned {} result(s) for {} box(es)",
                r.len(),
                chunk.len()
            )));
        }
        recs.extend(r);
    }
    let kept: Vec<_> = boxes
        .iter()
        .zip(recs)
        .filter(|(_, r)| r.conf >= p.min_box_conf && !r.text.trim().is_empty())
        .collect();
    if kept.is_empty() {
        return Ok(page);
    }
    let rects: Vec<Rect> = kept.iter().map(|(b, _)| b.aabb()).collect();
    let ink = match &page.aligned_gray {
        Some(g) => ink_map(g, p.ink_ratio),
        None => ink_map(&raster::gray(img), p.ink_ratio),
    };
    let ink = InkMap {
        data: &ink,
        width: img.width() as usize,
        height: img.height() as usize,
    };
    let (mut chars, mut conf_sum) = (0usize, 0f32);
    for (bi, block) in xy_cut(&rects, &p.layout).iter().enumerate() {
        for line in lines_in_block(&rects, block) {
            let li = page.lines.len() as u32;
            let mut texts = Vec::new();
            let mut lconf = (0f32, 0usize);
            let first_word = page.words.len();
            for &k in &line {
                let (b, r) = &kept[k];
                let a = rects[k].map(|v| v.max(0.0).round() as usize);
                for w in split_words(a, &ink, r) {
                    page.words.push(OcrWord {
                        text: w.text,
                        bbox: w.bbox,
                        conf: r.conf,
                        line: li,
                        block: bi as u32,
                        angle: page.skew_deg + b.angle_deg(),
                        font_px: w.font_px,
                        char_x: w.char_x,
                    });
                }
                let n = r.text.chars().filter(|c| !c.is_whitespace()).count();
                lconf = (lconf.0 + r.conf * n as f32, lconf.1 + n);
                texts.push(r.text.as_str());
            }
            let mut text = texts.join(" ");
            if p.homoglyphs {
                text = fix::fix_homoglyphs(&text, 0.6);
            }
            if p.decimal_comma {
                text = fix::fix_decimal_comma(&text);
            }
            if p.homoglyphs || p.decimal_comma {
                // Same tokenisation as split_words: re-apply the fixed tokens to the line's words.
                let fixed: Vec<&str> = text.split(' ').filter(|t| !t.is_empty()).collect();
                let words = &mut page.words[first_word..];
                if fixed.len() == words.len() {
                    for (w, t) in words.iter_mut().zip(fixed) {
                        w.text = t.to_string();
                    }
                }
            }
            let ws = &page.words[first_word..];
            let bbox = ws.iter().fold(rects[line[0]], |a, w| {
                [
                    a[0].min(w.bbox[0]),
                    a[1].min(w.bbox[1]),
                    a[2].max(w.bbox[2]),
                    a[3].max(w.bbox[3]),
                ]
            });
            chars += lconf.1;
            conf_sum += lconf.0;
            page.lines.push(OcrLine {
                text,
                bbox,
                conf: if lconf.1 > 0 {
                    lconf.0 / lconf.1 as f32
                } else {
                    0.0
                },
                block: bi as u32,
            });
        }
    }
    page.mean_conf = if chars > 0 {
        conf_sum / chars as f32
    } else {
        0.0
    };
    Ok(page)
}

#[cfg(test)]
#[path = "pipeline_test.rs"]
mod tests;
