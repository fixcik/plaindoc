//! Cutting removed spans out of the table words, and the per-word rules.

use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::TextSpan;

use super::{
    HiddenCounts, PageFacts, TINY, TINY_MAX_ADVANCE, TINY_MAX_HEIGHT, UNI_TAGS, content_rule,
    is_tag, non_ws, removes,
};
use crate::tables::InputWord;

/// The box of a removed span for [`filter_words`]. The fork's box runs from the first pen
/// position to the last, so a move back (a `TJ` jump, a negative `Tw`) leaves glyphs outside it
/// or gives it a negative width; then it is widened to the pen positions of all chars
/// (`char_widths` = the pen advance after each char).
pub(crate) fn glyph_box(s: &TextSpan) -> Rect {
    let b = s.bbox;
    if !s.char_widths.iter().any(|&w| w < 0.0) {
        return b;
    }
    let (mut x, mut lo, mut hi) = (b.x, b.x.min(b.x + b.width), b.x.max(b.x + b.width));
    for &w in &s.char_widths {
        lo = lo.min(x);
        hi = hi.max(x + w.max(0.0));
        x += w;
    }
    Rect::new(lo, b.y, hi - lo, b.height)
}

/// Chars left out when a word is compared with the text of a removed span: whitespace, controls
/// and the default-ignorable and format chars (zero-width, bidi controls, tags, variation
/// selectors, soft hyphen, ...). Extraction drops or keeps them differently in spans and words.
fn ignorable(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(c,
            '\u{AD}' | '\u{34F}' | '\u{61C}' | '\u{600}'..='\u{605}' | '\u{6DD}' | '\u{70F}'
            | '\u{8E2}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}' | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}'
            | '\u{3164}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}' | '\u{FFA0}' | '\u{FFF0}'..='\u{FFFB}'
            | '\u{110BD}' | '\u{110CD}' | '\u{13430}'..='\u{1343F}' | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}' | '\u{E0000}'..='\u{E0FFF}')
}

/// A removed span in the words' coordinates with its text without [`ignorable`] chars: the
/// whole, the whitespace tokens, and the chars not cut yet (each is cut at most once).
struct Cut {
    r: crate::tables::Rect,
    text: Vec<char>,
    tokens: Vec<Vec<char>>,
    left: Vec<char>,
}

impl Cut {
    /// Takes one `c` from the chars not cut yet.
    fn take(&mut self, c: char) -> bool {
        let Some(i) = self.left.iter().position(|&l| l == c) else {
            return false;
        };
        self.left.swap_remove(i);
        true
    }
}

/// Removed boxes taller than this are checked against every word; the others are found by their
/// top, at most this far above the word's.
const TALL_CUT: f64 = 24.0;

/// Cuts the glyphs of the removed spans (`removed`, PDF coordinates) out of `words` and drops
/// the words with nothing left ([`cut_word`]). Returns the number of (word, removed span) pairs
/// looked at.
pub(super) fn cut_removed(
    words: &mut Vec<InputWord>,
    removed: &[(Rect, String)],
    (ox, top_y): (f64, f64),
) -> usize {
    if removed.is_empty() {
        return 0;
    }
    let norm = |t: &str| t.chars().filter(|&c| !ignorable(c)).collect::<Vec<char>>();
    let (mut short, mut tall): (Vec<Cut>, Vec<Cut>) = removed
        .iter()
        .map(|(b, t)| {
            let (x, y, w, h) = (
                f64::from(b.x),
                f64::from(b.y),
                f64::from(b.width),
                f64::from(b.height),
            );
            let text = norm(t);
            Cut {
                r: crate::tables::Rect::new(x - ox, top_y - (y + h), w, h),
                tokens: t
                    .split_whitespace()
                    .map(norm)
                    .filter(|t| !t.is_empty())
                    .collect(),
                left: text.clone(),
                text,
            }
        })
        .filter(|c| {
            [c.r.x, c.r.top, c.r.w, c.r.h].iter().all(|v| v.is_finite()) && !c.text.is_empty()
        })
        .partition(|c| c.r.h <= TALL_CUT);
    short.sort_by(|a, b| a.r.top.total_cmp(&b.r.top));
    let reach = short.iter().map(|c| c.r.h).fold(0.0, f64::max);
    let mut work = 0;
    words.retain_mut(|w| {
        let (wt, wh) = (w.rect.top, w.rect.h);
        let lo = short.partition_point(|c| c.r.top < wt - reach);
        let hi = short.partition_point(|c| c.r.top <= wt + wh).max(lo);
        for c in short[lo..hi].iter_mut().chain(tall.iter_mut()) {
            work += 1;
            if cut_word(w, c) {
                return false;
            }
        }
        true
    });
    work
}

/// Cuts the glyphs of the removed span `c` out of `w`; `true` when nothing of it is left. Only
/// text of the span inside its box is cut, compared without [`ignorable`] chars: the whole word
/// when its centre is in the box and its text is part of the span's; else (one `char_x` per
/// char) each token of the span found as a run of the word with its x-midpoints in the box
/// (cut from the first to the last char of the run); else, for glyphs out of order (a `TJ` jump
/// back, a negative `Tw`), the chars with the x-midpoint in the box that the span has. So a
/// removed span with a large box (a clipped bracket glyph) does not take its neighbours.
fn cut_word(w: &mut InputWord, c: &mut Cut) -> bool {
    let (wt, wh) = (w.rect.top, w.rect.h);
    let r = c.r;
    let (cx, cy) = (w.rect.x + w.rect.w / 2.0, wt + wh / 2.0);
    let in_y = |y: f64| (r.top..=r.top + r.h).contains(&y);
    let in_x = |x: f64| (r.x..=r.x + r.w).contains(&x);
    // same line (either vertical centre inside the other's vertical range), overlapping in x
    if !(in_y(cy) || (wt..=wt + wh).contains(&(r.top + r.h / 2.0)))
        || r.x > w.rect.x + w.rect.w
        || r.x + r.w < w.rect.x
    {
        return false;
    }
    let chars: Vec<char> = w.text.chars().collect();
    // indices of the chars compared
    let kept: Vec<usize> = (0..chars.len()).filter(|&j| !ignorable(chars[j])).collect();
    if kept.is_empty() {
        return false;
    }
    if in_x(cx) && in_y(cy) {
        let word: Vec<char> = kept.iter().map(|&j| chars[j]).collect();
        if c.text.windows(word.len()).any(|t| t == word) {
            word.iter().for_each(|&ch| _ = c.take(ch));
            return true;
        }
    }
    if w.char_x.len() != chars.len() {
        return false;
    }
    let mid_in = |j: usize| in_x((w.char_x[j].0 + w.char_x[j].1) / 2.0);
    let mut cut = vec![false; chars.len()];
    let mut matched = Vec::new();
    for tok in &c.tokens {
        let found = (0..(kept.len() + 1).saturating_sub(tok.len())).find(|&i| {
            tok.iter().enumerate().all(|(k, &t)| {
                let j = kept[i + k];
                !cut[j] && chars[j] == t && mid_in(j)
            })
        });
        if let Some(i) = found {
            cut[kept[i]..=kept[i + tok.len() - 1]].fill(true);
            matched.extend_from_slice(tok);
        }
    }
    let mut any = !matched.is_empty();
    if any {
        matched.into_iter().for_each(|ch| _ = c.take(ch));
    } else {
        for &j in &kept {
            if mid_in(j) && c.take(chars[j]) {
                cut[j] = true;
                any = true;
            }
        }
        if !any {
            return false;
        }
    }
    // the span's ignorable chars before or after the cut ones (an RLO in front of the payload)
    for (j, &ch) in chars.iter().enumerate() {
        if ignorable(ch) && mid_in(j) {
            cut[j] = true;
        }
    }
    let rest: Vec<usize> = (0..chars.len()).filter(|&j| !cut[j]).collect();
    if rest.iter().all(|&j| ignorable(chars[j])) {
        return true;
    }
    w.text = rest.iter().map(|&j| chars[j]).collect();
    w.char_x = rest.iter().map(|&j| w.char_x[j]).collect();
    let l = w.char_x.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
    let rr = w
        .char_x
        .iter()
        .map(|c| c.1)
        .fold(f64::NEG_INFINITY, f64::max);
    w.rect = crate::tables::Rect::new(l, wt, rr - l, wh);
    false
}

/// The per-word rules of [`filter_words`]; `false` when the word is removed.
pub(super) fn keep_word(
    w: &mut InputWord,
    strip: bool,
    facts: &PageFacts,
    counts: &mut HiddenCounts,
) -> bool {
    let text = w.text.trim();
    if text.is_empty() {
        return true;
    }
    let n = non_ws(text);
    if w.rect.h <= f64::from(TINY_MAX_HEIGHT)
        && w.rect.w / n.max(1) as f64 <= f64::from(TINY_MAX_ADVANCE)
    {
        counts.add(TINY, n, removes(TINY, strip, facts));
        if removes(TINY, strip, facts) {
            return false;
        }
    }
    if let Some(rule) = content_rule(text) {
        counts.add(rule, n, strip);
        if strip {
            return false;
        }
    }
    let tags = w.text.chars().filter(|&c| is_tag(c)).count();
    if tags > 0 {
        counts.add(UNI_TAGS, tags, strip);
        if strip {
            if w.char_x.len() == w.text.chars().count() {
                (w.text, w.char_x) = w
                    .text
                    .chars()
                    .zip(w.char_x.iter().copied())
                    .filter(|(c, _)| !is_tag(*c))
                    .unzip();
            } else {
                w.text = w.text.chars().filter(|&c| !is_tag(c)).collect();
                w.char_x.clear();
            }
        }
    }
    true
}
