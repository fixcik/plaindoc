//! Nested tables: an accepted table lying inside another one moves into the outer cell that holds its
//! words (the outer grid saw them as flat cell text), so its text appears once.

use super::select::{DetectedGrid, GridCell};

/// Slack for "inside" (pt).
const INSIDE_SLACK: f64 = 2.0;

fn area(b: [f64; 4]) -> f64 {
    (b[2] - b[0]) * (b[3] - b[1])
}

fn inside(inner: [f64; 4], outer: [f64; 4]) -> bool {
    inner != outer
        && outer[0] - INSIDE_SLACK <= inner[0]
        && outer[1] - INSIDE_SLACK <= inner[1]
        && inner[2] <= outer[2] + INSIDE_SLACK
        && inner[3] <= outer[3] + INSIDE_SLACK
        && area(inner) < area(outer)
}

fn tokens(t: &DetectedGrid) -> Vec<String> {
    t.rows
        .iter()
        .flatten()
        .flat_map(|c| {
            c.flat_text()
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// How many of `toks` occur in `text`, each word matching at most one token (first occurrence).
fn count_tokens(text: &str, toks: &[String]) -> usize {
    let mut left: Vec<&str> = text.split_whitespace().collect();
    let mut found = 0;
    for t in toks {
        if let Some(i) = left.iter().position(|w| w == t) {
            left.remove(i);
            found += 1;
        }
    }
    found
}

/// Puts `inner` into the cell of `outer` sharing most of its words and removes its words from the outer
/// cells (that cell first, then the nearest ones: SP4 review M4); gives `inner` back when no cell shares
/// a word.
fn attach(outer: &mut DetectedGrid, inner: DetectedGrid) -> Option<DetectedGrid> {
    let toks = tokens(&inner);
    let toks = &toks;
    let best = outer
        .rows
        .iter()
        .enumerate()
        .flat_map(|(r, row)| {
            row.iter()
                .enumerate()
                .map(move |(c, cell)| (r, c, count_tokens(&cell.text, toks)))
        })
        .max_by_key(|&(r, c, n)| (n, std::cmp::Reverse((r, c))));
    let Some((br, bc, n)) = best else {
        return Some(inner);
    };
    if n == 0 {
        return Some(inner);
    }
    let mut left: Vec<String> = toks.clone();
    let mut strip = |cell: &mut GridCell| {
        let mut words: Vec<&str> = cell.text.split_whitespace().collect();
        left.retain(|t| match words.iter().position(|w| w == t) {
            Some(i) => {
                words.remove(i);
                false
            }
            None => true,
        });
        cell.text = words.join(" ");
    };
    // That cell first, then the others nearest first (an unruled outer grid spreads the inner words over
    // neighbouring cells; a far cell with the same token, such as `1` of a number column, comes last).
    let mut others: Vec<(usize, usize)> = (0..outer.rows.len())
        .flat_map(|r| (0..outer.rows[r].len()).map(move |c| (r, c)))
        .filter(|&rc| rc != (br, bc))
        .collect();
    others.sort_by_key(|&(r, c)| (r.abs_diff(br) + c.abs_diff(bc), r, c));
    strip(&mut outer.rows[br][bc]);
    for (r, c) in others {
        strip(&mut outer.rows[r][c]);
    }
    outer.rows[br][bc].nested.push(inner);
    None
}

/// Nests every table into the smallest accepted table containing it; order of the rest is kept.
pub(crate) fn nest(ts: Vec<DetectedGrid>) -> Vec<DetectedGrid> {
    let n = ts.len();
    let parent: Vec<Option<usize>> = (0..n)
        .map(|i| {
            (0..n)
                .filter(|&j| inside(ts[i].bbox, ts[j].bbox))
                .min_by(|&a, &b| area(ts[a].bbox).total_cmp(&area(ts[b].bbox)))
        })
        .collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| area(ts[a].bbox).total_cmp(&area(ts[b].bbox)));
    let mut slots: Vec<Option<DetectedGrid>> = ts.into_iter().map(Some).collect();
    for i in order {
        if let Some(j) = parent[i] {
            let child = slots[i].take().expect("child");
            if let Some(back) = attach(slots[j].as_mut().expect("parent"), child) {
                slots[i] = Some(back);
            }
        }
    }
    slots.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::{CandidateKind, Features, Structure};

    fn grid(bbox: [f64; 4], rows: &[&[&str]]) -> DetectedGrid {
        DetectedGrid {
            kind: CandidateKind::Lined,
            bbox,
            cells: bbox,
            rows: rows
                .iter()
                .map(|r| r.iter().map(|t| GridCell::new(*t, 1, 1)).collect())
                .collect(),
            header_rows: 1,
            structure: Structure::Planar,
            probability: 1.0,
            features: Features::default(),
        }
    }

    #[test]
    fn inner_table_moves_into_the_cell_with_its_words() {
        let outer = grid(
            [50.0, 200.0, 400.0, 380.0],
            &[&["№", "Set"], &["1", "Kit 1 Item Qty Bolt 11"]],
        );
        let inner = grid(
            [195.0, 246.0, 323.0, 301.0],
            &[&["Item", "Qty"], &["Bolt", "11"]],
        );
        let out = nest(vec![inner, outer]);
        assert_eq!(out.len(), 1);
        let cell = &out[0].rows[1][1];
        assert_eq!((cell.text.as_str(), cell.nested.len()), ("Kit 1", 1));
        assert_eq!(cell.flat_text(), "Kit 1 Item Qty Bolt 11");
    }

    #[test]
    fn nested_tokens_are_taken_from_the_nearest_cells() {
        // The unruled outer grid spread the inner "1" into the cell next to the best one; the "1" of the
        // number column (farther, but first in row order) stays.
        let outer = grid(
            [50.0, 200.0, 400.0, 380.0],
            &[
                &["1", "Set", ""],
                &["", "", ""],
                &["", "Kit Item Qty Bolt", ""],
                &["", "", "1"],
            ],
        );
        let inner = grid(
            [195.0, 246.0, 323.0, 301.0],
            &[&["Item", "Qty"], &["Bolt", "1"]],
        );
        let out = nest(vec![inner, outer]);
        let t = |r: usize, c: usize| out[0].rows[r][c].text.as_str();
        assert_eq!((t(0, 0), t(2, 1), t(3, 2)), ("1", "Kit", ""));
    }

    #[test]
    fn disjoint_or_unrelated_tables_stay() {
        let a = grid([0.0, 0.0, 100.0, 100.0], &[&["a", "b"]]);
        let b = grid([200.0, 0.0, 300.0, 100.0], &[&["c", "d"]]);
        assert_eq!(nest(vec![a.clone(), b]).len(), 2);
        let inside_no_words = grid([10.0, 10.0, 50.0, 50.0], &[&["x", "y"]]);
        assert_eq!(
            nest(vec![a, inside_no_words]).len(),
            2,
            "no shared words: not nested"
        );
    }
}
