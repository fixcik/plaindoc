//! Running headers and footers by layers across neighbouring pages: the k-th paragraph from the
//! top (bottom) of page i matches the k-th of page i±1 or i±2 (same size, overlapping boxes, same
//! text or a page number that differs by the page step). Matched body text is moved into
//! `Header`/`Footer` regions, which the layout maps to `Block::Note`; nothing is deleted.
//!
//! Idea and thresholds follow OpenDataLoader's header/footer layering (ideas only; no code taken).

use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};

use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::TextSpan;
use pdf_oxide::{RegionRole, StructuredPage, StructuredRegion};
use plaindoc_core::BBox;

use super::headings::{paragraphs, styled_lines};
use super::tables::DetectedTable;

/// Layers examined from each edge.
const MAX_LAYERS: usize = 5;
/// A layer must lie within this distance (pt) of the previous layer on the same page.
const MAX_LAYER_GAP: f32 = 30.0;
/// Header/footer candidates lie within this share of the page height from the edge.
const EDGE_BAND: f32 = 0.15;
/// Same size within this many points.
const SIZE_TOL: f32 = 0.5;
/// Gate against repeated table headers: a candidate is kept when its gap to the nearest body
/// paragraph is at least this many line heights, or when at most one other paragraph shares its row.
const MIN_BODY_GAP_LINES: f32 = 1.0;
const MAX_ROW_NEIGHBOURS: usize = 1;
/// A gap wider than this many font sizes separates two cells of one row.
const CHUNK_GAP_FS: f32 = 1.0;
/// A page number sits apart from its running title by more than this many font sizes (wider than
/// a word space; folios set close to the title are common).
const FOLIO_GAP_FS: f32 = 0.5;
/// Text larger than the body size by this factor is a heading, never a running head/footer.
const MAX_SIZE_OF_BODY: f32 = 1.1;
/// Paragraph candidates per page and edge examined (service guard).
const MAX_CANDIDATES: usize = 64;
/// A constant running line detected on at least this many pages is a running head on its first
/// page too (a line seen on fewer pages keeps its first copy as body text, ruling B3), and its
/// copies outside the ±2 page window are marked as well ([`propagate`]).
const MIN_SUPPORT: usize = 3;
/// Copies of an established running line kept to check other pages against (service guard).
const MAX_COPIES: usize = 4;

#[path = "running_cands.rs"]
mod cands;
use cands::*;

/// Edge order: top-down (by top edge) or bottom-up (by bottom edge), stable.
fn edge_order(c: &[Cand], edge: Edge) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..c.len()).collect();
    match edge {
        Edge::Top => idx.sort_by(|&a, &b| {
            (c[b].bbox.y + c[b].bbox.height).total_cmp(&(c[a].bbox.y + c[a].bbox.height))
        }),
        Edge::Bottom => idx.sort_by(|&a, &b| c[a].bbox.y.total_cmp(&c[b].bbox.y)),
    }
    idx.truncate(MAX_CANDIDATES);
    idx
}

fn in_band(c: &Cand, edge: Edge, h: f32) -> bool {
    match edge {
        Edge::Top => c.bbox.y >= (1.0 - EDGE_BAND) * h,
        Edge::Bottom => c.bbox.y + c.bbox.height <= EDGE_BAND * h,
    }
}

/// Gate against repeated table header rows (see [`MIN_BODY_GAP_LINES`]).
fn passes_gate(c: &Cand, all: &[Cand], marked: &HashSet<usize>, me: usize, edge: Edge) -> bool {
    let yc = |x: &Cand| x.bbox.y + x.bbox.height / 2.0;
    let row = all
        .iter()
        .enumerate()
        .filter(|(i, x)| *i != me && (yc(x) - yc(c)).abs() < 0.5 * x.bbox.height.min(c.bbox.height))
        .count()
        + c.chunks.saturating_sub(1);
    let gap = all
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != me && !marked.contains(i))
        .filter_map(|(_, x)| match edge {
            Edge::Top if x.bbox.y + x.bbox.height <= c.bbox.y => {
                Some(c.bbox.y - (x.bbox.y + x.bbox.height))
            }
            Edge::Bottom if x.bbox.y >= c.bbox.y + c.bbox.height => {
                Some(x.bbox.y - (c.bbox.y + c.bbox.height))
            }
            _ => None,
        })
        .fold(f32::INFINITY, f32::min);
    gap >= MIN_BODY_GAP_LINES * c.bbox.height || row <= MAX_ROW_NEIGHBOURS
}

/// Running header/footer paragraphs per page: `(page, candidate)` pairs for one edge.
fn detect_edge(cands: &[Vec<Cand>], heights: &[f32], edge: Edge) -> Vec<HashSet<usize>> {
    let n = cands.len();
    let mut found: Vec<HashSet<usize>> = vec![HashSet::new(); n];
    if n < 2 {
        return found;
    }
    let order: Vec<Vec<usize>> = cands.iter().map(|c| edge_order(c, edge)).collect();
    let mut prev_edge: Option<Vec<Option<f32>>> = None;
    for k in 0..MAX_LAYERS {
        let mut layer: Vec<(usize, usize)> = Vec::new();
        for i in 0..n {
            let Some(&ci) = order[i].get(k) else { continue };
            let c = &cands[i][ci];
            if !in_band(c, edge, heights[i]) {
                continue;
            }
            if let Some(prev) = &prev_edge {
                let Some(pe) = prev[i] else { continue };
                let gap = match edge {
                    Edge::Top => pe - (c.bbox.y + c.bbox.height),
                    Edge::Bottom => c.bbox.y - pe,
                };
                if gap > MAX_LAYER_GAP {
                    continue;
                }
            }
            let matched = [
                (i + 1, 1u64),
                (i.wrapping_sub(1), 1),
                (i + 2, 2),
                (i.wrapping_sub(2), 2),
            ]
            .into_iter()
            .any(|(j, step)| {
                j < n
                    && order[j]
                        .get(k)
                        .is_some_and(|&cj| similar(c, &cands[j][cj], step))
            });
            if matched && passes_gate(c, &cands[i], &found[i], ci, edge) {
                layer.push((i, ci));
            }
        }
        if layer.is_empty() {
            break;
        }
        let mut pe = vec![None; n];
        for &(i, ci) in &layer {
            let b = &cands[i][ci].bbox;
            pe[i] = Some(match edge {
                Edge::Top => b.y,
                Edge::Bottom => b.y + b.height,
            });
            found[i].insert(ci);
        }
        prev_edge = Some(pe);
    }
    found
}

/// Copies of an established constant running line that the ±2 page window missed (head variants
/// alternating in runs: "A B B … B A A A"): an undetected in-band candidate of the same edge with
/// the exact text of a line detected on at least [`MIN_SUPPORT`] pages, [`similar`] to one of its
/// first [`MAX_COPIES`] copies (size, overlapping box) and passing the gate. Numbered lines differ
/// in text from page to page and are never propagated.
fn propagate(cands: &[Vec<Cand>], heights: &[f32], found: &mut [HashSet<usize>], edge: Edge) {
    let mut lines: HashMap<String, (usize, Vec<(usize, usize)>)> = HashMap::new();
    for (i, f) in found.iter().enumerate() {
        let mut f: Vec<usize> = f.iter().copied().collect();
        f.sort_unstable();
        let mut texts: HashSet<String> = HashSet::new();
        for ci in f {
            let t = cands[i][ci].key.norm.clone();
            let e = lines.entry(t.clone()).or_default();
            if texts.insert(t) {
                e.0 += 1; // pages
            }
            if e.1.len() < MAX_COPIES {
                e.1.push((i, ci));
            }
        }
    }
    lines.retain(|_, (pages, _)| *pages >= MIN_SUPPORT);
    if lines.is_empty() {
        return;
    }
    for i in 0..cands.len() {
        let add: Vec<usize> = (0..cands[i].len())
            .filter(|ci| !found[i].contains(ci))
            .filter(|&ci| {
                let c = &cands[i][ci];
                in_band(c, edge, heights[i])
                    && lines
                        .get(&c.key.norm)
                        .is_some_and(|(_, v)| v.iter().any(|&(j, cj)| similar(c, &cands[j][cj], 0)))
                    && passes_gate(c, &cands[i], &found[i], ci, edge)
            })
            .collect();
        found[i].extend(add);
    }
}

/// Paragraphs of one page to move out of the body: `(region, span)` indices and the target role.
pub(crate) type Moves = Vec<Move>;
/// One paragraph to move: its `(region, span)` indices and the target role.
pub(crate) type Move = (Vec<(usize, usize)>, RegionRole);

/// A page number ("12", "Page 3 of 9", "стр. 4", "3/9"): digits masked, nothing else but `page`,
/// `стр.`, `of` and `/`.
fn is_folio(mask: &str) -> bool {
    mask.contains('#')
        && mask
            .split(|c: char| c.is_whitespace() || c == '#' || c == '/')
            .all(|w| matches!(w, "" | "page" | "стр." | "of"))
}

/// Running header/footer paragraphs per page, to apply with [`move_out`] page by page (the caller
/// builds one changed page at a time). The first occurrence of each text (digits masked) in the
/// document stays in the body, so a title repeated as a running head keeps its first copy; page
/// numbers ([`is_folio`]), numbered running lines (copies differ in a number: "… 3 / 12") and
/// constant lines detected on at least [`MIN_SUPPORT`] pages are marked from their first page on. In a single-page document a page
/// number at the end of the outermost row of an edge is marked alone ([`outer_folio`]). Each page
/// comes with its tables: their cells are never running heads. Pages are read one at a time (a page built for the call can be dropped right
/// after). Memory O(pages × edge-band candidates): candidate texts (with their normalized keys) and span indices, no page
/// copies; work O(pages × layers).
pub(crate) fn mark_running<'t, P: Borrow<StructuredPage>>(
    pages: impl IntoIterator<Item = (P, &'t [DetectedTable])>,
    body: f32,
) -> Vec<Moves> {
    let mut cands: Vec<Vec<Cand>> = Vec::new();
    let mut heights: Vec<f32> = Vec::new();
    // first page: the page number at the end of the outermost in-band row of each edge
    let mut outer: Vec<Move> = Vec::new();
    for (page, tables) in pages {
        let page = page.borrow();
        let c = candidates(page, tables, body);
        let h = page.page_height;
        if cands.is_empty() {
            outer = [
                (Edge::Top, RegionRole::Header),
                (Edge::Bottom, RegionRole::Footer),
            ]
            .into_iter()
            .filter_map(|(edge, role)| {
                let ci = *edge_order(&c, edge).first()?;
                in_band(&c[ci], edge, h).then_some(())?;
                Some((outer_folio(page, &c[ci], edge)?, role))
            })
            .collect();
        }
        cands.push(c);
        heights.push(h);
    }
    let mut head = detect_edge(&cands, &heights, Edge::Top);
    let mut foot = detect_edge(&cands, &heights, Edge::Bottom);
    propagate(&cands, &heights, &mut head, Edge::Top);
    propagate(&cands, &heights, &mut foot, Edge::Bottom);
    // per mask: pages it is detected on and its distinct texts
    let mut stats: HashMap<String, (HashSet<usize>, HashSet<String>)> = HashMap::new();
    for (pi, c) in cands.iter().enumerate() {
        for &ci in head[pi].iter().chain(&foot[pi]) {
            let k = &c[ci].key;
            let e = stats.entry(k.mask.clone()).or_default();
            e.0.insert(pi);
            e.1.insert(k.norm.clone());
        }
    }
    let keeps_first = |mask: &str| {
        !is_folio(mask)
            && stats
                .get(mask)
                .is_none_or(|(pages, texts)| texts.len() < 2 && pages.len() < MIN_SUPPORT)
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(cands.len());
    let single = cands.len() == 1;
    for (pi, cands) in cands.into_iter().enumerate() {
        let mut all: Vec<(usize, RegionRole)> = head[pi]
            .iter()
            .map(|&c| (c, RegionRole::Header))
            .chain(
                foot[pi]
                    .iter()
                    .filter(|c| !head[pi].contains(c))
                    .map(|&c| (c, RegionRole::Footer)),
            )
            .collect();
        all.sort_by_key(|(c, _)| *c);
        let mut cands: Vec<Option<Cand>> = cands.into_iter().map(Some).collect();
        let mut moves = Moves::new();
        for (ci, role) in all {
            let Some(c) = cands.get_mut(ci).and_then(Option::take) else {
                continue;
            };
            if keeps_first(&c.key.mask) && seen.insert(c.key.mask.clone()) {
                cands[ci] = Some(c); // first occurrence stays body text
                continue;
            }
            moves.push((c.spans, role));
        }
        if single {
            moves.extend(std::mem::take(&mut outer));
        }
        out.push(moves);
    }
    out
}

/// `page` with the `moves` paragraphs in `Header` (first) and `Footer` (last) regions; built from
/// the page fields, unchanged regions are copied once.
pub(crate) fn move_out(
    page: &StructuredPage,
    moves: &[(Vec<(usize, usize)>, RegionRole)],
) -> StructuredPage {
    let mut taken: HashSet<(usize, usize)> = HashSet::new();
    let mut heads = Vec::new();
    let mut foots = Vec::new();
    for (idx, role) in moves {
        let spans: Vec<TextSpan> = idx
            .iter()
            .filter_map(|&(r, s)| page.regions.get(r)?.spans.get(s).cloned())
            .collect();
        taken.extend(idx.iter().copied());
        let Some(like) = idx.first().and_then(|&(r, _)| page.regions.get(r)) else {
            continue;
        };
        let r = region_of(role.clone(), like, spans);
        if *role == RegionRole::Header {
            heads.push(r);
        } else {
            foots.push(r);
        }
    }
    let mut regions = heads;
    for (ri, r) in page.regions.iter().enumerate() {
        if !(0..r.spans.len()).any(|si| taken.contains(&(ri, si))) {
            regions.push(r.clone());
            continue;
        }
        let rest: Vec<TextSpan> = r
            .spans
            .iter()
            .enumerate()
            .filter(|(si, _)| !taken.contains(&(ri, *si)))
            .map(|(_, s)| s.clone())
            .collect();
        if rest.iter().any(|s| !s.text.trim().is_empty()) {
            regions.push(region_of(r.kind.clone(), r, rest));
        }
    }
    regions.extend(foots);
    StructuredPage {
        page_index: page.page_index,
        page_width: page.page_width,
        page_height: page.page_height,
        regions,
    }
}

/// A region with `spans`, text joined like the fork (trimmed span texts, single spaces) less the
/// spaces between touching spans at punctuation ([`super::spacing::tighten`]).
fn region_of(kind: RegionRole, like: &StructuredRegion, spans: Vec<TextSpan>) -> StructuredRegion {
    let bbox = spans
        .iter()
        .skip(1)
        .fold(spans.first().map_or(like.bbox, |s| s.bbox), |b, s| {
            b.union(&s.bbox)
        });
    let text = spans
        .iter()
        .map(|s| s.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut r = StructuredRegion {
        column_index: if kind == RegionRole::BodyBlock || kind == RegionRole::MarginalLabel {
            like.column_index
        } else {
            None
        },
        kind,
        text,
        bbox,
        spans,
        section_id: like.section_id,
    };
    super::spacing::tighten(&mut r);
    r
}

#[cfg(test)]
#[path = "running_test.rs"]
mod tests;
