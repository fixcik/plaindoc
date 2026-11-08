//! Vector list bullets (SP8b S1): a bullet drawn as a small filled shape (a CSS `::before` dash or
//! dot printed by a browser) instead of a glyph. Such a shape just left of a line marks it as a
//! list item for `super::lists`.

use pdf_oxide::PdfDocument;
use pdf_oxide::geometry::Rect;

use super::headings::StyledLine;

/// Side limits of a bullet shape, in points.
const MIN_SIDE: f32 = 0.3;
const MAX_SIDE: f32 = 8.0;
/// A page with more small shapes than this (glyph outlines, patterns) gets no vector bullets.
const MAX_BOXES: usize = 2_000;
/// A bullet's sides are at most this many font sizes of its line.
const SIDE_FS: f32 = 0.8;
/// The gap between a bullet and its line's text is at most this many font sizes.
const GAP_FS: f32 = 1.0;

/// A small filled shape: its box and a key of its fill colour and size (bullets of one list share
/// it; the swatches of a chart legend differ in colour).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bullet {
    pub rect: Rect,
    pub key: u64,
}

/// A page content stream longer than this is not read for paths (bullets are an extra; the
/// table adapter has its own limits).
const MAX_CONTENT_BYTES: usize = 1 << 20;
/// A page with more paths than this gets no vector bullets.
const MAX_PATHS: usize = 50_000;

/// Small filled shapes of the page (PDF user space, as the spans), or nothing when there are too
/// many, the content stream is over [`MAX_CONTENT_BYTES`] or the paths cannot be read.
pub(crate) fn bullet_boxes(doc: &PdfDocument, page: usize) -> Vec<Bullet> {
    bullet_boxes_within(doc, page, MAX_CONTENT_BYTES)
}

fn bullet_boxes_within(doc: &PdfDocument, page: usize, max_bytes: usize) -> Vec<Bullet> {
    // the size check comes before the path parser builds anything
    if doc
        .get_page_content_data_shared(page)
        .map_or(true, |d| d.len() > max_bytes)
    {
        return Vec::new();
    }
    let Ok(paths) = doc.extract_paths_shared(page) else {
        return Vec::new();
    };
    if paths.len() > MAX_PATHS {
        return Vec::new();
    }
    let side = |v: f32| (MIN_SIDE..=MAX_SIDE).contains(&v);
    let boxes: Vec<Bullet> = paths
        .iter()
        .filter_map(|p| Some((p.fill_color?, p.rendered_bbox())))
        .filter(|(_, r)| side(r.width) && side(r.height))
        .map(|(c, rect)| {
            let q = |v: f32, k: f32| (v * k).round().clamp(0.0, 255.0) as u64;
            Bullet {
                rect,
                key: q(c.r, 255.0)
                    | q(c.g, 255.0) << 8
                    | q(c.b, 255.0) << 16
                    | q(rect.width, 2.0) << 24
                    | q(rect.height, 2.0) << 32,
            }
        })
        .collect();
    if boxes.len() > MAX_BOXES {
        Vec::new()
    } else {
        boxes
    }
}

/// Per line: the left x and key of a bullet shape ending left of the line's text within
/// [`GAP_FS`] font sizes, with its centre inside the line's height and its sides within
/// [`SIDE_FS`] font sizes.
pub(crate) fn vector_marks(lines: &[StyledLine], boxes: &[Bullet]) -> Vec<Option<(f32, u64)>> {
    lines
        .iter()
        .map(|l| {
            let s = l.size.max(1.0);
            boxes
                .iter()
                .filter(|b| {
                    let b = &b.rect;
                    let (right, yc) = (b.x + b.width, b.y + b.height / 2.0);
                    b.width <= SIDE_FS * s
                        && b.height <= SIDE_FS * s
                        && right <= l.bbox.x + 0.1
                        && l.bbox.x - right <= GAP_FS * s
                        && yc >= l.bbox.y
                        && yc <= l.bbox.y + l.bbox.height
                })
                .map(|b| (b.rect.x, b.key))
                .max_by(|a, b| a.0.total_cmp(&b.0))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(x: f32, y: f32) -> StyledLine {
        StyledLine {
            text: "item".into(),
            size: 10.0,
            bold: false,
            font: String::new(),
            color: (0, 0, 0),
            bbox: Rect::new(x, y, 100.0, 12.0),
            gap: 0.0,
            words: None,
        }
    }

    fn b(x: f32, y: f32, w: f32, h: f32) -> Bullet {
        Bullet {
            rect: Rect::new(x, y, w, h),
            key: 7,
        }
    }

    #[test]
    fn a_small_box_left_of_a_line_is_its_bullet() {
        let lines = [line(82.0, 700.0), line(82.0, 686.0), line(72.0, 672.0)];
        let boxes = [
            b(72.0, 705.0, 4.4, 0.7), // dash before line 0
            b(50.0, 690.0, 3.0, 3.0), // too far from line 1
            b(70.0, 676.0, 3.0, 3.0), // overlaps line 2's text
            b(74.0, 640.0, 3.0, 3.0), // below every line
        ];
        assert_eq!(vector_marks(&lines, &boxes), [Some((72.0, 7)), None, None]);
    }

    #[test]
    fn a_long_content_stream_is_not_read_for_bullets() {
        let pdf = crate::pdf::tests::raw_pdf("0 0 612 792", "0 0 0 rg 80 713 4 1 re f\n");
        let doc = PdfDocument::from_bytes(pdf).unwrap();
        assert_eq!(bullet_boxes(&doc, 0).len(), 1);
        assert!(bullet_boxes_within(&doc, 0, 10).is_empty());
    }

    #[test]
    fn a_box_too_big_for_the_font_is_not_a_bullet() {
        let lines = [line(82.0, 700.0)];
        assert_eq!(vector_marks(&lines, &[b(70.0, 701.0, 9.0, 9.0)]), [None]);
    }
}
