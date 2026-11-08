//! Page orientation check (0/90/180/270), a cascade that costs nothing on upright pages:
//! - 90/270: only when most of the detector box area runs vertically. The engine reads vertical boxes
//!   both ways (`Recognized::flipped`), so the reading direction of the widest boxes is already known;
//!   their vote picks the turn. Short vertical lines without body text (turned column headers of an
//!   upright table) turn the page only if its horizontal boxes read better turned.
//! - 180: only when the page read at 0° is unsure (`UPRIGHT_CONF`); the widest boxes are re-read upside down.

use image::imageops;

use crate::{Image, OcrEngine, OcrError, Recognized, TextBox};

/// Boxes read per orientation probe (the widest ones).
const ORIENT_PROBES: usize = 5;
/// An upside-down reading must beat the upright one by at least this confidence.
const ORIENT_MARGIN: f32 = 0.1;
/// Share of box area that makes a page "vertical" (90/270° candidates); the turned page must have the
/// same share of horizontal area.
pub(crate) const VERTICAL_SHARE: f32 = 0.6;
/// Fewer vertical boxes do not turn a page (a stamp, a side label, a single line).
const MIN_VERTICAL_BOXES: usize = 3;
/// Vertical boxes read for the direction vote.
const VOTE_PROBES: usize = 8;
/// Readings below this confidence do not vote (barcodes, rules, noise).
const VOTE_CONF: f32 = 0.6;
/// The winning direction needs this share of the vote and at least `VOTE_CHARS` chars.
const VOTE_SHARE: f32 = 0.8;
const VOTE_CHARS: usize = 12;
/// At least `BODY_LINES` vote readings of `BODY_CHARS` chars are body text, not turned column headers (the
/// headers of the review matrices are 27-39 chars; 95 % of the sideways eval pages have two longer lines).
const BODY_CHARS: usize = 40;
const BODY_LINES: usize = 2;
/// Horizontal boxes needed to judge the minority of a vertical page.
const MIN_MINORITY: usize = 3;
/// A page read at 0° with a mean box confidence of at least this is upright: no 180° probe.
pub(crate) const UPRIGHT_CONF: f32 = 0.8;

/// The `n` boxes with the longest text edge (ties by index).
fn widest(boxes: &[TextBox], n: usize) -> Vec<TextBox> {
    let mut idx: Vec<usize> = (0..boxes.len()).collect();
    idx.sort_by(|&a, &b| {
        boxes[b]
            .extent()
            .0
            .total_cmp(&boxes[a].extent().0)
            .then(a.cmp(&b))
    });
    idx.into_iter().take(n).map(|i| boxes[i]).collect()
}

/// Mean box confidence (empty readings count as 0).
pub(crate) fn mean_conf(r: &[Recognized]) -> f32 {
    if r.is_empty() {
        0.0
    } else {
        r.iter().map(|r| r.conf).sum::<f32>() / r.len() as f32
    }
}

/// Share of the box area (text edge × line height) in boxes with `is_vertical() == vertical`.
pub(crate) fn area_share(boxes: &[TextBox], vertical: bool) -> f32 {
    let area = |b: &TextBox| {
        let (l, t) = b.extent();
        l * t
    };
    let total: f32 = boxes.iter().map(area).sum();
    let part: f32 = boxes
        .iter()
        .filter(|b| b.is_vertical() == vertical)
        .map(area)
        .sum();
    if total > 0.0 { part / total } else { 0.0 }
}

/// `img` turned clockwise by `rotation` (90, 180 or 270; else a copy).
pub(crate) fn turn(img: &Image, rotation: u32) -> Image {
    match rotation {
        90 => imageops::rotate90(img),
        180 => imageops::rotate180(img),
        270 => imageops::rotate270(img),
        _ => img.clone(),
    }
}

/// `engine.recognize` with the one-result-per-box contract checked (as the page recognition does).
fn read(
    engine: &dyn OcrEngine,
    img: &Image,
    boxes: &[TextBox],
) -> Result<Vec<Recognized>, OcrError> {
    let r = engine.recognize(img, boxes)?;
    if r.len() != boxes.len() {
        return Err(OcrError::Engine(format!(
            "recognizer returned {} result(s) for {} box(es)",
            r.len(),
            boxes.len()
        )));
    }
    Ok(r)
}

fn chars(r: &Recognized) -> usize {
    r.text.chars().filter(|c| !c.is_whitespace()).count()
}

/// Clockwise turn (90 or 270) that makes the mostly vertical text of `boxes` horizontal, or `None` when
/// the page is not vertical, the reading direction is unclear or the page looks upright with turned
/// labels. Text running top-to-bottom was turned clockwise (undo: 270), bottom-to-top counter-clockwise
/// (undo: 90). One recognition of `VOTE_PROBES` boxes, two more without body text.
pub(crate) fn sideways(
    engine: &dyn OcrEngine,
    img: &Image,
    boxes: &[TextBox],
) -> Result<Option<u32>, OcrError> {
    let vertical: Vec<TextBox> = boxes.iter().filter(|b| b.is_vertical()).copied().collect();
    if vertical.len() < MIN_VERTICAL_BOXES || area_share(boxes, true) < VERTICAL_SHARE {
        return Ok(None);
    }
    let probes = widest(&vertical, VOTE_PROBES);
    let reads = read(engine, img, &probes)?;
    // (weight, chars) of top-to-bottom and bottom-to-top readings.
    let (mut down, mut up) = ((0f32, 0usize), (0f32, 0usize));
    for (b, r) in probes.iter().zip(&reads) {
        if r.conf < VOTE_CONF {
            continue;
        }
        let n = chars(r);
        // quad[0] -> quad[1] runs down when the angle is positive (y down); a flipped crop reads it backwards.
        let v = if (b.angle_deg() > 0.0) != r.flipped {
            &mut down
        } else {
            &mut up
        };
        *v = (v.0 + r.conf * n as f32, v.1 + n);
    }
    let total = down.0 + up.0;
    let (win, rotation) = if down.0 >= up.0 {
        (down, 270)
    } else {
        (up, 90)
    };
    if win.1 < VOTE_CHARS || win.0 < VOTE_SHARE * total {
        return Ok(None);
    }
    // Long vertical lines are body text: the page is sideways. Short ones may be turned column headers of
    // an upright table (review K1): then the horizontal minority decides.
    let body = reads
        .iter()
        .filter(|r| r.conf >= VOTE_CONF && chars(r) >= BODY_CHARS)
        .count()
        >= BODY_LINES;
    Ok((body || !upright_minority(engine, img, boxes)?).then_some(rotation))
}

/// The horizontal boxes (the widest `VOTE_PROBES`) read better as they stand than turned by 90°: the sum of
/// `conf(0°) - conf(90/270°)` is positive (cells, row labels, a title of an upright table). On a sideways
/// page these are fragments of turned text that read better turned. Fewer than `MIN_MINORITY` horizontal
/// boxes are no evidence (a stray glyph of a sideways two-column page): `false`.
fn upright_minority(
    engine: &dyn OcrEngine,
    img: &Image,
    boxes: &[TextBox],
) -> Result<bool, OcrError> {
    let horizontal: Vec<TextBox> = boxes.iter().filter(|b| !b.is_vertical()).copied().collect();
    let probes = widest(&horizontal, VOTE_PROBES);
    if probes.len() < MIN_MINORITY {
        return Ok(false);
    }
    // The same region read along y: a vertical box, which the engine reads both ways (90° and 270°).
    let turned: Vec<TextBox> = probes
        .iter()
        .map(|b| {
            let [x0, y0, x1, y1] = b.aabb();
            TextBox {
                quad: [[x0, y1], [x0, y0], [x1, y0], [x1, y1]],
                score: b.score,
            }
        })
        .collect();
    let (now, alt) = (read(engine, img, &probes)?, read(engine, img, &turned)?);
    Ok(now
        .iter()
        .zip(&alt)
        .map(|(a, b)| a.conf - b.conf)
        .sum::<f32>()
        > 0.0)
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

/// The page `img` with its horizontal `boxes` reads clearly better turned by 180°: the widest boxes
/// re-read upside down win by `ORIENT_MARGIN` on at least 3 of 5 with `VOTE_CHARS` between them. Returns the
/// turned raster then.
pub(crate) fn upside_down(
    engine: &dyn OcrEngine,
    img: &Image,
    boxes: &[TextBox],
) -> Result<Option<Image>, OcrError> {
    let probes = widest(boxes, ORIENT_PROBES);
    if probes.is_empty() {
        return Ok(None);
    }
    let base = read(engine, img, &probes)?;
    let turned = imageops::rotate180(img);
    let (w, h) = (img.width() as f32, img.height() as f32);
    let mirrored: Vec<TextBox> = probes.iter().map(|b| turn180(b, w, h)).collect();
    let flipped = read(engine, &turned, &mirrored)?;
    let wins: Vec<&Recognized> = base
        .iter()
        .zip(&flipped)
        .filter(|(a, f)| f.conf >= a.conf + ORIENT_MARGIN)
        .map(|(_, f)| f)
        .collect();
    // Single glyphs (a sparse table of a sideways page) win by chance: the winners need `VOTE_CHARS`.
    let n: usize = wins.iter().map(|f| chars(f)).sum();
    Ok((wins.len() * ORIENT_PROBES >= 3 * probes.len() && n >= VOTE_CHARS).then_some(turned))
}

#[cfg(test)]
#[path = "orient_test.rs"]
mod tests;
