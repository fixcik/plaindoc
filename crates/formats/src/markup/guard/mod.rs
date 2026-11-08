//! Pre-pass against html5ever's superlinear cases; a timeout only detaches the
//! thread, so such input is rejected before tree building.
//! - Depth: the tree builder is quadratic in the depth of open elements
//!   (200 000 nested `<div>` take minutes).
//! - Reconstruction: unclosed formatting elements (`<b id=1>`...`<b id=3000>`)
//!   are re-created in every following block, which is rows x elements.
//!
//! A linear tokenizer pass models the stack of open elements with the tree
//! builder's implied closes and scopes (approximately, never popping more
//! than the tree builder) and counts reconstructed formatting elements.

use std::cell::RefCell;
use std::collections::HashMap;

mod attrs;
use attrs::max_attrs;

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use plaindoc_core::{ParseError, Result};

/// Most open elements the tree builder is allowed to see (20 000 nested `div`s take ~290 ms).
pub(crate) const MAX_OPEN_ELEMENTS: usize = 16_384;
/// Most formatting elements the tree builder may re-create in total, or the input
/// length in bytes if larger (R18: the counting sink already bounds the real work).
pub(crate) const MAX_RECONSTRUCTED: usize = 1_000_000;
/// Formatting-list scan work allowed per input byte and on top (R14): every formatting
/// start tag compares itself with each active entry, cloning and sorting attributes.
const FMT_WORK_PER_BYTE: u64 = 16;
const FMT_WORK_BASE: u64 = 50_000_000;
/// Most attributes on one element: the tokenizer checks each against all earlier ones.
pub(crate) const MAX_ATTRS: usize = 8192;

/// Never stay open (void, or not part of the depth).
const NOT_PUSHED: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr", "param", "keygen", "frame", "basefont", "bgsound", "html", "head", "body", "colgroup",
    "rb", "rt", "rtc", "rp",
];
/// Zero-weight scope markers: a later sibling closes them and what was opened inside (I1).
const MARKERS: &[&str] = &[
    "p", "li", "dd", "dt", "option", "optgroup", "tr", "td", "th", "thead", "tbody", "tfoot",
    "caption",
];
/// Start tags whose contents the tokenizer reads as text up to the matching end tag.
const RAW_TEXT: &[(&str, RawKind)] = &[
    ("script", RawKind::ScriptData),
    ("style", RawKind::Rawtext),
    ("xmp", RawKind::Rawtext),
    ("iframe", RawKind::Rawtext),
    ("noembed", RawKind::Rawtext),
    ("noframes", RawKind::Rawtext),
    ("noscript", RawKind::Rawtext),
    ("title", RawKind::Rcdata),
    ("textarea", RawKind::Rcdata),
];
const FORMATTING: &[&str] = &[
    "a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small", "strike", "strong", "tt", "u",
];
/// Start tags that close an open `p` in button scope.
const CLOSES_P: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "center",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "header",
    "hgroup",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "search",
    "section",
    "summary",
    "ul",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "pre",
    "listing",
    "form",
    "hr",
    "xmp",
    "li",
    "dd",
    "dt",
    "plaintext",
];
/// Start tags after which the tree builder does not reconstruct formatting elements.
const NO_RECONSTRUCT: &[&str] = &[
    "table", "html", "head", "body", "caption", "col", "colgroup", "tbody", "td", "tfoot", "th",
    "thead", "tr", "base", "link", "meta", "style", "script", "title", "template", "frameset",
];
const SPECIAL: &[&str] = &[
    "address",
    "applet",
    "article",
    "aside",
    "blockquote",
    "button",
    "caption",
    "center",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "iframe",
    "li",
    "listing",
    "main",
    "marquee",
    "menu",
    "nav",
    "noembed",
    "noframes",
    "object",
    "ol",
    "p",
    "plaintext",
    "pre",
    "search",
    "section",
    "select",
    "summary",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
    "xmp",
];
const DEFAULT_SCOPE: &[&str] = &[
    "applet", "caption", "html", "table", "td", "th", "marquee", "object", "template",
];

// Scope kinds: index into `Entry::near`.
const SPECIAL_K: usize = 0; // any special element (ends "any other end tag" and adoption)
const LI_STOP: usize = 1; // special except address, div, p (ends the li/dd/dt search)
const SCOPE: usize = 2; // default scope
const BUTTON: usize = 3; // button scope (for p)
const TABLE_K: usize = 4; // table scope

fn kinds(name: &str) -> [bool; 5] {
    let special = SPECIAL.contains(&name);
    let scope = DEFAULT_SCOPE.contains(&name);
    [
        special,
        special && !matches!(name, "address" | "div" | "p"),
        scope,
        scope || name == "button",
        matches!(name, "html" | "table" | "template"),
    ]
}

struct Entry {
    /// Empty for a reconstructed formatting element (never matched by an end tag).
    name: String,
    weight: bool,
    fmt: bool,
    /// `td`/`th`/`caption`: owns a level of active formatting elements.
    cell: bool,
    /// Per scope kind: 1 + index of the nearest boundary at or below this entry (0: none).
    near: [usize; 5],
}

/// Active formatting elements after the last cell marker, as counters.
#[derive(Default)]
struct Level {
    active: usize,
    on_stack: usize,
    /// Attributes of the active entries.
    attrs: usize,
    by_key: HashMap<String, usize>,
    keys: HashMap<String, Vec<(String, usize)>>,
}

#[derive(Default)]
struct State {
    stack: Vec<Entry>,
    pos: HashMap<String, Vec<usize>>,
    /// Never empty: the base level, then one per open cell.
    levels: Vec<Level>,
    depth: usize,
    max: usize,
    reconstructed: usize,
    /// `max(MAX_RECONSTRUCTED, input length)`.
    rec_cap: usize,
    /// Formatting-list scan work and its cap.
    fmt_work: u64,
    fmt_cap: u64,
}

impl State {
    fn level(&mut self) -> &mut Level {
        self.levels.last_mut().expect("level")
    }

    fn push(&mut self, name: &str, fmt: bool) {
        let i = self.stack.len();
        let mut near = self.stack.last().map_or([0; 5], |e| e.near);
        if !name.is_empty() {
            for (k, on) in kinds(name).into_iter().enumerate() {
                if on {
                    near[k] = i + 1;
                }
            }
        }
        let weight = name.is_empty() || !MARKERS.contains(&name);
        let cell = matches!(name, "td" | "th" | "caption");
        if !name.is_empty() {
            self.pos.entry(name.to_string()).or_default().push(i);
        }
        if cell {
            self.levels.push(Level::default());
        }
        if weight {
            self.depth += 1;
            self.max = self.max.max(self.depth);
        }
        self.stack.push(Entry {
            name: name.to_string(),
            weight,
            fmt,
            cell,
            near,
        });
    }

    fn pop_to(&mut self, j: usize) {
        while self.stack.len() > j {
            let e = self.stack.pop().expect("entry");
            if let Some(p) = self.pos.get_mut(&e.name) {
                p.pop();
            }
            if e.weight {
                self.depth -= 1;
            }
            if e.fmt {
                let l = self.level();
                l.on_stack = l.on_stack.saturating_sub(1);
            }
            if e.cell {
                self.levels.pop();
            }
        }
    }

    fn last(&self, names: &[&str]) -> Option<usize> {
        names
            .iter()
            .filter_map(|n| self.pos.get(*n).and_then(|p| p.last().copied()))
            .max()
    }

    /// No boundary of scope kind `k` lies above index `j`.
    fn in_scope(&self, j: usize, k: usize) -> bool {
        self.stack.last().is_some_and(|e| j + 1 >= e.near[k])
    }

    fn close(&mut self, names: &[&str], k: usize) {
        if let Some(j) = self.last(names)
            && self.in_scope(j, k)
        {
            self.pop_to(j);
        }
    }

    fn top_is(&self, name: &str) -> bool {
        self.stack.last().is_some_and(|e| e.name == name)
    }

    /// The tree builder re-creates active formatting elements that are not open.
    fn reconstruct(&mut self) {
        let l = self.level();
        let missing = l.active.saturating_sub(l.on_stack);
        l.on_stack = l.active;
        self.reconstructed += missing;
        if self.reconstructed > self.rec_cap {
            return;
        }
        for _ in 0..missing {
            self.push("", true);
        }
    }

    /// A formatting start tag; Noah's Ark keeps at most 3 identical ones.
    fn add_active(&mut self, tag: &Tag) {
        let mut attrs: Vec<String> = tag
            .attrs
            .iter()
            .map(|a| format!("{}={}", a.name.local, a.value))
            .collect();
        attrs.sort();
        let name = tag.name.to_string();
        let key = format!("{name} {}", attrs.join(" "));
        let n = attrs.len();
        let l = self.level();
        // the builder compares the tag with every active entry (both attribute lists)
        let work = (l.active as u64)
            .saturating_mul(1 + n as u64)
            .saturating_add(l.attrs as u64);
        let same = l.by_key.entry(key.clone()).or_default();
        if *same < 3 {
            *same += 1;
            l.active += 1;
            l.attrs += n;
            l.keys.entry(name).or_default().push((key, n));
        }
        l.on_stack = (l.on_stack + 1).min(l.active);
        self.fmt_work = self.fmt_work.saturating_add(work);
    }

    /// Formatting end tag (adoption agency, approximated): pops the element when
    /// no special element is above it, and removes it from the active ones.
    fn end_formatting(&mut self, name: &str) {
        if let Some(j) = self.last(&[name])
            && self.in_scope(j, SPECIAL_K)
        {
            self.pop_to(j);
        }
        let l = self.level();
        if let Some((k, n)) = l.keys.get_mut(name).and_then(Vec::pop) {
            l.active -= 1;
            l.attrs -= n;
            if let Some(c) = l.by_key.get_mut(&k) {
                *c -= 1;
            }
        }
        l.on_stack = l.on_stack.min(l.active);
    }

    fn start(&mut self, tag: &Tag) {
        let name: &str = &tag.name;
        if CLOSES_P.contains(&name) {
            self.close(&["p"], BUTTON);
        }
        match name {
            "li" => self.close(&["li"], LI_STOP),
            "dd" | "dt" => self.close(&["dd", "dt"], LI_STOP),
            "td" | "th" => self.close(&["td", "th"], TABLE_K),
            "tr" => {
                self.close(&["td", "th"], TABLE_K);
                self.close(&["tr"], TABLE_K);
            }
            "tbody" | "thead" | "tfoot" => {
                self.close(&["td", "th"], TABLE_K);
                self.close(&["tr"], TABLE_K);
                self.close(&["tbody", "thead", "tfoot"], TABLE_K);
            }
            "option" | "optgroup" => {
                if self.top_is("option") {
                    let j = self.stack.len() - 1;
                    self.pop_to(j);
                }
                if name == "optgroup" && self.top_is("optgroup") {
                    let j = self.stack.len() - 1;
                    self.pop_to(j);
                }
            }
            "a" | "nobr" if self.level().keys.get(name).is_some_and(|v| !v.is_empty()) => {
                self.end_formatting(name);
            }
            _ => {}
        }
        if !NO_RECONSTRUCT.contains(&name) && !CLOSES_P.contains(&name) {
            self.reconstruct();
        }
        // `<div/>` opens a div in HTML; the slash counts only in svg and math.
        let foreign = self.last(&["svg", "math"]).is_some();
        if NOT_PUSHED.contains(&name) || (tag.self_closing && foreign) {
            return;
        }
        let fmt = FORMATTING.contains(&name);
        self.push(name, fmt);
        if fmt {
            self.add_active(tag);
        }
    }

    fn end(&mut self, name: &str) {
        if FORMATTING.contains(&name) {
            // the adoption agency searches the active list for the name
            let active = self.level().active as u64;
            self.fmt_work = self.fmt_work.saturating_add(active);
            self.end_formatting(name);
            return;
        }
        let k = match name {
            "p" => BUTTON,
            "td" | "th" | "tr" | "tbody" | "thead" | "tfoot" | "table" | "caption" => TABLE_K,
            n if SPECIAL.contains(&n) => SCOPE,
            _ => SPECIAL_K,
        };
        self.close(&[name], k);
    }

    /// The first limit passed, as `(kind, limit)`.
    fn exceeded(&self) -> Option<(&'static str, u64)> {
        if self.max > MAX_OPEN_ELEMENTS {
            Some(("html nesting depth", MAX_OPEN_ELEMENTS as u64))
        } else if self.reconstructed > self.rec_cap {
            Some(("html formatting elements", self.rec_cap as u64))
        } else if self.fmt_work > self.fmt_cap {
            Some(("html formatting elements", self.fmt_cap))
        } else {
            None
        }
    }
}

struct Sink(RefCell<State>);

impl TokenSink for Sink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let mut s = self.0.borrow_mut();
        if s.exceeded().is_some() {
            return TokenSinkResult::Continue;
        }
        let tag = match token {
            Token::TagToken(tag) => tag,
            Token::CharacterTokens(_) => {
                s.reconstruct();
                return TokenSinkResult::Continue;
            }
            _ => return TokenSinkResult::Continue,
        };
        let name: &str = &tag.name;
        match tag.kind {
            TagKind::StartTag => {
                if let Some(&(_, kind)) = RAW_TEXT.iter().find(|(r, _)| *r == name) {
                    return TokenSinkResult::RawData(kind);
                }
                s.start(&tag);
                if name == "plaintext" {
                    return TokenSinkResult::Plaintext;
                }
            }
            TagKind::EndTag => s.end(name),
        }
        TokenSinkResult::Continue
    }
}

fn scan(text: &str) -> State {
    let state = State {
        fmt_cap: FMT_WORK_PER_BYTE
            .saturating_mul(text.len() as u64)
            .saturating_add(FMT_WORK_BASE),
        rec_cap: MAX_RECONSTRUCTED.max(text.len()),
        levels: vec![Level::default()],
        ..State::default()
    };
    let tok = Tokenizer::new(Sink(RefCell::new(state)), TokenizerOpts::default());
    let queue = BufferQueue::default();
    queue.push_back(StrTendril::from(text));
    let _ = tok.feed(&queue);
    tok.end();
    tok.sink.0.into_inner()
}

/// Deepest nesting of open elements, approximated by a tokenizer pass.
#[cfg(test)]
fn max_depth(text: &str) -> usize {
    scan(text).max
}

/// `Limit { "html attributes per element", 8192 }` past [`MAX_ATTRS`] (before tokenizing),
/// `Limit { "html nesting depth", 16384 }` past [`MAX_OPEN_ELEMENTS`],
/// `Limit { "html formatting elements", max(1000000, input length) }` past [`MAX_RECONSTRUCTED`]
/// or the input length, and
/// `Limit { "html formatting elements", cap }` past the formatting-list scan work cap.
pub(crate) fn check_nesting(text: &str) -> Result<()> {
    if max_attrs(text) > MAX_ATTRS {
        return Err(ParseError::Limit {
            kind: "html attributes per element".into(),
            limit: MAX_ATTRS as u64,
        });
    }
    match scan(text).exceeded() {
        Some((kind, limit)) => Err(ParseError::Limit {
            kind: kind.into(),
            limit,
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "guard_test.rs"]
mod tests;
