//! PDF bookmarks (the document outline) as a heading signal: an entry whose title matches a
//! paragraph on its destination page (±1) makes that paragraph a heading, with the outline depth
//! as its level.
//!
//! The order of the signals (outline first, then numbering, then style) and the fuzzy match of a
//! title on its page follow Docling 2.148 (MIT; logic only, no code taken).

use std::collections::HashSet;

use pdf_oxide::PdfDocument;
use pdf_oxide::object::{Object, ObjectRef};
use pdf_oxide::outline::{Destination, OutlineItem};

use super::headings::{Para, StyledLine};
use super::headings_number::is_caption;
use super::headings_style::{despaced, letter_spaced, title_text};

/// Outline items read at most; a larger (or cyclic, or deeper) outline is not used.
const MAX_ITEMS: usize = 4096;
const MAX_DEPTH: usize = 32;
/// Entries tried per page: a page with more is a table of contents or a broken outline.
const MAX_PER_PAGE: usize = 64;
/// Normalised Levenshtein similarity of a title and a paragraph that counts as a match.
const MIN_SIMILARITY: f64 = 0.85;
/// Highest level given from the outline depth (H4-H6 are out of scope).
const MAX_LEVEL: u8 = 3;

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    key: String,
    level: u8,
    page: usize,
    used: bool,
}

/// Outline entries with a page destination, by page (document order within a page); each matches
/// one paragraph.
#[derive(Clone, Debug, Default)]
pub(crate) struct Outline {
    entries: Vec<Entry>,
}

/// Lower case, letters and digits only, words separated by one space.
fn key(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The key without a leading section number ("2 1 methods" → "methods").
fn unnumbered(k: &str) -> &str {
    let mut rest = k;
    while let Some((w, tail)) = rest.split_once(' ') {
        if w.len() > 3 || !w.chars().all(|c| c.is_ascii_digit()) {
            break;
        }
        rest = tail;
    }
    rest
}

/// The closing punctuation of a text, if any.
fn closing(t: &str) -> Option<char> {
    t.trim_end().chars().last().filter(|c| ".!?;,".contains(*c))
}

/// A sentence: three words or more ending in `.`, `!`, `?`, `;` or `,`, neither bold nor larger
/// than the body (bookmarks set on paragraphs; a bold or larger "Статья 5. Права сторон." is a
/// title).
fn sentence(p: &Para, body: f32) -> bool {
    closing(&p.text).is_some()
        && p.text.split_whitespace().count() >= 3
        && !p.bold
        && !(body > 0.0 && p.size > LARGER_THAN_BODY * body)
}

/// Size over the body that makes a paragraph styled.
const LARGER_THAN_BODY: f32 = 1.05;

/// The last word of a key when it is a number (a page number on a contents line).
fn last_number(k: &str) -> Option<&str> {
    k.rsplit(' ')
        .next()
        .filter(|w| w.chars().all(|c| c.is_ascii_digit()))
}

fn similar(a: &str, b: &str) -> bool {
    let (na, nb) = (a.chars().count(), b.chars().count());
    if na.min(nb) < 2 || na.abs_diff(nb) * 4 > na.max(nb) {
        return false;
    }
    if na.min(nb) <= 5 {
        return a == b;
    }
    strsim::normalized_levenshtein(a, b) >= MIN_SIMILARITY
}

/// Whether the outline is acyclic, at most [`MAX_ITEMS`] items and [`MAX_DEPTH`] levels deep:
/// the fork's reader follows `/First` and `/Next` without a guard.
fn bounded(doc: &PdfDocument) -> bool {
    let get = |o: &Object, k: &str| o.as_dict().and_then(|d| d.get(k)?.as_reference());
    let Some(root) = doc.catalog().ok().and_then(|c| get(&c, "Outlines")) else {
        return false;
    };
    let Some(first) = doc.load_object(root).ok().and_then(|o| get(&o, "First")) else {
        return false;
    };
    let mut seen: HashSet<ObjectRef> = HashSet::new();
    let mut stack = vec![(first, 0usize)];
    while let Some((r, depth)) = stack.pop() {
        if depth >= MAX_DEPTH || !seen.insert(r) || seen.len() > MAX_ITEMS {
            return false;
        }
        let Ok(o) = doc.load_object(r) else {
            continue;
        };
        if let Some(n) = get(&o, "Next") {
            stack.push((n, depth));
        }
        if let Some(c) = get(&o, "First") {
            stack.push((c, depth + 1));
        }
    }
    true
}

fn flatten(items: &[OutlineItem], depth: usize, out: &mut Vec<Entry>) {
    for it in items {
        let k = key(&it.title);
        if let Some(Destination::PageIndex(page)) = it.dest
            && k.chars().count() >= 2
        {
            out.push(Entry {
                key: k,
                level: (depth + 1).min(MAX_LEVEL as usize) as u8,
                page,
                used: false,
            });
        }
        flatten(&it.children, depth + 1, out);
    }
}

impl Outline {
    /// The document outline, empty when there is none, it is too large or cyclic, or it fails to
    /// read.
    pub(crate) fn read(doc: &PdfDocument) -> Self {
        let mut entries = Vec::new();
        if bounded(doc)
            && let Ok(Some(items)) = doc.get_outline()
        {
            flatten(&items, 0, &mut entries);
        }
        entries.sort_by_key(|e| e.page);
        Self { entries }
    }

    #[cfg(test)]
    pub(crate) fn from_titles(t: &[(&str, u8, usize)]) -> Self {
        let mut entries: Vec<Entry> = t
            .iter()
            .map(|&(title, level, page)| Entry {
                key: key(title),
                level,
                page,
                used: false,
            })
            .collect();
        entries.sort_by_key(|e| e.page);
        Self { entries }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Indices of the entries of pages `lo..=hi`.
    fn pages(&self, lo: usize, hi: usize) -> std::ops::Range<usize> {
        let a = self.entries.partition_point(|e| e.page < lo);
        let b = self.entries.partition_point(|e| e.page <= hi);
        a..b.max(a)
    }

    /// Index of the first unused entry whose title matches `text`: on `page` first, then on the
    /// pages next to it. A text ending in a number other than the title's (a contents line
    /// "Introduction ...... 2") matches nothing.
    pub(crate) fn find(&self, page: usize, text: &str) -> Option<usize> {
        let k = key(&despaced(text));
        if k.is_empty() {
            return None;
        }
        let bare = unnumbered(&k);
        // letters whose words are not known: compare without spaces
        let squash = |s: &str| s.split_whitespace().collect::<String>();
        let spaced = letter_spaced(text).then(|| squash(&k));
        let matches = |e: &Entry| {
            !e.used
                && (last_number(&k).is_none() || last_number(&k) == last_number(&e.key))
                && (similar(&e.key, &k)
                    || similar(unnumbered(&e.key), bare)
                    || spaced.as_ref().is_some_and(|s| *s == squash(&e.key)))
        };
        let near = self.pages(page.saturating_sub(1), page + 1);
        let exact = self.pages(page, page);
        exact
            .clone()
            .take(MAX_PER_PAGE)
            .chain(near.filter(|i| !exact.contains(i)).take(MAX_PER_PAGE))
            .find(|&i| matches(&self.entries[i]))
    }

    /// Marks entry `i` used; its level.
    pub(crate) fn take(&mut self, i: usize) -> u8 {
        self.entries[i].used = true;
        self.entries[i].level
    }
}

/// Lines a matched title may span (paper titles wrap).
const MAX_TITLE_LINES: usize = 4;
const MAX_TITLE_CHARS: usize = 200;

/// The paragraph of `lines[r]` (first line's style, largest size), as `headings::paragraphs`.
fn para_of(lines: &[StyledLine], r: std::ops::Range<usize>) -> Para {
    let first = &lines[r.start];
    let rest = &lines[r.start + 1..r.end];
    Para {
        text: std::iter::once(first)
            .chain(rest)
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        size: rest.iter().fold(first.size, |s, l| s.max(l.size)),
        bold: first.bold,
        font: first.font.clone(),
        color: first.color,
        bbox: rest.iter().fold(first.bbox, |b, l| b.union(&l.bbox)),
        lines: r,
    }
}

/// A matched paragraph still has to look like a title: few lines, not long, not a caption, no
/// dot leader (a contents line), not a [`sentence`] (`body` is the body size).
fn title_shaped(p: &Para, body: f32) -> bool {
    let t = p.text.trim_end();
    let n = t.chars().filter(|c| !c.is_whitespace()).count();
    let lower = t.to_lowercase();
    let leader = t.contains("...") || t.contains('…') || t.contains(". . .") || t.contains("· · ·");
    p.lines.len() <= MAX_TITLE_LINES
        && n <= MAX_TITLE_CHARS
        && !is_caption(&lower)
        && !leader
        && !sentence(p, body)
}

/// Paragraphs of one region with their outline level: a paragraph matching an entry of `page`, or
/// its first one to three lines matching one (a title joined with the text under it, split off).
pub(crate) fn mark_paragraphs(
    lines: &[StyledLine],
    paras: Vec<Para>,
    page: usize,
    body: f32,
    outline: &mut Outline,
) -> (Vec<Para>, Vec<Option<u8>>) {
    let mut out = Vec::with_capacity(paras.len());
    let mut levels = Vec::with_capacity(paras.len());
    for p in paras {
        if title_shaped(&p, body)
            && let Some(i) = outline.find(page, &title_text(&p, lines, 0))
        {
            out.push(p);
            levels.push(Some(outline.take(i)));
            continue;
        }
        let (a, b) = (p.lines.start, p.lines.end);
        let head = (1..(b - a).min(MAX_TITLE_LINES)).find_map(|k| {
            let h = para_of(lines, a..a + k);
            let i = title_shaped(&h, body).then(|| outline.find(page, &h.text))??;
            Some((h, i))
        });
        match head {
            Some((h, i)) => {
                let k = h.lines.end;
                out.push(h);
                levels.push(Some(outline.take(i)));
                out.push(para_of(lines, k..b));
                levels.push(None);
            }
            None => {
                out.push(p);
                levels.push(None);
            }
        }
    }
    (out, levels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_contents_line_does_not_take_the_title_of_the_next_page() {
        let mut o = Outline::from_titles(&[("Общие сведения", 1, 1), ("Глава 2", 1, 1)]);
        let toc = o.find(0, "Общие сведения 2");
        assert_eq!(toc, None, "page number of a contents line");
        assert!(o.find(0, "Глава 2").is_some(), "a number of the title");
        assert!(!title_shaped(&para("Общие сведения ........ 2"), 11.0));
        let mut stmt = para("Клиент вправе отозвать согласие в любой момент.");
        stmt.bold = false;
        assert!(!title_shaped(&stmt, 11.0), "a plain sentence");
        stmt.size = 14.0;
        assert!(title_shaped(&stmt, 11.0), "larger than the body: a title");
        assert!(title_shaped(&para("Общие сведения"), 11.0));
        // the exact page goes first
        o = Outline::from_titles(&[("Results", 1, 2), ("Results", 2, 3)]);
        let i = o.find(3, "Results").unwrap();
        assert_eq!(o.take(i), 2);
    }

    #[test]
    fn a_bookmarked_article_title_with_a_closing_dot_is_a_heading() {
        // a bold article title over plain text
        let mut body = line("Стороны вправе изменить договор по соглашению.", 380.0);
        body.bold = false;
        let lines = [line("Статья 5. Права сторон.", 400.0), body.clone()];
        let paras = super::super::headings::paragraphs(&lines);
        let mut o = Outline::from_titles(&[("Статья 5. Права сторон.", 2, 0)]);
        let (_, m) = mark_paragraphs(&lines, paras, 0, 11.0, &mut o);
        assert_eq!(m[0], Some(2));
        // a plain body-sized sentence under a bookmark is not a heading
        let lines = [body];
        let paras = super::super::headings::paragraphs(&lines);
        let title = "Стороны вправе изменить договор по соглашению.";
        let mut o = Outline::from_titles(&[(title, 2, 0)]);
        let (_, m) = mark_paragraphs(&lines, paras, 0, 11.0, &mut o);
        assert_eq!(m, [None]);
    }

    fn para(text: &str) -> Para {
        para_of(&[line(text, 400.0)], 0..1)
    }

    #[test]
    fn titles_match_on_their_page_once() {
        let mut o = Outline::from_titles(&[("2. Methods", 2, 3), ("Results", 2, 4)]);
        assert_eq!(
            o.find(0, "2. Methods"),
            None,
            "far from the destination page"
        );
        let i = o.find(3, "2 Methods").unwrap();
        assert_eq!(o.take(i), 2);
        assert_eq!(o.find(3, "2. Methods"), None, "an entry matches once");
        assert!(o.find(5, "RESULTS").is_some(), "next page, any case");
        assert!(o.find(4, "Results and discussion").is_none());
    }

    #[test]
    fn numbering_may_differ_but_short_titles_match_exactly() {
        let o = Outline::from_titles(&[("Introduction", 1, 0), ("Data", 1, 0)]);
        assert!(o.find(0, "1 Introduction").is_some());
        assert!(o.find(0, "Introductions").is_some());
        assert!(o.find(0, "Date").is_none());
    }

    /// One empty page, an outline of two entries; `cycle` links the second back to the first.
    fn outline_pdf(cycle: bool) -> Vec<u8> {
        let next = if cycle { "/Next 5 0 R" } else { "" };
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".into(),
            "<< /Type /Outlines /First 5 0 R >>".into(),
            "<< /Title (Methods) /Dest [3 0 R /Fit] /Next 6 0 R >>".into(),
            format!("<< /Title (Results) /Dest [3 0 R /Fit] {next} /First 7 0 R >>"),
            "<< /Title (Data) /Dest [3 0 R /Fit] >>".into(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offs = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offs.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let x = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
        for o in offs {
            out.extend(format!("{o:010} 00000 n \n").bytes());
        }
        let n = objs.len() + 1;
        out.extend(
            format!("trailer\n<< /Size {n} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n").bytes(),
        );
        out
    }

    #[test]
    fn outline_entries_with_depth_and_no_cyclic_outline() {
        let doc = PdfDocument::from_bytes(outline_pdf(false)).unwrap();
        let o = Outline::read(&doc);
        let got: Vec<(&str, u8, usize)> = o
            .entries
            .iter()
            .map(|e| (e.key.as_str(), e.level, e.page))
            .collect();
        assert_eq!(got, [("methods", 1, 0), ("results", 1, 0), ("data", 2, 0)]);
        let doc = PdfDocument::from_bytes(outline_pdf(true)).unwrap();
        assert!(Outline::read(&doc).is_empty(), "a cycle is not followed");
    }

    fn line(text: &str, y: f32) -> StyledLine {
        StyledLine {
            text: text.into(),
            size: 11.0,
            bold: true,
            font: "Helvetica-Bold".into(),
            color: (0, 0, 0),
            bbox: pdf_oxide::geometry::Rect::new(40.0, y, 200.0, 11.0),
            gap: 0.0,
            words: None,
        }
    }

    #[test]
    fn a_title_joined_with_the_line_under_it_is_split_off() {
        let lines = [
            line("Education", 400.0),
            line("Test State University", 386.0),
        ];
        let paras = super::super::headings::paragraphs(&lines);
        assert_eq!(paras.len(), 1);
        let mut o = Outline::from_titles(&[("Education", 2, 0)]);
        let (p, m) = mark_paragraphs(&lines, paras.clone(), 0, 11.0, &mut o);
        assert_eq!(
            p.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(),
            ["Education", "Test State University"]
        );
        assert_eq!(m, [Some(2), None]);
        // no entry left: nothing changes
        let (p, m) = mark_paragraphs(&lines, paras, 0, 11.0, &mut o);
        assert_eq!((p.len(), m), (1, vec![None]));
    }
}
