//! Fixes over the fork's single-gutter column split (`extract_structured`, ColumnMode::Auto):
//! a gate that rejects a gutter found inside a two-column *table* (the page is then re-extracted
//! with `ColumnMode::Single`), left-before-right order of column regions, and (for the XY-cut
//! reading order) two columns of prose the fork left inside one-column regions, found by runs of
//! rows with a common gap and the same gate.

use pdf_oxide::layout::TextSpan;
use pdf_oxide::{RegionRole, StructuredPage, StructuredRegion};

use super::layout::group_lines;

/// A column side needs at least this many lines to count as prose.
const MIN_SIDE_LINES: usize = 3;
/// Prose fills its column: median line width >= this share of the widest line on that side.
const MIN_FILL: f32 = 0.6;
/// A narrow side (at most this share of the content width of both sides) that does not fill its
/// column is a sidebar of short lines, not a table column, unless the sides are aligned.
/// Measured side widths (share of content width): résumé sidebar .25, form decoy (a false gutter
/// only because its other side fails fill) .34, ODL 102 .37 and ODL 073 .39 (both must keep the
/// gutter, so 0.40 regresses them); 0.35 sits in the thin gap between the form and ODL 102.
const MAX_SIDEBAR: f32 = 0.35;
/// Sides are aligned (table rows, a table of contents) when at least this share of the lines of
/// *both* sides has a line on the other side at the same height (|centre difference| <
/// `ALIGN_TOL` of the line height); independent columns are not.
const MIN_ALIGNED: f32 = 0.6;
/// Two lines are at the same height when their centres differ by less than this share of the
/// line height.
const ALIGN_TOL: f32 = 0.3;
/// A gap wider than this many font sizes splits a side line into chunks (justified prose
/// stretches spaces up to ~1.7 fs in the synthetic corpus, so 1.0 would see chunks in prose).
const CHUNK_GAP_FS: f32 = 2.0;
/// More than this share of side lines with >= 2 chunks means table cells, not prose.
const MAX_MULTI: f32 = 0.3;
/// Upper bound on spans examined per page (service guard): above it the gate keeps the split.
const MAX_GATE_SPANS: usize = 20_000;

pub(crate) fn is_column_region(r: &StructuredRegion) -> bool {
    r.column_index.is_some() && matches!(r.kind, RegionRole::BodyBlock | RegionRole::MarginalLabel)
}

struct SideLine {
    x0: f32,
    x1: f32,
    /// Vertical centre and height of the line's first (anchor) span.
    yc: f32,
    h: f32,
    chunks: usize,
}

/// Lines of one column side: spans sorted top-down by vertical centre and grouped while the centre
/// stays within half of the smaller height (as `layout::group_lines`); chunks are counted over the
/// x-sorted spans of a line. O(n log n).
fn side_lines(spans: &[&TextSpan]) -> Vec<SideLine> {
    let mut v: Vec<&TextSpan> = spans
        .iter()
        .copied()
        .filter(|s| {
            [s.bbox.x, s.bbox.y, s.bbox.width, s.bbox.height]
                .iter()
                .all(|c| c.is_finite())
        })
        .collect();
    let yc = |s: &TextSpan| s.bbox.y + s.bbox.height / 2.0;
    v.sort_by(|a, b| yc(b).total_cmp(&yc(a)).then(a.bbox.x.total_cmp(&b.bbox.x)));
    let mut rows: Vec<(f32, f32, Vec<&TextSpan>)> = Vec::new(); // (anchor centre, height, spans)
    for s in v {
        match rows.last_mut() {
            Some((c, h, row)) if (yc(s) - *c).abs() < 0.5 * h.min(s.bbox.height) => row.push(s),
            _ => rows.push((yc(s), s.bbox.height, vec![s])),
        }
    }
    rows.into_iter()
        .map(|(yc, h, mut v)| {
            v.sort_by(|a, b| a.bbox.x.total_cmp(&b.bbox.x));
            let mut chunks = 1;
            let mut right = v[0].bbox.x + v[0].bbox.width;
            for s in &v[1..] {
                if s.bbox.x - right > CHUNK_GAP_FS * s.font_size.max(1.0) {
                    chunks += 1;
                }
                right = right.max(s.bbox.x + s.bbox.width);
            }
            SideLine {
                x0: v[0].bbox.x,
                x1: right,
                yc,
                h,
                chunks,
            }
        })
        .collect()
}

/// Share of `lines` with a line of `other` at the same height. `other` is sorted by descending
/// centre (as `side_lines` returns it), so each lookup is a binary search: O(n log m).
fn aligned_share(lines: &[SideLine], other: &[SideLine]) -> f32 {
    if lines.is_empty() {
        return 0.0;
    }
    let hits = lines
        .iter()
        .filter(|l| {
            let tol = ALIGN_TOL * l.h.abs();
            // first line of `other` whose centre is below `l.yc + tol`
            let i = other.partition_point(|o| o.yc >= l.yc + tol);
            other.get(i).is_some_and(|o| (o.yc - l.yc).abs() < tol)
        })
        .count();
    hits as f32 / lines.len() as f32
}

/// More than `MAX_MULTI` of the lines split into chunks: table cells, not prose. An empty side
/// counts as cells.
fn side_is_cells(lines: &[SideLine]) -> bool {
    let multi = lines.iter().filter(|l| l.chunks >= 2).count() as f32 / lines.len().max(1) as f32;
    lines.is_empty() || multi > MAX_MULTI
}

/// Horizontal extent of a side.
fn extent(lines: &[SideLine]) -> (f32, f32) {
    lines
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), l| {
            (a.min(l.x0), b.max(l.x1))
        })
}

/// Median line width at least `MIN_FILL` of the widest line on that side.
fn side_fills(lines: &[SideLine]) -> bool {
    let mut w: Vec<f32> = lines.iter().map(|l| l.x1 - l.x0).collect();
    w.sort_by(f32::total_cmp);
    match (w.get(w.len() / 2), w.last()) {
        (Some(&median), Some(&max)) => max > 0.0 && median >= MIN_FILL * max,
        _ => false,
    }
}

/// Lines of both sides that lie in the vertical band where the sides overlap (lines are
/// `yc ± h/2` intervals; a line touching the band is kept). Column 0 also holds what the fork put
/// above or below the columns (a title, headings, a table), which says nothing about the gutter.
/// Sides stacked one above the other share no band: both come back empty (a false gutter), and
/// reading the page as one column keeps that order.
fn beside(l: Vec<SideLine>, r: Vec<SideLine>) -> [Vec<SideLine>; 2] {
    let span = |v: &[SideLine]| {
        v.iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), x| {
                (a.min(x.yc - x.h / 2.0), b.max(x.yc + x.h / 2.0))
            })
    };
    let ((lb, lt), (rb, rt)) = (span(&l), span(&r));
    let (bottom, top) = (lb.max(rb), lt.min(rt));
    let keep = |v: Vec<SideLine>| -> Vec<SideLine> {
        v.into_iter()
            .filter(|x| x.yc + x.h / 2.0 >= bottom && x.yc - x.h / 2.0 <= top)
            .collect()
    };
    [keep(l), keep(r)]
}

/// True when the fork split the page into columns but the two sides do not read as prose
/// (a two-column table, a form, a key-value list): the caller re-extracts with `ColumnMode::Single`.
/// Line count judges each whole side; chunking, fill, alignment and width only the band where
/// both sides have lines (`beside`).
/// A narrow side of short lines (a résumé sidebar) keeps the gutter unless its lines are aligned
/// row by row with the other side's.
pub(crate) fn false_gutter(page: &StructuredPage) -> bool {
    let mut sides: [Vec<&TextSpan>; 2] = [Vec::new(), Vec::new()];
    for r in page.regions.iter().filter(|r| is_column_region(r)) {
        let side = usize::from(r.column_index != Some(0));
        sides[side].extend(r.spans.iter().filter(|s| !s.text.trim().is_empty()));
    }
    if sides[0].is_empty() && sides[1].is_empty() {
        return false;
    }
    if sides[0].len() + sides[1].len() > MAX_GATE_SPANS {
        return false;
    }
    sides_are_not_prose(side_lines(&sides[0]), side_lines(&sides[1]))
}

/// The gate of [`false_gutter`] over the lines of the two sides of a gutter.
fn sides_are_not_prose(l: Vec<SideLine>, r: Vec<SideLine>) -> bool {
    if l.len() < MIN_SIDE_LINES || r.len() < MIN_SIDE_LINES {
        return true;
    }
    let [l, r] = beside(l, r);
    if side_is_cells(&l) || side_is_cells(&r) {
        return true;
    }
    let (fl, fr) = (side_fills(&l), side_fills(&r));
    if fl && fr {
        return false;
    }
    // aligned rows (a table, a table of contents): a side that does not fill is a table column
    if aligned_share(&l, &r) >= MIN_ALIGNED && aligned_share(&r, &l) >= MIN_ALIGNED {
        return true;
    }
    // misaligned: a narrow side of short lines is a sidebar, it needs no fill
    let (el, er) = (extent(&l), extent(&r));
    let content = el.1.max(er.1) - el.0.min(er.0);
    let sidebar = |e: (f32, f32)| content > 0.0 && e.1 - e.0 <= MAX_SIDEBAR * content;
    !((fl || sidebar(el)) && (fr || sidebar(er)))
}

/// Region order with column regions left before right: the slots held by column regions are
/// refilled by the same regions stably sorted by `column_index`; other regions keep their slots.
pub(crate) fn column_order(regions: &[StructuredRegion]) -> Vec<&StructuredRegion> {
    let mut cols: Vec<&StructuredRegion> = regions.iter().filter(|r| is_column_region(r)).collect();
    cols.sort_by_key(|r| r.column_index);
    let mut cols = cols.into_iter();
    regions
        .iter()
        .map(|r| {
            if is_column_region(r) {
                cols.next().unwrap_or(r)
            } else {
                r
            }
        })
        .collect()
}

/// A gap of at least this many font sizes between neighbouring spans of a row may be a gutter
/// (ODL-200 gutters are 1.0–1.3 font sizes; the fork needs a corridor empty over the whole page,
/// so a full-width caption or heading hides the gutter from it).
const GUTTER_FS: f32 = 1.0;
/// A gutter is a run of at least this many rows with a common gap; rows that lie entirely on one
/// side of it do not break the run.
const MIN_RUN: usize = 4;
/// The gutter lies in this middle share of the body text's width, as the fork requires of its own
/// gutter (a two-column table of names and dates sits left of the middle).
const GUTTER_MIDDLE: (f32, f32) = (0.3, 0.7);
/// Upper bound on rows visited and spans gated while looking for runs (service guard): the search
/// stops there.
const MAX_RUN_VISITS: usize = 1_000_000;

/// A row of spans on one baseline (grouped as in `side_lines`), with its gaps of at least
/// [`GUTTER_FS`] font sizes as `(left edge, right edge)`.
struct Row {
    top: f32,
    bottom: f32,
    x0: f32,
    x1: f32,
    fs: f32,
    gaps: Vec<(f32, f32)>,
    spans: Vec<usize>,
}

/// Two columns inside one-column regions: the rows from `top` down to `bottom` (PDF y up) are
/// read left of `x`, then right of it.
struct Band {
    top: f32,
    bottom: f32,
    x: f32,
}

fn rows_of(spans: &[&TextSpan]) -> Vec<Row> {
    let yc = |s: &TextSpan| s.bbox.y + s.bbox.height / 2.0;
    let mut idx: Vec<usize> = (0..spans.len()).collect();
    idx.sort_by(|&a, &b| {
        yc(spans[b])
            .total_cmp(&yc(spans[a]))
            .then(spans[a].bbox.x.total_cmp(&spans[b].bbox.x))
    });
    let mut groups: Vec<(f32, f32, Vec<usize>)> = Vec::new(); // anchor centre, height, spans
    for i in idx {
        let s = spans[i];
        match groups.last_mut() {
            Some((c, h, g)) if (yc(s) - *c).abs() < 0.5 * h.min(s.bbox.height) => g.push(i),
            _ => groups.push((yc(s), s.bbox.height, vec![i])),
        }
    }
    groups
        .into_iter()
        .map(|(_, _, mut g)| {
            g.sort_by(|&a, &b| spans[a].bbox.x.total_cmp(&spans[b].bbox.x));
            let fs = g.iter().map(|&i| spans[i].font_size).fold(1.0f32, f32::max);
            let mut gaps = Vec::new();
            let mut right = f32::NEG_INFINITY;
            for &i in &g {
                let b = &spans[i].bbox;
                if right.is_finite() && b.x - right >= GUTTER_FS * fs {
                    gaps.push((right, b.x));
                }
                right = right.max(b.x + b.width);
            }
            let fold = |f: fn(f32, f32) -> f32, init: f32, v: &dyn Fn(&TextSpan) -> f32| {
                g.iter().map(|&i| v(spans[i])).fold(init, f)
            };
            Row {
                top: fold(f32::max, f32::NEG_INFINITY, &|s| s.bbox.y + s.bbox.height),
                bottom: fold(f32::min, f32::INFINITY, &|s| s.bbox.y),
                x0: fold(f32::min, f32::INFINITY, &|s| s.bbox.x),
                x1: right,
                fs,
                gaps,
                spans: g,
            }
        })
        .collect()
}

/// Gutters by runs of rows (as ODL's two-column blocks): from a row with a gap, the following rows
/// whose gaps keep a common interval at least [`GUTTER_FS`] font sizes wide extend the run; a row
/// entirely on one side of the interval is part of the run without narrowing it; any other row
/// ends it. The longest run of at least [`MIN_RUN`] rows with its gutter in the middle of the text
/// ([`GUTTER_MIDDLE`]) whose sides read as prose (the [`false_gutter`] gate) becomes a band, and the
/// search goes on below it.
fn bands_of(spans: &[&TextSpan], rows: &[Row]) -> Vec<Band> {
    let (left, right) = rows
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), r| {
            (a.min(r.x0), b.max(r.x1))
        });
    let middle = |x: f32| {
        let rel = (x - left) / (right - left);
        (GUTTER_MIDDLE.0..=GUTTER_MIDDLE.1).contains(&rel)
    };
    let mut bands = Vec::new();
    let mut visits = 0usize;
    let mut i = 0;
    while i < rows.len() && visits < MAX_RUN_VISITS {
        let mut best: Option<(usize, usize, f32)> = None; // members, end (exclusive), x
        for &(g0, g1) in &rows[i].gaps {
            let (mut lo, mut hi) = (g0, g1);
            let (mut members, mut j) = (1, i + 1);
            while j < rows.len() && visits < MAX_RUN_VISITS {
                visits += 1;
                let l = &rows[j];
                if l.x1 <= lo + 1.0 || l.x0 >= hi - 1.0 {
                    j += 1;
                    continue;
                }
                let min_w = GUTTER_FS * l.fs.min(rows[i].fs);
                let Some(cut) = l.gaps.iter().find_map(|&(a, b)| {
                    let (a, b) = (lo.max(a), hi.min(b));
                    (b - a >= min_w).then_some((a, b))
                }) else {
                    break;
                };
                (lo, hi) = cut;
                members += 1;
                j += 1;
            }
            let x = (lo + hi) / 2.0;
            if members >= MIN_RUN
                && best.is_none_or(|b| members > b.0)
                && middle(x)
                && {
                    // the gate's span work counts against the same budget
                    visits += rows[i..j].iter().map(|r| r.spans.len()).sum::<usize>();
                    visits < MAX_RUN_VISITS
                }
                && !run_is_not_prose(spans, &rows[i..j], x)
            {
                best = Some((members, j, x));
            }
        }
        match best {
            Some((_, j, x)) => {
                bands.push(Band {
                    top: rows[i].top,
                    bottom: rows[j - 1].bottom,
                    x,
                });
                i = j;
            }
            None => i += 1,
        }
    }
    bands
}

/// The [`false_gutter`] gate on the two sides of a run split at `x`.
fn run_is_not_prose(spans: &[&TextSpan], rows: &[Row], x: f32) -> bool {
    let mut sides: [Vec<&TextSpan>; 2] = [Vec::new(), Vec::new()];
    for &i in rows.iter().flat_map(|r| &r.spans) {
        let s = spans[i];
        sides[usize::from(s.bbox.x + s.bbox.width / 2.0 >= x)].push(s);
    }
    sides_are_not_prose(side_lines(&sides[0]), side_lines(&sides[1]))
}

/// The page with its one-column body regions split where their rows hold two columns of prose
/// that the fork did not separate (no corridor over the whole page, or the page re-read as one
/// column): each such region becomes, in this order, its part above a band, the left side of
/// the band, its right side, and so on downwards. A piece's text is rebuilt from its spans, one
/// line per row. `None` when nothing is split.
pub(crate) fn split_gutters(page: &StructuredPage) -> Option<StructuredPage> {
    let usable = |s: &TextSpan| {
        !s.text.trim().is_empty()
            && [s.bbox.x, s.bbox.y, s.bbox.width, s.bbox.height, s.font_size]
                .iter()
                .all(|c| c.is_finite())
    };
    let one_column = |r: &StructuredRegion| {
        r.column_index.is_none()
            && matches!(r.kind, RegionRole::BodyBlock | RegionRole::MarginalLabel)
    };
    let spans: Vec<&TextSpan> = page
        .regions
        .iter()
        .filter(|r| one_column(r))
        .flat_map(|r| r.spans.iter().filter(|s| usable(s)))
        .collect();
    if spans.len() > MAX_GATE_SPANS {
        return None;
    }
    let bands = bands_of(&spans, &rows_of(&spans));
    if bands.is_empty() {
        return None;
    }
    // (bands above, 0 = outside a band / 1 = left / 2 = right)
    let key = |s: &TextSpan| {
        let (x, y) = (
            s.bbox.x + s.bbox.width / 2.0,
            s.bbox.y + s.bbox.height / 2.0,
        );
        let k = bands.iter().take_while(|b| b.bottom > y).count();
        match bands.get(k) {
            Some(b) if y <= b.top => (k, 1 + u8::from(x >= b.x)),
            _ => (k, 0),
        }
    };
    let mut out = StructuredPage {
        page_index: page.page_index,
        page_width: page.page_width,
        page_height: page.page_height,
        regions: Vec::with_capacity(page.regions.len()),
    };
    let mut split = false;
    for r in &page.regions {
        if !one_column(r) {
            out.regions.push(r.clone());
            continue;
        }
        // unusable spans stay with the span before them
        let mut last = None;
        let keys: Vec<Option<(usize, u8)>> = r
            .spans
            .iter()
            .map(|s| {
                if usable(s) {
                    last = Some(key(s));
                }
                last
            })
            .collect();
        let first = keys.iter().flatten().next().copied();
        let keys: Vec<(usize, u8)> = keys.iter().map(|k| k.or(first).unwrap_or((0, 0))).collect();
        let mut distinct = keys.clone();
        distinct.sort_unstable();
        distinct.dedup();
        if distinct.len() < 2 {
            out.regions.push(r.clone());
            continue;
        }
        split = true;
        for k in distinct {
            let spans: Vec<TextSpan> = r
                .spans
                .iter()
                .zip(&keys)
                .filter(|(_, kk)| **kk == k)
                .map(|(s, _)| s.clone())
                .collect();
            let mut piece = r.clone();
            piece.bbox = spans
                .iter()
                .skip(1)
                .fold(spans[0].bbox, |b, s| b.union(&s.bbox));
            piece.text = group_lines(&spans)
                .into_iter()
                .map(|l| l.text)
                .collect::<Vec<_>>()
                .join("\n");
            piece.spans = spans;
            out.regions.push(piece);
        }
    }
    split.then_some(out)
}

#[cfg(test)]
#[path = "columns_test.rs"]
mod tests;
