//! `box_flow` (SP8b S3): tag chips (skills of a résumé) wrapped in rows of separate boxes are not a table.

use crate::tables::prims::{PRect, Word};

/// A box of the candidate lies within this many pt of its bbox; a box inside another one within this slack
/// is the same chip (a fill under its stroke).
const BOX_TOL: f64 = 1.5;
/// Boxes of one row closer than this (pt) touch: cells of a grid, not chips.
const BOX_MIN_GAP: f64 = 1.0;
/// A box edge matches an edge of another row within this many pt.
const BOX_ALIGN: f64 = 1.0;
/// At least this many boxes; more than [`BOX_MAX`] are not examined (the pair test is quadratic).
const BOX_MIN: usize = 4;
const BOX_MAX: usize = 200;

/// The candidate is a flow of separate boxes: every word lies in exactly one box of `bbox` (`rects`
/// without background fills, a box inside another counted once), the boxes of one row do not touch, there
/// are at least two rows of two boxes, and an inner edge of a row (the left edge of a box after the first
/// or the right edge of a box before the last) has no edge of another row within [`BOX_ALIGN`]: chips
/// wrapped by their text width. The boxes of a table with spaced cells keep their columns aligned.
pub(super) fn box_flow(words: &[&Word], rects: &[PRect], bbox: [f64; 4]) -> bool {
    let inside = |r: &PRect, b: [f64; 4]| {
        r.x0 >= b[0] - BOX_TOL
            && r.t >= b[1] - BOX_TOL
            && r.x1 <= b[2] + BOX_TOL
            && r.b <= b[3] + BOX_TOL
    };
    let cand: Vec<&PRect> = rects
        .iter()
        .filter(|r| !r.background && inside(r, bbox))
        .collect();
    if cand.len() > 2 * BOX_MAX {
        return false;
    }
    let mut boxes: Vec<[f64; 4]> = Vec::new();
    for (i, r) in cand.iter().enumerate() {
        // Keep the outermost of nested boxes (the first of equal ones).
        let outer = cand.iter().enumerate().any(|(j, o)| {
            j != i
                && inside(r, [o.x0, o.t, o.x1, o.b])
                && (!inside(o, [r.x0, r.t, r.x1, r.b]) || j < i)
        });
        if !outer {
            boxes.push([r.x0, r.t, r.x1, r.b]);
        }
    }
    if boxes.len() < BOX_MIN || boxes.len() > BOX_MAX {
        return false;
    }
    let holds =
        |b: &[f64; 4], w: &Word| b[0] <= w.cx && w.cx <= b[2] && b[1] <= w.cy && w.cy <= b[3];
    if !words
        .iter()
        .all(|w| boxes.iter().filter(|b| holds(b, w)).count() == 1)
    {
        return false;
    }
    // Rows: boxes whose vertical centre lies within the first box's height, left to right.
    boxes.sort_by(|a, b| (a[1] + a[3]).total_cmp(&(b[1] + b[3])));
    let mut rows: Vec<Vec<[f64; 4]>> = Vec::new();
    for b in boxes {
        let cy = (b[1] + b[3]) / 2.0;
        match rows.last_mut() {
            Some(r) if r[0][1] <= cy && cy <= r[0][3] => r.push(b),
            _ => rows.push(vec![b]),
        }
    }
    for r in &mut rows {
        r.sort_by(|a, b| a[0].total_cmp(&b[0]));
        if r.windows(2).any(|p| p[1][0] - p[0][2] < BOX_MIN_GAP) {
            return false;
        }
    }
    let rows: Vec<&Vec<[f64; 4]>> = rows.iter().filter(|r| r.len() >= 2).collect();
    if rows.len() < 2 {
        return false;
    }
    let edges = |r: &[[f64; 4]]| -> Vec<f64> {
        let n = r.len();
        r.iter()
            .enumerate()
            .flat_map(|(k, b)| [(k > 0).then_some(b[0]), (k + 1 < n).then_some(b[2])])
            .flatten()
            .collect()
    };
    rows.iter().enumerate().any(|(i, r)| {
        edges(r).iter().any(|&e| {
            !rows.iter().enumerate().any(|(j, o)| {
                j != i
                    && o.iter()
                        .any(|b| (b[0] - e).abs() <= BOX_ALIGN || (b[2] - e).abs() <= BOX_ALIGN)
            })
        })
    })
}
