//! Structured regions to blocks: column order, body font size, headings (`super::headings`), notes.

use std::collections::BTreeMap;

use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::TextSpan;
use pdf_oxide::{RegionRole, StructuredPage};
use plaindoc_core::{BBox, Block, NoteKind};

use super::bullets::{Bullet, vector_marks};
use super::columns::{column_order, is_column_region, split_gutters};
use super::fields::rewrite_fields;
use super::form_labels::field_labels;
use super::heading_rank::RankedHead;
use super::headings::{
    FontStats, HEADING_THRESHOLD, Para, StyledLine, baseline_neighbours, numbering_depth,
    paragraphs, passes_filters, scores, split_points, styled_lines,
};
use super::headings_number::{numbered_heading, numbered_level};
use super::headings_style::{caps_heading, link_shaped, title_text};
use super::lists::{Piece, group, sequence_cuts};
use super::outline::{Outline, mark_paragraphs};
use super::tables::DetectedTable;

/// Order of the blocks of a page.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReadingOrderMode {
    /// The order of the fork's regions, column regions left before right.
    Regions,
    /// Recursive XY-cut over the finished blocks of a page with columns (paragraphs, headings,
    /// tables), with two columns the fork left inside one region split first; running heads and
    /// page numbers keep their places, one-column pages keep the region order.
    #[default]
    XyCut,
}

/// Heading level and reading order settings.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutConfig {
    /// Minimum size ratio to the body for a level-2 heading of a page laid out alone; in a
    /// document, the largest headings at this ratio or more are level 1 (a title), the next sizes
    /// follow by rank.
    pub h2_ratio: f32,
    /// Minimum size ratio to the body for a level-1 heading of a page laid out alone.
    pub h1_ratio: f32,
    /// Order of the blocks of a page.
    pub reading_order: ReadingOrderMode,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            h2_ratio: 1.45,
            h1_ratio: 1.8,
            reading_order: ReadingOrderMode::default(),
        }
    }
}

pub(crate) fn non_ws_chars(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

pub(crate) fn to_bbox(r: &Rect) -> BBox {
    BBox::new(r.x, r.y, r.width, r.height)
}

pub(crate) struct Line {
    pub text: String,
    pub size: f32,
    pub bbox: Rect,
}

/// Spans arrive in reading order. A span joins the current line when its vertical centre is
/// within half of the smaller height of the line's first span (no x check: drop caps like
/// "Г" + "одовой" start left of the running right edge).
pub(crate) fn group_lines(spans: &[TextSpan]) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let (mut anchor_yc, mut anchor_h, mut right) = (0.0f32, 0.0f32, 0.0f32);
    for s in spans.iter().filter(|s| !s.text.trim().is_empty()) {
        let yc = s.bbox.y + s.bbox.height / 2.0;
        if let Some(l) = lines.last_mut()
            && (yc - anchor_yc).abs() < 0.5 * anchor_h.min(s.bbox.height)
        {
            if s.bbox.x - right > 0.15 * s.font_size
                && !l.text.ends_with(' ')
                && !s.text.starts_with(' ')
            {
                l.text.push(' ');
            }
            l.text.push_str(&s.text);
            l.size = l.size.max(s.font_size);
            l.bbox = l.bbox.union(&s.bbox);
            right = s.bbox.x + s.bbox.width;
            continue;
        }
        (anchor_yc, anchor_h, right) = (yc, s.bbox.height, s.bbox.x + s.bbox.width);
        lines.push(Line {
            text: s.text.clone(),
            size: s.font_size,
            bbox: s.bbox,
        });
    }
    for l in &mut lines {
        l.text = l.text.trim().to_string();
    }
    lines
}

/// Char-weighted mode of BodyBlock span font sizes, bucketed to 0.5 pt; ties go to the smaller size.
pub(crate) fn body_font_size(pages: &[&StructuredPage]) -> f32 {
    let mut hist: BTreeMap<i32, usize> = BTreeMap::new();
    for p in pages {
        for r in p.regions.iter().filter(|r| r.kind == RegionRole::BodyBlock) {
            for s in &r.spans {
                *hist.entry((s.font_size * 2.0).round() as i32).or_default() +=
                    non_ws_chars(&s.text);
            }
        }
    }
    hist.into_iter()
        .filter(|(_, n)| *n > 0)
        .max_by_key(|(k, n)| (*n, -*k))
        .map(|(k, _)| k as f32 / 2.0)
        .unwrap_or(0.0)
}

/// Blocks of one page plus what was dropped (artifact regions and their non-whitespace chars).
pub(crate) struct PageBlocks {
    pub blocks: Vec<Block>,
    /// The page has two columns: the fork's column regions, or rows split by [`split_gutters`].
    pub columns: bool,
    pub dropped_regions: usize,
    pub dropped_chars: usize,
}

/// Level of a detected heading: by size ratio to the body (h1/h2 thresholds), else by numbering
/// depth ("3" → 2, "3.1" → 3, "3.1.2" → 4), else 3.
fn styled_level(p: &Para, body: f32, cfg: &LayoutConfig) -> u8 {
    let ratio = if body > 0.0 { p.size / body } else { 0.0 };
    if ratio.is_finite() && ratio >= cfg.h1_ratio {
        1
    } else if ratio.is_finite() && ratio >= cfg.h2_ratio {
        2
    } else {
        numbering_depth(&p.text).map_or(3, |d| (d + 1).min(6) as u8)
    }
}

/// Whether a table's text area lies vertically between two consecutive paragraphs (PDF coordinates,
/// y up) and overlaps both in x: the table text was removed from the prose, so the paragraphs are
/// not neighbours.
fn table_between(tables: &[DetectedTable], above: &Para, below: &Para) -> bool {
    let x_overlap = |t: &BBox, p: &Para| t.x < p.bbox.x + p.bbox.width && p.bbox.x < t.x + t.w;
    tables.iter().any(|t| {
        let t = &t.text_bbox;
        t.y + t.h <= above.bbox.y
            && t.y >= below.bbox.y + below.bbox.height
            && x_overlap(t, above)
            && x_overlap(t, below)
    })
}

/// The engine text of a region without headings, cut before the lines in `cuts`: each paragraph is
/// a slice of `r.text` (its non-whitespace chars are those of the lines, in order). One paragraph
/// when there are no cuts or the text does not match the lines.
fn cut_region(r: &pdf_oxide::StructuredRegion, lines: &[StyledLine], cuts: &[usize]) -> Vec<Piece> {
    let counts: Vec<usize> = lines.iter().map(|l| non_ws_chars(&l.text)).collect();
    if cuts.is_empty() || counts.iter().sum::<usize>() != non_ws_chars(&r.text) {
        return vec![Piece {
            block: Block::paragraph(r.text.clone(), Some(to_bbox(&r.bbox))),
            lines: (!lines.is_empty()).then_some(0..lines.len()),
        }];
    }
    let pos: Vec<usize> = r
        .text
        .char_indices()
        .filter(|(_, c)| !c.is_whitespace())
        .map(|(i, _)| i)
        .collect();
    let starts: Vec<usize> = std::iter::once(0).chain(cuts.iter().copied()).collect();
    let ends = cuts.iter().copied().chain(std::iter::once(lines.len()));
    let mut before = 0; // non-whitespace chars before the current piece
    starts
        .iter()
        .zip(ends)
        .map(|(&a, b)| {
            let n: usize = counts[a..b].iter().sum();
            let from = if a == 0 {
                0
            } else {
                pos.get(before).copied().unwrap_or(r.text.len())
            };
            let to = pos.get(before + n).copied().unwrap_or(r.text.len());
            before += n;
            let rect = lines[a + 1..b]
                .iter()
                .fold(lines[a].bbox, |acc, l| acc.union(&l.bbox));
            Piece {
                block: Block::paragraph(r.text[from..to].trim(), Some(to_bbox(&rect))),
                lines: Some(a..b),
            }
        })
        .collect()
}

/// Facts of the page outside its body text: index, tables, the document outline; `ranked`
/// collects the style-detected headings whose level the document size rank sets
/// ([`super::heading_rank`]).
pub(crate) struct PageContext<'a> {
    pub page: usize,
    pub tables: &'a [DetectedTable],
    pub bullets: &'a [Bullet],
    pub outline: &'a mut Outline,
    pub ranked: Vec<RankedHead>,
}

/// Body regions of a page: headings from the outline ([`mark_paragraphs`]) and by
/// [`super::headings`] scores over all body paragraphs of the page in reading order; the rest is
/// cut into paragraphs at [`split_points`] (`pitch` is the document line pitch ratio).
fn styled_body_blocks(
    regions: &[&pdf_oxide::StructuredRegion],
    body: f32,
    stats: &FontStats,
    pitch: f32,
    ctx: &mut PageContext<'_>,
    cfg: &LayoutConfig,
) -> Vec<Vec<Block>> {
    let (tables, bullets) = (ctx.tables, ctx.bullets);
    let lines: Vec<Vec<StyledLine>> = regions.iter().map(|r| styled_lines(&r.spans)).collect();
    let (paras, marks): (Vec<Vec<Para>>, Vec<Vec<Option<u8>>>) = lines
        .iter()
        .map(|l| {
            let p = paragraphs(l);
            if ctx.outline.is_empty() {
                let n = p.len();
                (p, vec![None; n])
            } else {
                mark_paragraphs(l, p, ctx.page, body, ctx.outline)
            }
        })
        .unzip();
    let all_lines: Vec<&StyledLine> = lines.iter().flatten().collect();
    let neighbour = baseline_neighbours(&all_lines);
    let offsets: Vec<usize> = lines
        .iter()
        .scan(0, |acc, l| {
            let o = *acc;
            *acc += l.len();
            Some(o)
        })
        .collect();
    let flat: Vec<&Para> = paras.iter().flatten().collect();
    let breaks: Vec<bool> = (0..flat.len())
        .map(|i| i > 0 && table_between(tables, flat[i - 1], flat[i]))
        .collect();
    let score = scores(&flat, &breaks, stats);
    let numbered: Vec<bool> = flat
        .iter()
        .map(|p| numbering_depth(&p.text).is_some())
        .collect();
    let next_to_numbered =
        |k: usize| (k > 0 && numbered[k - 1]) || numbered.get(k + 1).copied().unwrap_or(false);
    let labels = field_labels(&flat, body);
    let mut k = 0;
    let mut out = Vec::with_capacity(regions.len());
    for (ri, r) in regions.iter().enumerate() {
        let heads: Vec<bool> = paras[ri]
            .iter()
            .zip(&marks[ri])
            .map(|(p, m)| {
                let (s, near, label) = (score[k], next_to_numbered(k), labels[k]);
                k += 1;
                (m.is_some()
                    || (s >= HEADING_THRESHOLD && passes_filters(p, stats))
                    || (!label
                        && (numbered_heading(p, s, stats, near)
                            || caps_heading(p, &lines[ri][p.lines.start], body))))
                    && (m.is_some() || !link_shaped(&p.text))
                    && !p.lines.clone().any(|i| neighbour[offsets[ri] + i])
            })
            .collect();
        let vector = &vector_marks(&lines[ri], bullets)[..];
        let cuts = sequence_cuts(&lines[ri], &split_points(&lines[ri], pitch), vector);
        if !heads.iter().any(|h| *h) {
            out.push(group(cut_region(r, &lines[ri], &cuts), &lines[ri], vector));
            continue;
        }
        let mut pieces = Vec::new();
        type Run = Option<(String, Rect, std::ops::Range<usize>)>;
        let flush = |run: &mut Run, pieces: &mut Vec<Piece>| {
            if let Some((text, rect, range)) = run.take() {
                pieces.push(Piece {
                    block: Block::paragraph(text, Some(to_bbox(&rect))),
                    lines: Some(range),
                });
            }
        };
        let mut run: Run = None;
        for ((p, h), m) in paras[ri].iter().zip(heads).zip(&marks[ri]) {
            if h {
                flush(&mut run, &mut pieces);
                let numbered = numbered_level(&p.text);
                if m.is_none() && numbered.is_none() {
                    ctx.ranked.push(RankedHead {
                        bbox: to_bbox(&p.bbox),
                        size: p.size,
                    });
                }
                pieces.push(Piece {
                    block: Block::heading(
                        m.or(numbered).unwrap_or_else(|| styled_level(p, body, cfg)),
                        title_text(p, &lines[ri], 0).into_owned(),
                        Some(to_bbox(&p.bbox)),
                    ),
                    lines: None,
                });
                continue;
            }
            for i in p.lines.clone() {
                let l = &lines[ri][i];
                if cuts.binary_search(&i).is_err()
                    && let Some((text, rect, range)) = &mut run
                {
                    range.end = i + 1;
                    text.push(' ');
                    text.push_str(&l.text);
                    *rect = rect.union(&l.bbox);
                } else {
                    flush(&mut run, &mut pieces);
                    run = Some((l.text.clone(), l.bbox, i..i + 1));
                }
            }
        }
        flush(&mut run, &mut pieces);
        out.push(group(pieces, &lines[ri], vector));
    }
    out
}

/// [`page_to_blocks_with`] with font statistics of this page only (tests, single pages).
#[cfg(test)]
pub(crate) fn page_to_blocks(
    page: &StructuredPage,
    body_size: f32,
    cfg: &LayoutConfig,
) -> PageBlocks {
    page_to_blocks_with(
        page,
        body_size,
        &super::headings::font_stats(&[page]),
        super::headings::line_pitch_ratio(&[page]),
        &mut PageContext {
            page: page.page_index,
            tables: &[],
            outline: &mut Outline::default(),
            bullets: &[],
            ranked: Vec::new(),
        },
        cfg,
    )
}

pub(crate) fn page_to_blocks_with(
    page: &StructuredPage,
    body_size: f32,
    stats: &FontStats,
    pitch: f32,
    ctx: &mut PageContext<'_>,
    cfg: &LayoutConfig,
) -> PageBlocks {
    // form fields: values after their labels, field rows as paragraphs
    let fields = rewrite_fields(page);
    let page = fields.as_ref().unwrap_or(page);
    // the XY-cut order also reads two columns the fork left inside one-column regions
    let split = (cfg.reading_order == ReadingOrderMode::XyCut)
        .then(|| split_gutters(page))
        .flatten();
    let page = split.as_ref().unwrap_or(page);
    let mut pb = PageBlocks {
        blocks: Vec::new(),
        columns: split.is_some() || page.regions.iter().any(is_column_region),
        dropped_regions: 0,
        dropped_chars: 0,
    };
    let order: Vec<&pdf_oxide::StructuredRegion> = column_order(&page.regions)
        .into_iter()
        .filter(|r| !r.text.trim().is_empty())
        .collect();
    let bodies: Vec<&pdf_oxide::StructuredRegion> = order
        .iter()
        .copied()
        .filter(|r| r.kind == RegionRole::BodyBlock)
        .collect();
    let mut styled = styled_body_blocks(&bodies, body_size, stats, pitch, ctx, cfg).into_iter();
    for region in order {
        let bbox = Some(to_bbox(&region.bbox));
        let note = |kind| Block::Note {
            kind,
            text: region.text.clone(),
            bbox,
        };
        match &region.kind {
            RegionRole::BodyBlock => pb.blocks.extend(styled.next().unwrap_or_default()),
            RegionRole::StructuralHeading { level } => pb.blocks.push(Block::heading(
                (*level).clamp(1, 6),
                region.text.clone(),
                bbox,
            )),
            RegionRole::Header => pb.blocks.push(note(NoteKind::Header)),
            RegionRole::Footer => pb.blocks.push(note(NoteKind::Footer)),
            RegionRole::PageNumber => pb.blocks.push(note(NoteKind::PageNumber)),
            RegionRole::MarginalLabel => pb.blocks.push(note(NoteKind::Marginal)),
            RegionRole::Artifact => {
                pb.dropped_regions += 1;
                pb.dropped_chars += non_ws_chars(&region.text);
            }
        }
    }
    pb
}

/// Sorted non-whitespace chars.
#[cfg(test)]
pub(crate) fn chars_of(s: &str) -> Vec<char> {
    let mut v: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
    v.sort_unstable();
    v
}

#[cfg(test)]
pub(crate) fn blocks_text(blocks: &[Block]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            Block::Heading { text, .. }
            | Block::Paragraph { text, .. }
            | Block::Note { text, .. } => Some(text.clone()),
            Block::List { items, .. } => Some(
                items
                    .iter()
                    .map(|i| blocks_text(&i.blocks))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "layout_test.rs"]
mod tests;
