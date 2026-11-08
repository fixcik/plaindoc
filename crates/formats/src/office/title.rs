//! Title of a DOCX without heading styles: the first one or two paragraphs set bold, centred
//! and larger than the body text (form and letter titles: "ОТЗЫВ" / "о результатах ...").

use std::collections::HashMap;

use super::inline::{NoteLabels, para_text};
use office_oxide::ir::{Element, InlineContent, Paragraph, ParagraphAlignment, inline_to_text};

/// Longest title paragraph, in characters.
const MAX_TITLE_CHARS: usize = 120;
/// Most title paragraphs merged into one heading.
const MAX_TITLE_PARAS: usize = 2;

/// Font size (half-points) of most body characters; `None` if unknown for most of them.
fn body_size(els: &[Element]) -> Option<u32> {
    let mut count: HashMap<Option<u32>, usize> = HashMap::new();
    for e in els {
        if let Element::Paragraph(p) = e {
            for c in &p.content {
                if let InlineContent::Text(s) = c {
                    *count.entry(s.font_size_half_pt).or_default() += s.text.chars().count();
                }
            }
        }
    }
    count
        .into_iter()
        .max_by_key(|&(k, n)| (n, k))
        .and_then(|(k, _)| k)
}

fn is_title(p: &Paragraph, body: u32) -> bool {
    let text = inline_to_text(&p.content);
    let mut spans = p.content.iter().filter_map(|c| match c {
        InlineContent::Text(s) if !s.text.trim().is_empty() => Some(s),
        _ => None,
    });
    p.alignment == Some(ParagraphAlignment::Center)
        && text.trim().chars().count() <= MAX_TITLE_CHARS
        && spans.clone().next().is_some()
        && spans.all(|s| s.bold && s.font_size_half_pt.is_some_and(|z| z > body))
}

/// `(elements taken, heading text)` of the title at the start of `els` (empty paragraphs before
/// it are taken too); the caller checks that the document has no headings.
pub(crate) fn leading_title(els: &[Element], notes: &mut NoteLabels) -> Option<(usize, String)> {
    let body = body_size(els)?;
    let mut parts = Vec::new();
    let mut taken = 0;
    for e in els {
        let Element::Paragraph(p) = e else { break };
        let text = inline_to_text(&p.content);
        if text.trim().is_empty() && parts.is_empty() {
            taken += 1;
            continue;
        }
        if parts.len() == MAX_TITLE_PARAS || !is_title(p, body) {
            break;
        }
        let text = para_text(&p.content, &p.tabs, false, Some(notes));
        parts.push(text.split_whitespace().collect::<Vec<_>>().join(" "));
        taken += 1;
    }
    (!parts.is_empty()).then(|| (taken, parts.join(" ")))
}
