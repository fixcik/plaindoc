//! Unicode content rules: tag characters, bidi overrides and zero-width interleaves.

use super::{MIN_ZW_INTERLEAVE, UNI_BIDI, UNI_ZW};

pub(crate) fn non_ws(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

pub(super) fn is_tag(c: char) -> bool {
    ('\u{E0000}'..='\u{E007F}').contains(&c)
}

/// LRO and RLO: the extracted order is not the visible one. Embeddings (RLE/LRE/PDF), marks
/// (RLM/LRM) and isolates (LRI/RLI/FSI/PDI) are legitimate in RTL text and kept.
fn is_bidi_override(c: char) -> bool {
    matches!(c, '\u{202D}' | '\u{202E}')
}

fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200B}' | '\u{2060}' | '\u{FEFF}')
}

/// Longest run of single ASCII letters separated by zero-width characters ("P\u{200B}W\u{200B}N" = 3).
/// Publishers put ZWSP into URLs and DNA sequences as break hints ("creat\u{200B}iveco"), which
/// never gives runs of single letters.
pub(super) fn zw_interleave_run(t: &str) -> usize {
    let v: Vec<char> = t.chars().collect();
    let (mut best, mut run) = (0usize, 0usize);
    let mut i = 0;
    while i < v.len() {
        let single = v[i].is_ascii_alphabetic()
            && (i == 0 || is_zero_width(v[i - 1]) || !v[i - 1].is_ascii_alphabetic())
            && v.get(i + 1).is_some_and(|&c| is_zero_width(c));
        if single && (run == 0 || is_zero_width(v[i - 1])) {
            run += 1;
            best = best.max(run);
            i += 2;
            continue;
        }
        if v[i].is_ascii_alphabetic() && run > 0 && i > 0 && is_zero_width(v[i - 1]) {
            // last letter of the run, followed by no zero-width character
            best = best.max(run + 1);
        }
        run = 0;
        i += 1;
    }
    best
}

/// Content rule (`uni_bidi`, `uni_zw`) for a span, checked also when a placement rule only flags
/// it: an RLO payload inside a default-off layer is still removed in `Strip`.
pub(crate) fn content_rule(text: &str) -> Option<usize> {
    if text.chars().any(is_bidi_override) {
        return Some(UNI_BIDI); // RLO + reversed payload: the visible order is not the extracted one
    }
    if zw_interleave_run(text) >= MIN_ZW_INTERLEAVE {
        return Some(UNI_ZW);
    }
    None
}
