//! Region text without the spaces the fork puts between spans that touch at punctuation.
//!
//! The fork's region text joins its spans with a space unless they are a kerned letter pair.
//! Its span merger keeps a glyph in another font as its own span unless both sides of the
//! boundary are letters, so a producer that draws punctuation in another font (Type3 fonts with
//! a few glyphs each, a bracket in another face) gets a span per bracket, hyphen, dot or comma,
//! and the region text reads `( Swift )`, `CAM -тулчейн`, `name @ mail . com`. A space there is
//! dropped when the page draws none: the spans touch on one line (gap under [`TOUCH_GAP_EM`],
//! overlap within [`KERN_OVERLAP_EM`]) and one side of the boundary is neither a letter nor a
//! digit. Letter and digit pairs keep the fork's decision (a width table with one or two values
//! fakes zero gaps between real words).

use pdf_oxide::layout::TextSpan;
use pdf_oxide::{StructuredPage, StructuredRegion};

/// Spans closer than this fraction of the smaller font size touch: half of the narrowest word
/// space of common faces (about 0.2 em). Touching runs measure within ±0.03 em, an italic
/// overhang up to 0.09 em; a tab stop with its tab glyph dropped (`Figure 1.2.⇥Title`) and dot
/// leaders start from about 0.1 em, so the bound sits below them and under the 0.15 em word glue
/// of the fork and of [`super::layout::group_lines`].
const TOUCH_GAP_EM: f32 = 0.1;
/// Kerning pulls a glyph back by up to this fraction of the font size (the fork's kerning bound);
/// a larger overlap, or one over half of the narrower span, is a duplicate or an overprint.
const KERN_OVERLAP_EM: f32 = 0.15;

/// `next` touches `prev` on the same upright line with no space drawn between them, and one side
/// of the boundary is punctuation or a symbol.
pub(crate) fn tight(prev: &TextSpan, next: &TextSpan) -> bool {
    let upright = |s: &TextSpan| s.rotation_degrees == 0.0 && !s.turned_bbox;
    let (Some(a), Some(b)) = (prev.text.chars().next_back(), next.text.chars().next()) else {
        return false;
    };
    if a.is_whitespace()
        || b.is_whitespace()
        || (a.is_alphanumeric() && b.is_alphanumeric())
        || !upright(prev)
        || !upright(next)
    {
        return false;
    }
    let size = prev.font_size.min(next.font_size);
    let yc = |s: &TextSpan| s.bbox.y + s.bbox.height / 2.0;
    let gap = next.bbox.x - (prev.bbox.x + prev.bbox.width);
    let floor = -(KERN_OVERLAP_EM * size).min(0.5 * prev.bbox.width.min(next.bbox.width));
    size.is_finite()
        && size > 0.0
        && (yc(prev) - yc(next)).abs() < 0.5 * prev.bbox.height.min(next.bbox.height)
        && gap >= floor
        && gap < TOUCH_GAP_EM * size
}

/// Drops the spaces between [`tight`] spans from `r.text`. Only spaces the fork put between two
/// spans are dropped; the text is kept as is when it is not the trimmed span texts joined by
/// nothing or one space.
pub(crate) fn tighten(r: &mut StructuredRegion) {
    let mut out = String::with_capacity(r.text.len());
    let mut rest = r.text.as_str();
    let mut prev: Option<&TextSpan> = None;
    for s in &r.spans {
        let t = s.text.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(p) = prev
            && let Some(r) = rest.strip_prefix(' ')
        {
            rest = r;
            if !tight(p, s) {
                out.push(' ');
            }
        }
        let Some(r) = rest.strip_prefix(t) else {
            return;
        };
        out.push_str(t);
        rest = r;
        prev = Some(s);
    }
    if rest.is_empty() {
        r.text = out;
    }
}

/// [`tighten`] over every region of the page.
pub(crate) fn tighten_page(page: &mut StructuredPage) {
    page.regions.iter_mut().for_each(tighten);
}

#[cfg(test)]
#[path = "spacing_test.rs"]
mod tests;
