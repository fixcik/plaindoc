//! Page facts and the placement and visibility-facts rules (`offpage`, `tiny`, `invisible`,
//! `alpha`, `clip`, `overlap`, `contrast`).

use pdf_oxide::extractors::{ClipKind, PaintKind};
use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::{Color, TextSpan};
use pdf_oxide::{PdfDocument, StructuredPage};

use super::{
    ALPHA, CLIP, CONTRAST, FULL_PAGE_IMAGE_SHARE, INVISIBLE, MAX_CONTRAST_DIFF, MAX_HIDDEN_ALPHA,
    MAX_HIDDEN_CLIP, MIN_COVERED_SHARE, OCR_IMAGE_SHARE, OFFPAGE, OFFPAGE_TOL, OPAQUE_ALPHA,
    OVERLAP, TINY, TINY_MAX_ADVANCE, TINY_MAX_HEIGHT,
};

/// Facts of one page the span rules need (fetched lazily by [`filter_page_boxes`]).
pub(crate) struct PageFacts {
    pub media: Rect,
    /// The page is a scan (`classify_page`): `tiny` is only flagged there.
    pub scan: bool,
    /// `(spans, chars)` the fork dropped outside CropBox ∩ MediaBox (visibility facts only).
    pub offpage: (usize, usize),
    /// Images covering >= [`OCR_IMAGE_SHARE`] of the page (visibility facts only).
    pub big_images: Vec<Rect>,
    /// Non-text paints of the page in paint order (visibility facts only).
    pub paints: Vec<Paint>,
    /// `(paint_order, box)` of the paints that may hide text under them ([`occluders`]), in
    /// paint order.
    pub occluders: Vec<(u32, Rect)>,
    /// Boxes (PDF coordinates) and texts of the spans [`filter_page_boxes`] removed, for
    /// [`filter_words`].
    pub removed: Vec<(Rect, String)>,
}

/// One non-text paint of the page (the fork's `PaintEvent`, which cannot be built in tests).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Paint {
    pub kind: PaintKind,
    pub bbox: Rect,
    /// Resolved fill colour of a `Rect`/`Path` (`None` for other kinds, a `Pattern` fill or an
    /// unresolved colour space).
    pub color: Option<Color>,
    pub alpha: f32,
    pub masked: bool,
    /// Shape of the clip the paint was drawn under.
    pub clip: ClipKind,
    /// Position in the page's paint order, shared with the spans' `paint_order`.
    pub order: u32,
}

/// Placement rule for a hidden span (`offpage`, `tiny`, the visibility-facts rules `invisible`,
/// `alpha`, `clip`, then `overlap` and `contrast`), `None` when visible.
pub(crate) fn placement_rule(s: &TextSpan, f: &PageFacts) -> Option<usize> {
    let b = &s.bbox;
    if ![b.x, b.y, b.width, b.height].iter().all(|v| v.is_finite()) {
        return None;
    }
    let m = &f.media;
    if b.x > m.x + m.width + OFFPAGE_TOL
        || b.x + b.width < m.x - OFFPAGE_TOL
        || b.y > m.y + m.height + OFFPAGE_TOL
        || b.y + b.height < m.y - OFFPAGE_TOL
    {
        return Some(OFFPAGE);
    }
    let (height, advance) = crate::pdf::hidden_turned::run_extent(s);
    if height <= TINY_MAX_HEIGHT && advance <= TINY_MAX_ADVANCE {
        return Some(TINY);
    }
    if let Some(v) = &s.visibility
        && let Some(rule) = facts_rule(
            v.render_mode,
            v.fill_alpha,
            v.stroke_alpha,
            v.clip_coverage,
            || ocr_layer(s, f),
        )
    {
        return Some(rule);
    }
    let v = s.visibility.as_ref()?;
    // Tr 3/7 paint nothing: whether they are hidden is the `invisible` rule's call
    if !matches!(v.render_mode, 3 | 7) && occluded(v.paint_order, b, f) {
        return Some(OVERLAP);
    }
    // the resolved fill colour (`s.color` reads a named colour space by its component count)
    v.fill_color
        .is_some_and(|c| low_contrast(v.render_mode, v.paint_order, b, c, f))
        .then_some(CONTRAST)
}

/// Visibility-facts rule for a span drawn with text render mode `tr`, opacities `fill`, `stroke`
/// and the share `clip` of its box inside the clip; `ocr_layer` is asked only for invisible text.
pub(super) fn facts_rule(
    tr: u8,
    fill: f32,
    stroke: f32,
    clip: f32,
    ocr_layer: impl FnOnce() -> bool,
) -> Option<usize> {
    // Tr 3 (invisible) and 7 (clip only) paint nothing; an OCR layer of a scan is drawn so
    if matches!(tr, 3 | 7) {
        return (!ocr_layer()).then_some(INVISIBLE);
    }
    // Tr 0/4 fill, 1/5 stroke, 2/6 fill and stroke: every painted part is (almost) transparent
    let clear = |a: f32| a <= MAX_HIDDEN_ALPHA;
    let transparent = match tr % 4 {
        0 => clear(fill),
        1 => clear(stroke),
        _ => clear(fill) && clear(stroke),
    };
    if transparent {
        return Some(ALPHA);
    }
    (tr < 4 && clip <= MAX_HIDDEN_CLIP).then_some(CLIP)
}

/// The span may be the OCR layer of a scan: the page is a scan, the font is GlyphLess
/// (Tesseract/ocrmypdf), or the span touches an image covering a large part of the page (words
/// of a jittered layer may stick out of the image).
pub(super) fn ocr_layer(s: &TextSpan, f: &PageFacts) -> bool {
    let b = &s.bbox;
    f.scan
        || s.font_name.contains("GlyphLess")
        || f.big_images.iter().any(|r| {
            b.x <= r.x + r.width
                && b.x + b.width >= r.x
                && b.y <= r.y + r.height
                && b.y + b.height >= r.y
        })
}

/// Share of `a`'s area inside `b` (0 for an empty `a`).
fn covered_share(a: &Rect, b: &Rect) -> f32 {
    let w = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let h = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    let area = a.width * a.height;
    if w <= 0.0 || h <= 0.0 || area <= 0.0 {
        return 0.0;
    }
    w * h / area
}

/// Paints that may hide text drawn before them: opaque, unmasked rectangles and images with an
/// area, except an image over >= [`FULL_PAGE_IMAGE_SHARE`] of the page (a scan over its OCR text)
/// and a paint under a clip that is not a single rectangle (`Path`, `EvenOdd`: its box
/// over-approximates the clip, e.g. a frame filled through an `outer re inner re W*` ring). The
/// fork reports the widest kind on the clip stack, so a page-wide polygon clip turns `overlap` off
/// for every paint under it (an evasion that only loses a warning while the rule only flags).
/// Filtered once per page, so the per-span search only sees these.
pub(super) fn occluders(paints: &[Paint], media: &Rect) -> Vec<(u32, Rect)> {
    let full_page = FULL_PAGE_IMAGE_SHARE * media.width * media.height;
    paints
        .iter()
        .filter(|p| {
            let area = p.bbox.width * p.bbox.height;
            let kind = match p.kind {
                PaintKind::Rect => true,
                PaintKind::Image => area < full_page,
                _ => false,
            };
            let exact_clip = !matches!(p.clip, ClipKind::Path | ClipKind::EvenOdd);
            kind && exact_clip && !p.masked && p.alpha >= OPAQUE_ALPHA && area > 0.0
        })
        .map(|p| (p.order, p.bbox))
        .collect()
}

/// A span drawn at paint position `order` with box `b` is covered by a later occluder.
pub(super) fn occluded(order: u32, b: &Rect, f: &PageFacts) -> bool {
    let later = f.occluders.partition_point(|(o, _)| *o <= order);
    f.occluders[later..]
        .iter()
        .any(|(_, r)| covered_share(b, r) >= MIN_COVERED_SHARE)
}

/// A span drawn with render mode `tr` at paint position `order` with box `b` in fill colour `c`
/// is about the colour of what is under it: the last earlier paint touching the box must be an
/// opaque, unmasked `Rect`/`Path` fill covering the box (anything else under it, such as an image,
/// a shading, a pattern or a smaller shape, is an unknown background), or nothing, which is a
/// white page. Only fill-only modes (Tr 0/4): the facts carry no stroke colour.
pub(super) fn low_contrast(tr: u8, order: u32, b: &Rect, c: Color, f: &PageFacts) -> bool {
    if !matches!(tr, 0 | 4) {
        return false;
    }
    let earlier = f.paints.partition_point(|p| p.order < order);
    let under = f.paints[..earlier]
        .iter()
        .rev()
        .find(|p| covered_share(b, &p.bbox) > 0.0);
    let bg = match under {
        None => Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
        },
        Some(p) => match (p.kind, p.color) {
            (PaintKind::Rect | PaintKind::Path, Some(bg))
                if !p.masked
                    && p.alpha >= OPAQUE_ALPHA
                    && covered_share(b, &p.bbox) >= MIN_COVERED_SHARE =>
            {
                bg
            }
            _ => return false,
        },
    };
    [(c.r, bg.r), (c.g, bg.g), (c.b, bg.b)]
        .iter()
        .all(|(t, u)| (t - u).abs() <= MAX_CONTRAST_DIFF)
}

/// Page facts from the document: the media box, whether the page is a scan (from the caller's
/// `classify_page`), and the fork's page visibility facts when they are on.
pub(crate) fn page_facts(
    doc: &PdfDocument,
    p: usize,
    page: &StructuredPage,
    scan: bool,
) -> PageFacts {
    // Facts of the page's last reading-order extraction (stage A extracts the used page last).
    let vis = doc.page_visibility(p);
    let offpage = vis
        .as_ref()
        .map_or((0, 0), |v| (v.offpage_spans, v.offpage_chars));
    let paints: Vec<Paint> = vis.map_or_else(Vec::new, |v| {
        v.paints
            .iter()
            .map(|e| Paint {
                kind: e.kind,
                bbox: e.bbox,
                color: e.color,
                alpha: e.alpha,
                masked: e.masked,
                clip: e.clip_kind,
                order: e.paint_order,
            })
            .collect()
    });
    let media = doc
        .get_page_media_box(p)
        .map(|(x0, y0, x1, y1)| Rect::new(x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs()))
        .unwrap_or(Rect::new(0.0, 0.0, page.page_width, page.page_height));
    let min_area = OCR_IMAGE_SHARE * media.width * media.height;
    let big_images = paints
        .iter()
        .filter(|e| matches!(e.kind, PaintKind::Image) && e.bbox.width * e.bbox.height >= min_area)
        .map(|e| e.bbox)
        .collect();
    let occluders = occluders(&paints, &media);
    PageFacts {
        media,
        scan,
        offpage,
        big_images,
        paints,
        occluders,
        removed: Vec::new(),
    }
}
