//! Forms in DOCX: fill-in lines drawn as borders.
//!
//! A field is a line to write on. Word forms draw it as the top border of the next paragraph (the
//! small hint under the line, or an empty 1 pt paragraph); the paragraph above the line then
//! gets [`super::inline::FIELD`] (see `inline::para_text`).

use office_oxide::ir::{BorderLine, BorderStyle, Element, InlineContent, Paragraph};

fn visible(b: Option<&BorderLine>) -> bool {
    b.is_some_and(|b| b.style != BorderStyle::None)
}

/// Font sizes (half-points) of the spans with visible text; `None` for a span without one.
fn sizes(p: &Paragraph) -> impl Iterator<Item = Option<u32>> + '_ {
    p.content.iter().filter_map(|c| match c {
        InlineContent::Text(s) if !s.text.trim().is_empty() => Some(s.font_size_half_pt),
        _ => None,
    })
}

/// `a` is the fill line of a field: `next` is a paragraph with a line above it (top border, no
/// side borders: not a box; not the same borders as `a`: one group) that is empty or set smaller
/// than `a` (a hint), and `a` leaves room for the answer (ends with a space or holds a tab; an
/// empty `a` only above a hint).
pub(crate) fn is_field(a: &Paragraph, next: Option<&Element>) -> bool {
    let Some(Element::Paragraph(b)) = next else {
        return false;
    };
    let Some(bd) = &b.border else { return false };
    if !visible(bd.top.as_ref()) || visible(bd.left.as_ref()) || visible(bd.right.as_ref()) {
        return false;
    }
    // A group of paragraphs with the same borders has one line above the first of them.
    if a.border.as_ref() == Some(bd) {
        return false;
    }
    let text = office_oxide::ir::inline_to_text(&a.content);
    let empty = text.trim().is_empty();
    let room = empty || text.ends_with([' ', '\t', '\u{a0}']) || text.contains('\t');
    if !room {
        return false;
    }
    let hint: Vec<Option<u32>> = sizes(b).collect();
    if hint.is_empty() {
        // A line under an empty paragraph is a rule, not a field.
        return !empty;
    }
    let Some(hint_max) = hint
        .iter()
        .copied()
        .collect::<Option<Vec<u32>>>()
        .and_then(|v| v.into_iter().max())
    else {
        return false;
    };
    match sizes(a).collect::<Option<Vec<u32>>>() {
        Some(v) if !v.is_empty() => v.into_iter().min().is_some_and(|m| hint_max < m),
        // No sized text in `a`: a hint of 10 pt or less.
        _ => hint_max <= 20,
    }
}
