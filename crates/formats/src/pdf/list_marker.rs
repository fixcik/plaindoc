//! List markers at the start of a PDF line or paragraph (SP8b S1) and the paragraph cuts that
//! number / letter sequences add (`sequence_cuts`). `headings::list_marker` (3b paragraph breaks)
//! is a separate, looser reading of the same markers (see docs/ROADMAP.md).

use super::headings::StyledLine;

/// A marker deeper than the previous one by more than this many font sizes opens a nested list;
/// markers within it are at the same x.
pub(crate) const LEVEL_FS: f32 = 0.8;

/// Bullets dropped from the item text.
const SYMBOLS: &str = "•●▪◦►▸‣∙";
/// Check boxes and radio buttons: a list marker that stays in the item text (`■` checked, `□`
/// not), so a form keeps which option is marked.
const BOXES: &str = "■□○";
const LATIN: &str = "abcdefghijklmnopqrstuvwxyz";
const CYRILLIC: &str = "абвгдежзийклмнопрстуфхцчшщъыьэюя";
/// GOST 2.105 enumeration letters: without ё, з, й, о, ч, ъ, ы, ь (`ж)` is followed by `и)`).
const GOST: &str = "абвгдежиклмнпрстуфхцшщэюя";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A bullet symbol.
    Symbol,
    /// A check box or radio button glyph (kept in the text).
    Box,
    /// A vector bullet with its colour and size key.
    Vector(u64),
    /// `–`, `—`, `-`, `*` or `·`.
    Dash,
    /// `N.` / `N)`.
    Number(u32),
    /// `a)`, `а.`: one lower-case Latin or Cyrillic letter.
    Letter(char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Marker {
    pub kind: Kind,
    /// `.` or `)` for numbers and letters.
    pub sep: char,
    /// Bytes of the marker and the whitespace after it at the start of the text.
    pub len: usize,
}

impl Marker {
    pub(crate) fn class(&self) -> u8 {
        match self.kind {
            Kind::Symbol | Kind::Dash | Kind::Box => 0,
            Kind::Number(_) => 1,
            Kind::Letter(c) if c.is_ascii() => 2,
            Kind::Letter(_) => 3,
            Kind::Vector(_) => 4,
        }
    }

    /// How far `next` is from `self` in a sequence (1 = the next number or letter, 2 = one
    /// skipped), with the same separator; letters count in the full alphabet or in [`GOST`].
    pub(crate) fn step(&self, next: &Marker) -> Option<u32> {
        if self.sep != next.sep {
            return None;
        }
        match (self.kind, next.kind) {
            (Kind::Number(a), Kind::Number(b)) => b.checked_sub(a),
            (Kind::Letter(a), Kind::Letter(b)) => [LATIN, CYRILLIC, GOST]
                .iter()
                .filter_map(|abc| {
                    let pos = |c| abc.chars().position(|x| x == c);
                    pos(b)?.checked_sub(pos(a)?).map(|d| d as u32)
                })
                .min(),
            _ => None,
        }
    }

    /// `next` follows `self` in a sequence, at most one number or letter skipped.
    pub(crate) fn precedes(&self, next: &Marker) -> bool {
        self.step(next).is_some_and(|s| (1..=2).contains(&s))
    }
}

/// The list marker at the start of `text` (leading whitespace allowed), with text after it.
pub(crate) fn marker(text: &str) -> Option<Marker> {
    let lead = text.len() - text.trim_start().len();
    let t = &text[lead..];
    let first = t.chars().next()?;
    // whitespace after the marker, when text follows (and the whitespace is there if needed)
    let after = |rest: &str, need_space: bool| -> Option<usize> {
        let body = rest.trim_start();
        let space = rest.len() - body.len();
        (!body.is_empty() && (space > 0 || !need_space)).then_some(space)
    };
    let mk = |kind, sep, n: usize, space: usize| Marker {
        kind,
        sep,
        len: lead + n + space,
    };
    let w = first.len_utf8();
    let glyph = |c: char| SYMBOLS.contains(c) || BOXES.contains(c);
    if glyph(first) {
        // `•Text`: the bullet was a separate span with no gap; `••` is not a marker
        if t[w..].starts_with(glyph) {
            return None;
        }
        let kind = if SYMBOLS.contains(first) {
            Kind::Symbol
        } else {
            Kind::Box
        };
        return after(&t[w..], false).map(|s| mk(kind, ' ', w, s));
    }
    if "·–—-*".contains(first) {
        // `·` also separates words inline (`Go · Rust`): a wrapped line may start with it, so it
        // needs a run like a dash
        return after(&t[w..], true).map(|s| mk(Kind::Dash, ' ', w, s));
    }
    let n = t.find(['.', ')'])?;
    let tok = &t[..n];
    let sep = if t[n..].starts_with('.') { '.' } else { ')' };
    let kind = if (1..=3).contains(&tok.len()) && tok.bytes().all(|b| b.is_ascii_digit()) {
        Kind::Number(tok.parse().ok()?)
    } else {
        let mut cs = tok.chars();
        let (Some(c), None) = (cs.next(), cs.next()) else {
            return None;
        };
        if !(LATIN.contains(c) || CYRILLIC.contains(c)) {
            return None;
        }
        Kind::Letter(c)
    };
    after(&t[n + 1..], true).map(|s| mk(kind, sep, n + 1, s))
}

/// `cuts` (sorted line indices that start a paragraph) plus the lines whose number or letter
/// marker follows the marker of an earlier line at the same x (`2.` after `1.`, `б)` after `а)`):
/// one-line items that do not end a sentence are not cut by `split_points`; plus the lines with a
/// vector bullet (`vector`, see `lists::group`) that another line shares.
pub(crate) fn sequence_cuts(
    lines: &[StyledLine],
    cuts: &[usize],
    vector: &[Option<(f32, u64)>],
) -> Vec<usize> {
    let marks: Vec<Option<Marker>> = lines
        .iter()
        .map(|l| marker(&l.text).filter(|m| m.class() > 0))
        .collect();
    let mut out = cuts.to_vec();
    for (i, m) in marks.iter().enumerate().skip(1) {
        let Some(m) = m else {
            continue;
        };
        let tol = LEVEL_FS * lines[i].size.max(1.0);
        let prev = (0..i).rev().find_map(|j| {
            marks[j].filter(|p| {
                p.class() == m.class()
                    && p.sep == m.sep
                    && (lines[j].bbox.x - lines[i].bbox.x).abs() <= tol
            })
        });
        if prev.is_some_and(|p| p.precedes(m)) && cuts.binary_search(&i).is_err() {
            out.push(i);
        }
    }
    // a line with a vector bullet starts an item when another line has the same bullet at its x
    for (i, v) in vector.iter().enumerate().skip(1) {
        let Some((x, key)) = *v else {
            continue;
        };
        let tol = LEVEL_FS * lines[i].size.max(1.0);
        let twin = vector
            .iter()
            .enumerate()
            .any(|(j, w)| j != i && w.is_some_and(|(wx, wk)| wk == key && (wx - x).abs() <= tol));
        if twin && marks[i].is_none() && cuts.binary_search(&i).is_err() {
            out.push(i);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}
