//! Reading order of finished page blocks (paragraphs, headings, tables, lists) by recursive XY-cut
//! with column rules after OpenDataLoader (ideas only): cuts need a gap >= 5 pt; a vertical gap
//! (a gutter) is cut first; blocks narrower than 10 % of the region are ignored when looking for
//! a gutter. Where no gutter runs through the node, the widest horizontal gap is cut, except a gap
//! of at most 2 font sizes touched by two side-by-side blocks (ODL's paragraph break inside
//! columns) and a gap with columns right above and right below it (paragraph breaks or figures of
//! two columns that line up). A node without a cut keeps its input order (the caller's region
//! order). Running headers/footers keep their places (`order_blocks`).
//!
//! The ODL rule "the larger gap wins" and the top-down sort of uncut nodes were measured on the SP8
//! corpora (task 9) and replaced: aligned paragraph gaps of two columns cut the columns into rows
//! (ODL 013, 041, 042) and a right-column figure caption came before the left column (PT1M), and
//! the top-down sort interleaved the columns of nodes that a full-width block keeps from cutting.
//!
//! Service limits: at most [`MAX_BLOCKS`] blocks (else the input order is kept and the caller warns)
//! and recursion depth [`MAX_DEPTH`]. A node sorts its blocks (O(n log n)) and checks each candidate
//! row gap against the blocks near it only (binary search in the blocks sorted by their edges). The
//! windows are bounded in points (`NEAR_FS` font sizes), not in blocks: with a large font size one
//! window can hold most of the node, so a gap costs O(log n + W log W) for W blocks in its windows,
//! and a node peeled one block per level repeats its rejected gaps at every level. The whole page is
//! therefore bounded by [`MAX_VISITS`] (blocks of every node plus window blocks of every tested gap):
//! when it runs out the page keeps its input order (`None`, the caller warns). The time budget
//! (<= 50 ms on the adversarial 2000-block pages) is the `xy_cut` bench in `benches/pdf.rs`. On
//! reaching `MAX_DEPTH` the rest keeps its input order.

use std::cell::Cell;

use plaindoc_core::{BBox, Block, NoteKind};

/// A block box in PDF coordinates (origin bottom-left, y up) with its font size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrderBox {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub font_size: f32,
}

pub(crate) const MIN_GAP: f32 = 5.0;
pub(crate) const PARAGRAPH_GAP_FS: f32 = 2.0;
pub(crate) const NARROW_SHARE: f32 = 0.1;
pub(crate) const MAX_DEPTH: usize = 40;
pub(crate) const MAX_BLOCKS: usize = 2_000;
/// Blocks this many font sizes above or below a horizontal gap decide whether it runs through
/// two columns.
pub(crate) const NEAR_FS: f32 = 6.0;
/// Work limit per page for [`xy_cut`]: blocks of every node plus blocks in the windows of every
/// tested row gap.
pub(crate) const MAX_VISITS: usize = 2_000_000;

/// Reading order as indices into `blocks`; `None` when the page exceeds [`MAX_BLOCKS`] or
/// [`MAX_VISITS`], or a box is not finite (the caller keeps the current order).
pub fn xy_cut(blocks: &[OrderBox]) -> Option<Vec<usize>> {
    if blocks.len() > MAX_BLOCKS
        || blocks.iter().any(|b| {
            ![b.x0, b.y0, b.x1, b.y1, b.font_size]
                .iter()
                .all(|v| v.is_finite())
        })
    {
        return None;
    }
    let mut out = Vec::with_capacity(blocks.len());
    let work = Cell::new(MAX_VISITS);
    cut(blocks, (0..blocks.len()).collect(), 0, &mut out, &work).then_some(out)
}

/// Gaps >= [`MIN_GAP`] between the projections `[lo, hi]` of `idx`: `(gap, cut position)`, widest
/// first (stable for equal gaps: lower position first).
fn gaps(idx: &[usize], span: impl Fn(usize) -> (f32, f32)) -> Vec<(f32, f32)> {
    let mut iv: Vec<(f32, f32)> = idx.iter().map(|&i| span(i)).collect();
    iv.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::new();
    let Some(&(_, first_hi)) = iv.first() else {
        return out;
    };
    let mut reach = first_hi;
    for &(lo, hi) in &iv[1..] {
        if lo - reach >= MIN_GAP {
            out.push((lo - reach, (lo + reach) / 2.0));
        }
        reach = reach.max(hi);
    }
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    out
}

/// Horizontal gaps >= [`MIN_GAP`] between the x-projections of `idx`, as `(left, right)` edges.
fn x_gaps(blocks: &[OrderBox], idx: &[usize]) -> Vec<(f32, f32)> {
    let mut iv: Vec<(f32, f32)> = idx.iter().map(|&i| (blocks[i].x0, blocks[i].x1)).collect();
    iv.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::new();
    let mut reach = f32::NEG_INFINITY;
    for (lo, hi) in iv {
        if reach.is_finite() && lo - reach >= MIN_GAP {
            out.push((reach, lo));
        }
        reach = reach.max(hi);
    }
    out
}

/// The widest row gap of the node `idx` that is a cut (`-y` of its middle), if any. The blocks are
/// sorted once by their lower and upper edges; each candidate gap looks only at the blocks within
/// `NEAR_FS` font sizes of it (binary search). That window is bounded in points, not in blocks (with
/// a large font size it holds most of the node, copied and sorted per gap), so every window block
/// counts against the page's work limit `work`; `Err` when it runs out.
fn row_cut(
    blocks: &[OrderBox],
    idx: &[usize],
    fs: f32,
    work: &Cell<usize>,
) -> Result<Option<f32>, ()> {
    // work spent so far: blocks of the windows (`spend`), checked after each gap
    let spend = |n: usize| work.set(work.get().saturating_sub(n));
    // (edge, position in `idx`), sorted by the edge
    let sorted = |key: fn(&OrderBox) -> f32| {
        let mut s: Vec<(f32, usize)> = (0..idx.len()).map(|k| (key(&blocks[idx[k]]), k)).collect();
        s.sort_unstable_by(|p, q| p.0.total_cmp(&q.0));
        s
    };
    let (by_y0, by_y1) = (sorted(|o| o.y0), sorted(|o| o.y1));
    // the entries of `s` from the first edge not `before` up to the last edge `upto` (both
    // monotone along `s`)
    fn window(
        s: &[(f32, usize)],
        before: impl Fn(f32) -> bool,
        upto: impl Fn(f32) -> bool,
    ) -> &[(f32, usize)] {
        let (lo, hi) = (
            s.partition_point(|e| before(e.0)),
            s.partition_point(|e| upto(e.0)),
        );
        &s[lo..hi.max(lo)]
    }
    // A small horizontal gap whose upper edge is touched by two side-by-side blocks is a paragraph
    // break inside columns, not a cut (y up: the cut lies at y = -c).
    let in_columns = |g: f32, c: f32| {
        if g > PARAGRAPH_GAP_FS * fs {
            return false;
        }
        let y = -c;
        let touching = window(&by_y0, |y0| y0 < y, |y0| y0 - y <= g / 2.0 + 1.0);
        spend(touching.len());
        // two of them side by side: some block ends left of where another starts
        let x = |&(_, k): &(f32, usize)| blocks[idx[k]];
        let right = touching
            .iter()
            .map(|e| x(e).x1)
            .fold(f32::INFINITY, f32::min);
        let left = touching
            .iter()
            .map(|e| x(e).x0)
            .fold(f32::NEG_INFINITY, f32::max);
        right < left
    };
    // the blocks of a window in node order (O(W log W) for W blocks in the window)
    let blocks_of = |w: &[(f32, usize)]| {
        let mut w = w.to_vec();
        w.sort_unstable_by_key(|e| e.1);
        w.into_iter().map(|(_, k)| idx[k]).collect::<Vec<usize>>()
    };
    // A horizontal gap with side-by-side blocks right above it and right below it, separated by a
    // common vertical gutter, runs through two columns (paragraph breaks or figures that happen to
    // line up): never a cut, whatever its size. Only the blocks within `NEAR_FS` font sizes of the
    // gap count, so a full-width block elsewhere in the node (a table at the page foot) does not
    // hide the columns.
    let through = |g: f32, c: f32| {
        let (top, bottom) = (-c + g / 2.0, -c - g / 2.0);
        let near = NEAR_FS * fs;
        let above = blocks_of(window(&by_y0, |y0| y0 < top - 0.5, |y0| y0 <= top + near));
        let below = blocks_of(window(
            &by_y1,
            |y1| y1 < bottom - near,
            |y1| y1 <= bottom + 0.5,
        ));
        spend(above.len() + below.len());
        let (ga, gb) = (x_gaps(blocks, &above), x_gaps(blocks, &below));
        let pairs = |&(a0, a1): &(f32, f32), &(b0, b1): &(f32, f32)| a1.min(b1) - a0.max(b0);
        let ordered = |g: &[(f32, f32)]| g.windows(2).all(|w| w[0].1 <= w[1].0);
        if !ordered(&ga) || !ordered(&gb) {
            // only with inverted boxes (x1 < x0)
            return ga.iter().any(|a| gb.iter().any(|g| pairs(a, g) >= MIN_GAP));
        }
        // both lists run left to right without overlaps: merge them
        let (mut i, mut j) = (0, 0);
        while let (Some(a), Some(g)) = (ga.get(i), gb.get(j)) {
            if pairs(a, g) >= MIN_GAP {
                return true;
            }
            if a.1 < g.1 {
                i += 1;
            } else {
                j += 1;
            }
        }
        false
    };
    for (g, c) in gaps(idx, |i| (-blocks[i].y1, -blocks[i].y0)) {
        spend(1);
        let cut = !in_columns(g, c) && !through(g, c);
        if work.get() == 0 {
            return Err(());
        }
        if cut {
            return Ok(Some(c));
        }
    }
    Ok(None)
}

/// Appends the order of `idx` to `out`; false when the page's work limit `work` runs out.
fn cut(
    blocks: &[OrderBox],
    idx: Vec<usize>,
    depth: usize,
    out: &mut Vec<usize>,
    work: &Cell<usize>,
) -> bool {
    if idx.len() <= 1 || depth >= MAX_DEPTH {
        out.extend(idx);
        return true;
    }
    work.set(work.get().saturating_sub(idx.len()));
    if work.get() == 0 {
        return false;
    }
    let b = |i: usize| blocks[i];
    let x0 = idx.iter().map(|&i| b(i).x0).fold(f32::INFINITY, f32::min);
    let x1 = idx
        .iter()
        .map(|&i| b(i).x1)
        .fold(f32::NEG_INFINITY, f32::max);
    let fs = idx
        .iter()
        .map(|&i| b(i).font_size)
        .fold(f32::INFINITY, f32::min)
        .max(1.0);
    let wide: Vec<usize> = idx
        .iter()
        .copied()
        .filter(|&i| b(i).x1 - b(i).x0 >= NARROW_SHARE * (x1 - x0))
        .collect();
    let v = gaps(if wide.len() >= 2 { &wide } else { &idx }, |i| {
        (b(i).x0, b(i).x1)
    })
    .first()
    .copied();
    // A gutter splits the node into columns first; rows are cut only where no gutter runs through.
    let parts: (Vec<usize>, Vec<usize>) = match v {
        Some((_, cv)) => idx.iter().partition(|&&i| (b(i).x0 + b(i).x1) / 2.0 < cv),
        None => match row_cut(blocks, &idx, fs, work) {
            Err(()) => return false,
            Ok(Some(ch)) => idx.iter().partition(|&&i| -(b(i).y0 + b(i).y1) / 2.0 < ch),
            Ok(None) => (Vec::new(), Vec::new()),
        },
    };
    if parts.0.is_empty() || parts.1.is_empty() {
        out.extend(idx);
        return true;
    }
    cut(blocks, parts.0, depth + 1, out, work) && cut(blocks, parts.1, depth + 1, out, work)
}

/// Running heads and page numbers keep their slots; every other block with a box is sorted.
fn sort_box(b: &Block) -> Option<BBox> {
    match b {
        Block::Note {
            kind: NoteKind::Header | NoteKind::Footer | NoteKind::PageNumber,
            ..
        } => None,
        Block::Heading { bbox, .. }
        | Block::Paragraph { bbox, .. }
        | Block::List { bbox, .. }
        | Block::Table { bbox, .. }
        | Block::Image { bbox, .. }
        | Block::Code { bbox, .. }
        | Block::Note { bbox, .. } => *bbox,
        _ => None,
    }
}

/// Reorders the page `blocks` by [`xy_cut`]: the slots of the sorted blocks (with a box, not a
/// running head/foot or page number) are refilled in XY-cut order, the others keep theirs. `fs` is
/// the font size of every box (blocks carry none). False (order kept) when the page exceeds the
/// limits of [`xy_cut`].
pub(crate) fn order_blocks(blocks: &mut Vec<Block>, fs: f32) -> bool {
    let slots: Vec<(usize, BBox)> = blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| sort_box(b).map(|bb| (i, bb)))
        .collect();
    let boxes: Vec<OrderBox> = slots
        .iter()
        .map(|(_, b)| OrderBox {
            x0: b.x,
            y0: b.y,
            x1: b.x + b.w,
            y1: b.y + b.h,
            font_size: fs,
        })
        .collect();
    let Some(order) = xy_cut(&boxes) else {
        return false;
    };
    // new position -> old index; slots refilled in XY order
    let mut next = order.iter().map(|&k| slots.get(k).map(|s| s.0));
    let mut sorted = slots.iter().map(|s| s.0).peekable();
    let perm: Vec<Option<usize>> = (0..blocks.len())
        .map(|i| {
            if sorted.peek() == Some(&i) {
                sorted.next();
                next.next().flatten()
            } else {
                Some(i)
            }
        })
        .collect();
    let mut seen = vec![false; blocks.len()];
    let valid = perm
        .iter()
        .all(|p| p.is_some_and(|i| i < seen.len() && !std::mem::replace(&mut seen[i], true)));
    if !valid {
        return false;
    }
    let mut old: Vec<Option<Block>> = blocks.drain(..).map(Some).collect();
    blocks.extend(perm.into_iter().flatten().filter_map(|i| old[i].take()));
    true
}

#[cfg(test)]
#[path = "order_test.rs"]
mod tests;
