//! Hyperlinks from `/Link` annotations with a `/URI` action (ISO 32000-1 §12.5.6.5, §12.6.4.7).
//!
//! Stage A ([`page_links`]) reads the page's annotations, keeps `http`, `https` and `mailto`
//! URLs ([`clean_url`]) and marks every non-whitespace glyph of the (hidden-filtered) page that
//! lies at least half (by area) inside a link's rectangles (`/QuadPoints` when given, else
//! `/Rect`); of nested or overlapping links the smallest wins, then the last in `/Annots`.
//! Glyphs of spans a hidden-text rule flags, and of rotated text, get no link. Stage B
//! ([`apply`]) finds each run of linked glyphs inside a finished block (heading, paragraph, or a
//! table's or list's paragraphs) by its non-whitespace chars and records a [`Link`] there; text,
//! order and blocks do not change. A run whose text is not found in the block is dropped
//! (precision first; text the layout normalised differently from the glyphs gets no link).

use pdf_oxide::annotation_types::{AnnotationFlags, AnnotationSubtype};
use pdf_oxide::geometry::Rect;
use pdf_oxide::object::Object;
use pdf_oxide::{LinkAction, PdfDocument, StructuredPage};
use plaindoc_core::{BBox, Block, Level, Link, Section, TextSource, Warning, WarningKind};

use super::hidden::{PageFacts, content_rule, placement_rule};

/// Longest URL kept, in bytes.
pub(crate) const MAX_URL: usize = 2048;
/// Share of a glyph's area inside a link rectangle that makes it linked.
const MIN_GLYPH_SHARE: f32 = 0.5;
/// Largest text rotation (degrees) whose glyphs can be linked.
const MAX_TILT: f32 = 1.0;
/// Slack around a block box when deciding a glyph centre is inside it (pt).
const BOX_SLACK: f32 = 1.0;

/// A non-whitespace glyph of the page in span order: its centre and link index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Glyph {
    pub c: char,
    pub x: f32,
    pub y: f32,
    pub link: Option<u32>,
}

/// Links of one page: empty `urls` (and `glyphs`) when the page has none.
#[derive(Debug, Default)]
pub(crate) struct PageLinks {
    pub urls: Vec<String>,
    pub glyphs: Vec<Glyph>,
    pub warnings: Vec<Warning>,
}

/// Format characters (Cf) that can hide or reorder parts of a URL.
fn is_format(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{0600}'..='\u{0605}' | '\u{061C}' | '\u{06DD}' | '\u{070F}' | '\u{180E}'
        | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}' | '\u{E0001}'
        | '\u{E0020}'..='\u{E007F}')
}

/// The URL without control and format chars and outer whitespace, with a lower-case scheme;
/// `None` unless it is `http(s)://host…` or `mailto:…` of at most [`MAX_URL`] bytes.
pub(crate) fn clean_url(u: &str) -> Option<String> {
    let s: String = u
        .chars()
        .filter(|&c| !c.is_control() && !is_format(c))
        .collect();
    let s = s.trim();
    let (scheme, rest) = s.split_once(':')?;
    let scheme = scheme.to_ascii_lowercase();
    let ok = match scheme.as_str() {
        "http" | "https" => rest
            .strip_prefix("//")
            .and_then(|r| r.split(['/', '?', '#']).next())
            .is_some_and(|host| !host.trim().is_empty()),
        "mailto" => rest.split('?').next().is_some_and(|a| !a.trim().is_empty()),
        _ => false,
    };
    let url = format!("{scheme}:{rest}");
    (ok && url.len() <= MAX_URL).then_some(url)
}

/// Normalised `[x0, y0, x1, y1]` of a rectangle given by two corners.
fn corners(v: [f64; 4]) -> Option<Rect> {
    let [a, b, c, d] = v.map(|x| x as f32);
    if ![a, b, c, d].iter().all(|x| x.is_finite()) {
        return None;
    }
    let (x0, y0) = (a.min(c), b.min(d));
    Some(Rect::new(x0, y0, a.max(c) - x0, b.max(d) - y0))
}

/// `/QuadPoints` of a link (8 numbers per quad) as the boxes of the quads.
fn quads(dict: &std::collections::HashMap<String, Object>) -> Vec<Rect> {
    let Some(Object::Array(a)) = dict.get("QuadPoints") else {
        return Vec::new();
    };
    let n: Vec<f32> = a
        .iter()
        .filter_map(|o| match o {
            Object::Integer(i) => Some(*i as f32),
            Object::Real(r) => Some(*r as f32),
            _ => None,
        })
        .collect();
    if n.len() != a.len() || !n.len().is_multiple_of(8) || n.iter().any(|v| !v.is_finite()) {
        return Vec::new();
    }
    n.chunks(8)
        .map(|q| {
            let xs = [q[0], q[2], q[4], q[6]];
            let ys = [q[1], q[3], q[5], q[7]];
            let x0 = xs.iter().copied().fold(f32::INFINITY, f32::min);
            let y0 = ys.iter().copied().fold(f32::INFINITY, f32::min);
            let x1 = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let y1 = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            Rect::new(x0, y0, x1 - x0, y1 - y0)
        })
        .collect()
}

fn overlap(a: &Rect, b: &Rect) -> f32 {
    let w = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let h = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
}

/// Whether a glyph box lies at least [`MIN_GLYPH_SHARE`] inside `areas` (a box without area: its
/// centre inside one).
fn covered(g: &Rect, areas: &[Rect]) -> bool {
    let area = g.width * g.height;
    if area > 0.0 && area.is_finite() {
        areas.iter().map(|r| overlap(g, r)).sum::<f32>() >= MIN_GLYPH_SHARE * area
    } else {
        let (x, y) = (g.x + g.width / 2.0, g.y + g.height / 2.0);
        areas
            .iter()
            .any(|r| r.x <= x && x <= r.x + r.width && r.y <= y && y <= r.y + r.height)
    }
}

/// The page's URI links (`urls` with their areas) and the dropped count.
fn read_links(doc: &PdfDocument, p: usize) -> (Vec<(String, Vec<Rect>)>, usize) {
    let Ok(annots) = doc.get_annotations(p) else {
        return (Vec::new(), 0);
    };
    let mut out = Vec::new();
    let mut dropped = 0;
    for a in annots {
        if a.subtype_enum != AnnotationSubtype::Link {
            continue;
        }
        let Some(LinkAction::Uri(uri)) = &a.action else {
            continue;
        };
        let hidden = a.flags.is_hidden() || a.flags.contains(AnnotationFlags::NO_VIEW);
        let url = clean_url(uri).filter(|_| !hidden);
        let mut areas = a.raw_dict.as_ref().map(quads).unwrap_or_default();
        if areas.is_empty() {
            areas.extend(a.rect.and_then(corners));
        }
        match url {
            Some(u) if !areas.is_empty() => out.push((u, areas)),
            _ => dropped += 1,
        }
    }
    (out, dropped)
}

/// Stage A: the links of page `p` over the glyphs of `page` (after the hidden-text filter;
/// `facts` when the hidden-text rules ran); nothing is read when `on` is false
/// (`ParseOptions::links`) or the page failed.
pub(crate) fn page_links(
    doc: &PdfDocument,
    p: usize,
    page: &Result<StructuredPage, String>,
    facts: Option<&PageFacts>,
    on: bool,
) -> PageLinks {
    let (true, Ok(page)) = (on, page) else {
        return PageLinks::default();
    };
    let (links, dropped) = read_links(doc, p);
    let mut out = PageLinks::default();
    if dropped > 0 {
        let mut w = Warning::new(
            WarningKind::ContentDropped,
            Level::Info,
            format!(
                "{dropped} link annotation(s) dropped: hidden, without a rectangle, or the URL \
                 is not http, https or mailto or is longer than {MAX_URL} bytes"
            ),
        );
        w.code = Some("link");
        out.warnings.push(w);
    }
    if links.is_empty() {
        return out;
    }
    let size: Vec<f32> = links
        .iter()
        .map(|(_, a)| a.iter().map(|r| r.width * r.height).sum())
        .collect();
    let spans = page.regions.iter().flat_map(|r| r.spans.iter());
    for s in spans {
        // `to_chars` lays glyphs out along x: the glyph boxes of turned text are not on its ink
        let r = s.rotation_degrees.rem_euclid(360.0);
        let flagged = s.turned_bbox
            || r.min(360.0 - r) > MAX_TILT
            || facts.is_some_and(|f| placement_rule(s, f).is_some())
            || content_rule(&s.text).is_some();
        for ch in s.to_chars().into_iter().filter(|c| !c.char.is_whitespace()) {
            let b = &ch.bbox;
            let link = if flagged {
                None
            } else {
                // nested or overlapping links: the smallest one, then the topmost (last)
                (0..links.len())
                    .filter(|&i| covered(b, &links[i].1))
                    .min_by(|&i, &j| size[i].total_cmp(&size[j]).then(j.cmp(&i)))
                    .map(|i| i as u32)
            };
            out.glyphs.push(Glyph {
                c: ch.char,
                x: b.x + b.width / 2.0,
                y: b.y + b.height / 2.0,
                link,
            });
        }
    }
    if out.glyphs.iter().any(|g| g.link.is_some()) {
        out.urls = links.into_iter().map(|(u, _)| u).collect();
    } else {
        out.glyphs.clear();
    }
    out
}

fn inside(b: &BBox, g: &Glyph) -> bool {
    b.x - BOX_SLACK <= g.x
        && g.x <= b.x + b.w + BOX_SLACK
        && b.y - BOX_SLACK <= g.y
        && g.y <= b.y + b.h + BOX_SLACK
}

/// Headings and paragraphs of a block, tables' cells and lists' items included, in order.
fn texts<'a>(b: &'a mut Block, out: &mut Vec<&'a mut Block>) {
    match b {
        Block::Heading { .. } | Block::Paragraph { .. } => out.push(b),
        Block::Table { rows, .. } => {
            for x in rows.iter_mut().flatten().flat_map(|c| c.blocks.iter_mut()) {
                texts(x, out);
            }
        }
        Block::List { items, .. } => {
            for x in items.iter_mut().flat_map(|i| i.blocks.iter_mut()) {
                texts(x, out);
            }
        }
        _ => {}
    }
}

fn parts(b: &mut Block) -> Option<(&str, &mut Vec<Link>)> {
    match b {
        Block::Heading { text, links, .. } | Block::Paragraph { text, links, .. } => {
            Some((text.as_str(), links))
        }
        _ => None,
    }
}

fn block_box(b: &Block) -> Option<BBox> {
    match b {
        Block::Heading { bbox, .. }
        | Block::Paragraph { bbox, .. }
        | Block::Table { bbox, .. }
        | Block::List { bbox, .. } => *bbox,
        _ => None,
    }
}

/// Stage B: the page's link warnings, and its links recorded in the blocks unless the page's
/// text is OCR (the layer's glyphs are not what it shows).
pub(crate) fn add_links(section: &mut Section, links: &PageLinks) {
    section.warnings.extend(links.warnings.iter().cloned());
    if section.source != TextSource::Ocr {
        apply(&mut section.blocks, links);
    }
}

/// Records the page's links in `blocks` (see the module docs).
pub(crate) fn apply(blocks: &mut [Block], links: &PageLinks) {
    if links.urls.is_empty() {
        return;
    }
    let mut used = vec![false; links.glyphs.len()];
    for b in blocks.iter_mut() {
        let Some(bb) = block_box(b) else {
            continue;
        };
        let mut targets = Vec::new();
        texts(b, &mut targets);
        apply_block(&mut targets, &bb, links, &mut used);
    }
}

/// One block (`targets`: its text blocks, in order) inside `bb`.
fn apply_block(targets: &mut [&mut Block], bb: &BBox, links: &PageLinks, used: &mut [bool]) {
    let ids: Vec<usize> = (0..links.glyphs.len())
        .filter(|&i| inside(bb, &links.glyphs[i]))
        .collect();
    if !ids
        .iter()
        .any(|&i| links.glyphs[i].link.is_some() && !used[i])
    {
        return;
    }
    // non-whitespace chars of the targets: (char, target, char index in its text)
    let hay: Vec<(char, usize, usize)> = targets
        .iter_mut()
        .enumerate()
        .filter_map(|(t, b)| parts(b).map(|(text, _)| (t, text.to_string())))
        .flat_map(|(t, text)| {
            text.chars()
                .enumerate()
                .filter(|(_, c)| !c.is_whitespace())
                .map(|(i, c)| (c, t, i))
                .collect::<Vec<_>>()
        })
        .collect();
    let mut k = 0;
    while k < ids.len() {
        let g = links.glyphs[ids[k]];
        let Some(link) = g.link.filter(|_| !used[ids[k]]) else {
            k += 1;
            continue;
        };
        let run_end = (k..ids.len())
            .find(|&j| links.glyphs[ids[j]].link != Some(link) || used[ids[j]])
            .unwrap_or(ids.len());
        let needle: Vec<char> = ids[k..run_end].iter().map(|&i| links.glyphs[i].c).collect();
        if let Some(at) = nearest(&hay, &needle, k) {
            let (_, t, s) = hay[at];
            let (_, _, e) = hay[at + needle.len() - 1];
            if let Some((_, l)) = parts(targets[t]) {
                l.push(Link::new(s, e + 1, links.urls[link as usize].clone()));
            }
            for &i in &ids[k..run_end] {
                used[i] = true;
            }
        }
        k = run_end;
    }
    for b in targets.iter_mut() {
        if let Some((text, l)) = parts(b) {
            tidy(text, l);
        }
    }
}

/// The occurrence of `needle` in `hay` (within one target) nearest to `expected`.
fn nearest(hay: &[(char, usize, usize)], needle: &[char], expected: usize) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len())
        .filter(|&a| {
            hay[a].1 == hay[a + needle.len() - 1].1
                && hay[a..a + needle.len()]
                    .iter()
                    .zip(needle)
                    .all(|(h, c)| h.0 == *c)
        })
        .min_by_key(|&a| a.abs_diff(expected))
}

/// Sorts the links of a text, drops overlapping ones and joins neighbours with the same URL that
/// only whitespace separates (a link set one annotation per line).
fn tidy(text: &str, links: &mut Vec<Link>) {
    if links.len() < 2 {
        return;
    }
    links.sort_by_key(|l| (l.start, l.end));
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Link> = Vec::with_capacity(links.len());
    for l in links.drain(..) {
        if let Some(last) = out.last_mut() {
            if l.start < last.end {
                continue;
            }
            if l.url == last.url && chars[last.end..l.start].iter().all(|c| c.is_whitespace()) {
                last.end = l.end;
                continue;
            }
        }
        out.push(l);
    }
    *links = out;
}

#[cfg(test)]
#[path = "links_test.rs"]
mod tests;
