//! Lists in PDF text (SP8b S1): paragraphs of one body region that start with a list marker become
//! `Block::List` (the office model, D5: item text first, a nested list inside its parent item).
//!
//! Item boundaries are the paragraph breaks of `headings::split_points` (its `list` rule already
//! cuts before marker lines), plus [`sequence_cuts`] (a number or letter that continues the
//! sequence of the line above at the same x) and lines with a vector bullet. Then this module
//! labels pieces:
//! - markers (`super::list_marker`): bullets `•●▪◦►▸‣∙` (the text may follow without a space);
//!   check boxes `■□○` (kept in the text: a form shows which option is marked); dashes `–—-`,
//!   `*` and `·` (whitespace after; a run of 2, as `·` also separates words inline); numbers
//!   `1.`/`1)` (1–3 digits); one lower-case Latin or Cyrillic letter `a)`/`а.`; a vector bullet
//!   (a small filled box left of the line, `super::bullets`);
//! - siblings (consecutive items of one level and marker class under one parent) stay a list when
//!   one of them has a symbol bullet or a check box, or there are at least 2 dash items (1 when
//!   nested in a kept item; not dash lines after a finished sentence with no item ending in `;` or
//!   `,`: a dialogue; a lower-case dash line under a full line that does not end a phrase is a
//!   wrapped clause, not an item), or at
//!   least 2 vector bullets of one colour and size (a chart legend has a colour per swatch), or at
//!   least 2 numbers / letters in sequence (`1, 2, 3`, `а, б`; one skipped number or letter is
//!   allowed, and Cyrillic letters may follow GOST 2.105, `ж)` then `и)`) with one separator. A lone `1. Введение`,
//!   `5. Sample…` or `— text` stays a paragraph, as before;
//! - levels by marker x: deeper by more than [`LEVEL_FS`] font sizes, up to [`MAX_LEVELS`];
//! - a piece without a marker whose lines all start right of an open item's marker continues that
//!   item (one more paragraph of it; a one-line piece only when it goes on in lower case); a
//!   paragraph with a first-line indent comes back to the margin and ends the list, as does a
//!   heading;
//! - numbers 1, 2, 3… without a gap make an ordered list without the markers; other numbers and
//!   letters stay in the item text of an unordered list (the renderer numbers ordered items from 1).

use plaindoc_core::{BBox, Block, ListItem};

use super::headings::StyledLine;
pub(crate) use super::list_marker::{Kind, LEVEL_FS, Marker, marker, sequence_cuts};

/// Nesting depth (top level included).
const MAX_LEVELS: usize = 3;
/// A piece continues an item when it starts this many font sizes right of the item's marker.
const CONT_FS: f32 = 0.5;

/// One block of a region and its lines.
pub(crate) struct Piece {
    pub block: Block,
    /// Its lines in the region's lines; `None` for a heading.
    pub lines: Option<std::ops::Range<usize>>,
}

struct Item {
    marker: Marker,
    /// Marker left edge, the estimated left edge of the item text, and font size.
    x: f32,
    text_x: f32,
    size: f32,
    /// The marker is `–` or `—` (the dashes of a dialogue).
    long_dash: bool,
    level: usize,
    /// Sibling group (see [`groups`]).
    group: usize,
    /// Last char of the item's text, and of the plain paragraph just before it (`None` when the
    /// item follows a list piece, a heading or nothing).
    end: Option<char>,
    lead: Option<char>,
}

/// Ends a sentence (a dialogue line or prose before a dash, not an enumeration's `:`).
const SENTENCE_END: &str = ".!?…";
/// Ends a phrase (a dash on the next line opens an item, not a wrapped clause).
const PHRASE_END: &str = ".!?…:;";

fn last_char(b: &Block) -> Option<char> {
    match b {
        Block::Paragraph { text, .. } => text.trim_end().chars().last(),
        _ => None,
    }
}

/// What a piece is to the lists.
#[derive(Clone, Copy, PartialEq)]
enum Role {
    Plain,
    /// Starts item `i`.
    Item(usize),
    /// Continues item `i`.
    More(usize),
}

/// Blocks of a region with list pieces grouped into lists. `vector[i]`: x and key of a vector
/// bullet left of line `i` (used when the line has no text marker).
pub(crate) fn group(
    pieces: Vec<Piece>,
    lines: &[StyledLine],
    vector: &[Option<(f32, u64)>],
) -> Vec<Block> {
    let mut items: Vec<Item> = Vec::new();
    let mut roles = vec![Role::Plain; pieces.len()];
    let mut starts = Vec::new(); // per item: it opens a run of list pieces
    let mut seg = 0; // first item of the current run of list pieces
    // the right edge of the region's text: the 90th percentile of line ends
    let mut rights: Vec<f32> = lines.iter().map(|l| l.bbox.x + l.bbox.width).collect();
    rights.sort_by(f32::total_cmp);
    let right = rights
        .get((rights.len().saturating_sub(1) as f32 * 0.9) as usize)
        .copied()
        .unwrap_or(0.0);
    for (k, p) in pieces.iter().enumerate() {
        let first = p.lines.as_ref().map(|r| r.start);
        let line = first.and_then(|f| lines.get(f));
        let (Some(line), Block::Paragraph { text, .. }) = (line, &p.block) else {
            seg = items.len();
            continue;
        };
        let size = line.size.max(1.0);
        let vec_x = first.and_then(|f| vector.get(f).copied().flatten());
        // a lower-case dash line under a full line of plain text that does not end a phrase: a
        // clause wrapped before its dash (`… веков` / `— это …`), not an item
        let wrapped = |m: &Marker| {
            let prev = first
                .and_then(|f| f.checked_sub(1))
                .and_then(|f| lines.get(f));
            m.kind == Kind::Dash
                && k > 0
                && roles[k - 1] == Role::Plain
                && text[m.len..].starts_with(char::is_lowercase)
                && prev.is_some_and(|l| {
                    !l.text.trim_end().ends_with(|c| PHRASE_END.contains(c))
                        && l.bbox.x + l.bbox.width >= right - 3.0 * l.size
                })
        };
        let m = match (marker(text).filter(|m| !wrapped(m)), vec_x) {
            (Some(m), _) => Some((m, line.bbox.x)),
            (None, Some((x, key))) => Some((
                Marker {
                    kind: Kind::Vector(key),
                    sep: ' ',
                    len: 0,
                },
                x,
            )),
            (None, None) => None,
        };
        if let Some((marker, x)) = m {
            let level = level_of(&items[seg..], x, size);
            roles[k] = Role::Item(items.len());
            let after_plain = k == 0 || roles[k - 1] == Role::Plain;
            starts.push(after_plain);
            // the item text starts after the marker: its share of the first line's width
            let chars = line.text.chars().count().max(1) as f32;
            let lead_chars = text[..marker.len].chars().count() as f32;
            items.push(Item {
                marker,
                x,
                text_x: line.bbox.x + line.bbox.width * lead_chars / chars,
                size,
                long_dash: text.trim_start().starts_with(['–', '—']),
                level,
                group: 0,
                end: last_char(&p.block),
                lead: (k > 0 && after_plain)
                    .then(|| last_char(&pieces[k - 1].block))
                    .flatten(),
            });
            continue;
        }
        // a continuation hangs: every line right of the item's marker (a paragraph with a first-line
        // indent comes back to the margin); a one-line piece must also start at the item's text
        // (within `LEVEL_FS`) or go on in lower case
        let span = p.lines.clone().unwrap_or_default();
        let one_line = span.len() <= 1;
        let lower = text.trim_start().starts_with(char::is_lowercase);
        let left = lines[span]
            .iter()
            .map(|l| l.bbox.x)
            .fold(f32::INFINITY, f32::min);
        let host = items[seg..]
            .iter()
            .rposition(|it| left > it.x + CONT_FS * it.size)
            .map(|i| seg + i)
            .filter(|&h| {
                let it = &items[h];
                !one_line || lower || (left - it.text_x).abs() <= LEVEL_FS * it.size
            });
        match host {
            Some(h) if k > 0 && roles[k - 1] != Role::Plain => roles[k] = Role::More(h),
            _ => seg = items.len(),
        }
    }
    let keep = groups(&mut items, &starts);
    build(pieces, &items, &roles, &keep)
}

/// Level of a marker at `x` after the items of its run: the level of the last item at the same x
/// (within [`LEVEL_FS`]), one deeper than the last item left of it, else 0.
fn level_of(run: &[Item], x: f32, size: f32) -> usize {
    let tol = LEVEL_FS * size;
    for it in run.iter().rev() {
        if (x - it.x).abs() <= tol {
            return it.level;
        }
        if x > it.x + tol {
            return (it.level + 1).min(MAX_LEVELS - 1);
        }
    }
    0
}

/// Sets each item's sibling group (`starts[i]`: item `i` opens a run of list pieces) and returns
/// whether the item stays an item (the rule of the module docs).
fn groups(items: &mut [Item], starts: &[bool]) -> Vec<bool> {
    let mut next = 0;
    // open[level] = (group, class, item) of the last item at that level in the current run
    let mut open: Vec<Option<(usize, u8, usize)>> = vec![None; MAX_LEVELS];
    let mut parent = vec![None; items.len()];
    for i in 0..items.len() {
        if starts[i] {
            open = vec![None; MAX_LEVELS];
        }
        let (level, class) = (items[i].level, items[i].marker.class());
        for o in open.iter_mut().skip(level + 1) {
            *o = None; // a shallower item closes the deeper lists
        }
        parent[i] = open[..level].iter().rev().find_map(|o| o.map(|o| o.2));
        items[i].group = match open[level] {
            Some((g, c, _)) if c == class => g,
            _ => {
                next += 1;
                next
            }
        };
        open[level] = Some((items[i].group, class, i));
    }
    let mut keep = vec![false; items.len()];
    // groups are numbered in item order, so a parent's group is decided before its children's
    for g in 1..=next {
        let ids: Vec<usize> = (0..items.len()).filter(|&i| items[i].group == g).collect();
        let ms: Vec<&Marker> = ids.iter().map(|&i| &items[i].marker).collect();
        let first = &items[ids[0]];
        let ok = match ms[0].kind {
            Kind::Symbol | Kind::Dash | Kind::Box => {
                let dashes = ms.iter().all(|m| m.kind == Kind::Dash);
                // dialogue or prose: `–`/`—` lines after a finished sentence, none ending as an
                // enumeration item (`;`, `,`)
                let dialogue = ids.iter().all(|&i| items[i].long_dash)
                    && first.level == 0
                    && first.lead.is_some_and(|c| SENTENCE_END.contains(c))
                    && !ids[..ids.len() - 1]
                        .iter()
                        .any(|&i| items[i].end.is_some_and(|c| ";,".contains(c)));
                let nested = first.level > 0 && parent[ids[0]].is_some_and(|p| keep[p]);
                !dialogue && (ms.len() >= 2 || nested || !dashes)
            }
            Kind::Vector(_) => ms.len() >= 2 && ms.windows(2).all(|w| w[0].kind == w[1].kind),
            // a sequence with at most one number or letter skipped (`1, 2, 3, 5`)
            _ => {
                let steps: Vec<Option<u32>> = ms.windows(2).map(|w| w[0].step(w[1])).collect();
                ms.len() >= 2
                    && ms.windows(2).all(|w| w[0].precedes(w[1]))
                    && steps.iter().filter(|s| **s == Some(2)).count() <= 1
            }
        };
        for &i in &ids {
            keep[i] = ok;
        }
    }
    keep
}

/// An open list while building: its level, sibling group, items, ordered flag and box.
struct Open {
    level: usize,
    group: usize,
    ordered: bool,
    items: Vec<ListItem>,
    bbox: Option<BBox>,
}

fn union(a: Option<BBox>, b: Option<BBox>) -> Option<BBox> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
            let (x1, y1) = ((a.x + a.w).max(b.x + b.w), (a.y + a.h).max(b.y + b.h));
            Some(BBox::new(x0, y0, x1 - x0, y1 - y0))
        }
        (a, b) => a.or(b),
    }
}

fn bbox_of(b: &Block) -> Option<BBox> {
    match b {
        Block::Paragraph { bbox, .. } => *bbox,
        _ => None,
    }
}

/// Closes the innermost open list: into its parent's last item, or into `out`.
fn close(stack: &mut Vec<Open>, out: &mut Vec<Block>) {
    let Some(o) = stack.pop() else {
        return;
    };
    let block = Block::List {
        ordered: o.ordered,
        items: o.items,
        bbox: o.bbox,
    };
    match stack.last_mut() {
        Some(parent) => {
            parent.bbox = union(parent.bbox, o.bbox);
            if let Some(it) = parent.items.last_mut() {
                it.blocks.push(block);
            }
        }
        None => out.push(block),
    }
}

/// Adds a paragraph to the last item of the innermost open list.
fn add_to_last(stack: &mut [Open], block: Block) {
    if let Some(o) = stack.last_mut() {
        o.bbox = union(o.bbox, bbox_of(&block));
        if let Some(it) = o.items.last_mut() {
            it.blocks.push(block);
        }
    }
}

fn build(pieces: Vec<Piece>, items: &[Item], roles: &[Role], keep: &[bool]) -> Vec<Block> {
    // group -> ordered (numbers 1, 2, 3… without a gap: the renderer numbers them again)
    let ordered = |g: usize| {
        let ms: Vec<&Marker> = items
            .iter()
            .filter(|it| it.group == g)
            .map(|it| &it.marker)
            .collect();
        ms[0].kind == Kind::Number(1) && ms.windows(2).all(|w| w[0].step(w[1]) == Some(1))
    };
    let mut out = Vec::with_capacity(pieces.len());
    let mut stack: Vec<Open> = Vec::new();
    for (piece, role) in pieces.into_iter().zip(roles) {
        match *role {
            Role::Item(i) if keep[i] => {
                let it = &items[i];
                while stack.last().is_some_and(|o| o.level > it.level) {
                    close(&mut stack, &mut out);
                }
                if stack
                    .last()
                    .is_some_and(|o| o.level == it.level && o.group != it.group)
                {
                    close(&mut stack, &mut out);
                }
                if stack.last().is_none_or(|o| o.level < it.level) {
                    stack.push(Open {
                        level: it.level,
                        group: it.group,
                        ordered: ordered(it.group),
                        items: Vec::new(),
                        bbox: None,
                    });
                }
                let o = stack.last_mut().expect("pushed above");
                let strip = matches!(it.marker.kind, Kind::Symbol | Kind::Dash | Kind::Vector(_))
                    || o.ordered;
                let block = match piece.block {
                    Block::Paragraph { text, bbox, links } if strip => Block::Paragraph {
                        text: text.get(it.marker.len..).unwrap_or(&text).to_string(),
                        bbox,
                        links,
                    },
                    b => b,
                };
                o.bbox = union(o.bbox, bbox_of(&block));
                let mut li = ListItem::new("");
                li.blocks.push(block);
                o.items.push(li);
            }
            Role::More(i) if keep[i] => {
                while stack.last().is_some_and(|o| o.level > items[i].level) {
                    close(&mut stack, &mut out);
                }
                add_to_last(&mut stack, piece.block);
            }
            // an item or continuation that is not kept: a paragraph of the open item, if any
            Role::Item(i) | Role::More(i) if !stack.is_empty() && items[i].level > 0 => {
                add_to_last(&mut stack, piece.block)
            }
            _ => {
                while !stack.is_empty() {
                    close(&mut stack, &mut out);
                }
                out.push(piece.block);
            }
        }
    }
    while !stack.is_empty() {
        close(&mut stack, &mut out);
    }
    out
}

#[cfg(test)]
#[path = "lists_test.rs"]
mod tests;
