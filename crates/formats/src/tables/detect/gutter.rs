//! Two-column prose guards (T15 `gutter_prose`, `page_columns`): layout gutters of the page.

use crate::tables::grid::{Line, text_lines, valleys};
use crate::tables::prims::Word;
use crate::tables::stats::median;

/// Layout gutters (T15, proto2.py `page_candidates`, `gutter_prose`): page valleys at least this wide
/// (and one median font size), crossed by at most this share of the page's lines, strictly inside this
/// x range of the page.
const GUTTER_MIN_GAP: f64 = 6.0;
const GUTTER_TOL_LINES: f64 = 0.03;
const GUTTER_PAGE_LO: f64 = 0.25;
const GUTTER_PAGE_HI: f64 = 0.75;
/// A candidate crosses a gutter strictly inside the middle half of its own width (`x0 + 0.25 W`).
const GUTTER_CAND_MARGIN: f64 = 0.25;
/// Mode 1: the median line has at least this many words on each side of the gutter.
const PROSE_SIDE_WORDS: f64 = 4.0;
/// `page_columns`: a column bound of a candidate within this distance (pt) of a page gutter is on it.
const GUTTER_COL_TOL: f64 = 3.0;
/// `page_columns`: lines each page column must have above the candidate.
const COLS_ABOVE_LINES: usize = 3;
/// `page_columns`: how far above the candidate (in font sizes) the page columns are looked for.
const COLS_ABOVE_BAND: f64 = 10.0;
/// `wide_gap_columns`: an inner column gap of a `T` candidate at least this many font sizes wide.
const WIDE_GAP_FS: f64 = 12.0;
/// Mode 3: fewer than this share of the lines have words on both sides (shifted column baselines).
const SHIFTED_BOTH_SHARE: f64 = 0.2;

/// Layout gutters over all words of the page (`fs`: their median font size): one [`valleys`] and one
/// [`text_lines`] pass, O(n log n).
pub(crate) fn page_gutters(ws: &[Word], page_w: f64, fs: f64) -> Vec<f64> {
    let all: Vec<&Word> = ws.iter().collect();
    let tol = (GUTTER_TOL_LINES * text_lines(&all).len() as f64) as usize;
    valleys(&all, GUTTER_MIN_GAP.max(fs), tol)
        .into_iter()
        .filter(|&g| GUTTER_PAGE_LO * page_w < g && g < GUTTER_PAGE_HI * page_w)
        .collect()
}

/// proto2.py guard (а) on a `T` candidate without numeric columns: for a gutter in the middle half of
/// `bbox`, the median line of `words` has at least [`PROSE_SIDE_WORDS`] words with `cx < g` and as many
/// with `cx >= g`; with `mode == 3` also when fewer than [`SHIFTED_BOTH_SHARE`] of the lines have words on
/// both sides. The lines are built once; each gutter costs one pass over the words, the first hit ends.
pub(super) fn prose_across_gutter(words: &[&Word], bbox: [f64; 4], gut: &[f64], mode: u8) -> bool {
    if mode == 0 || words.is_empty() {
        return false;
    }
    let (x0, x1) = (bbox[0], bbox[2]);
    let m = GUTTER_CAND_MARGIN * (x1 - x0);
    let mut lines: Option<Vec<Line>> = None;
    for &g in gut.iter().filter(|&&g| x0 + m < g && g < x1 - m) {
        let ls = lines.get_or_insert_with(|| text_lines(words));
        let (mut lc, mut rc) = (Vec::with_capacity(ls.len()), Vec::with_capacity(ls.len()));
        let mut both = 0usize;
        for l in ls.iter() {
            let left = l.w.iter().filter(|w| w.cx < g).count();
            let right = l.w.iter().filter(|w| w.cx >= g).count();
            lc.push(left as f64);
            rc.push(right as f64);
            both += usize::from(left > 0 && right > 0);
        }
        if median(&lc) >= PROSE_SIDE_WORDS && median(&rc) >= PROSE_SIDE_WORDS {
            return true;
        }
        if mode == 3 && (both as f64) < SHIFTED_BOTH_SHARE * ls.len() as f64 {
            return true;
        }
    }
    false
}

/// `page_columns`: the gutter of the page ([`page_gutters`]) coincides (within [`GUTTER_COL_TOL`]) with an
/// inner column bound of the `T` candidate, and the text directly above it (within [`COLS_ABOVE_BAND`]
/// font sizes, inside its x-span) has at least [`COLS_ABOVE_LINES`] lines with words on each side of the
/// gutter, not counting lines that cross it (one-column text, a title), and those lines of at least one
/// side have a median of [`PROSE_SIDE_WORDS`] words (prose, not the keys and values of a form section
/// above): the candidate is the bottom of two page columns. A line is crossing when a word spans the
/// gutter or the words around it are closer than [`GUTTER_MIN_GAP`].
pub(super) fn columns_continue_above(
    ws: &[Word],
    bbox: [f64; 4],
    fs: f64,
    colb: &[f64],
    gut: &[f64],
) -> bool {
    let inner = colb.get(1..colb.len().saturating_sub(1)).unwrap_or(&[]);
    let gs: Vec<f64> = gut
        .iter()
        .copied()
        .filter(|&g| inner.iter().any(|&c| (c - g).abs() <= GUTTER_COL_TOL))
        .collect();
    columns_above_at(ws, bbox, fs, &gs)
}

/// `wide_gap_columns`: the page gutters that lie inside an empty gap of the `T` candidate at least
/// [`WIDE_GAP_FS`] font sizes wide, between the words left and right of an inner column bound: the
/// candidate's rows join two page columns (a résumé's main column and sidebar) across a wide gap.
pub(super) fn wide_gap_gutters(words: &[&Word], colb: &[f64], gut: &[f64], fs: f64) -> Vec<f64> {
    let inner = colb.get(1..colb.len().saturating_sub(1)).unwrap_or(&[]);
    let gaps: Vec<(f64, f64)> = inner
        .iter()
        .map(|&c| {
            let l = words.iter().filter(|w| w.cx < c).map(|w| w.x1);
            let r = words.iter().filter(|w| w.cx >= c).map(|w| w.x0);
            (l.fold(f64::MIN, f64::max), r.fold(f64::MAX, f64::min))
        })
        // A bound with no word on one side is no gap between two columns.
        .filter(|&(l, r)| l > f64::MIN && r < f64::MAX && r - l >= WIDE_GAP_FS * fs)
        .collect();
    gut.iter()
        .copied()
        .filter(|&g| gaps.iter().any(|&(l, r)| l < g && g < r))
        .collect()
}

/// The `page_columns` test above the candidate at the gutters `gs` (see [`columns_continue_above`]).
pub(super) fn columns_above_at(ws: &[Word], bbox: [f64; 4], fs: f64, gs: &[f64]) -> bool {
    if gs.is_empty() {
        return false;
    }
    let above: Vec<&Word> = ws
        .iter()
        .filter(|w| {
            bbox[1] - COLS_ABOVE_BAND * fs <= w.cy
                && w.cy < bbox[1]
                && bbox[0] <= w.cx
                && w.cx <= bbox[2]
        })
        .collect();
    let lines = text_lines(&above);
    gs.iter().any(|&g| {
        // Words per counted line on each side: at least one page column above must be prose.
        let (mut lw, mut rw) = (Vec::new(), Vec::new());
        for line in &lines {
            let crossing = line.w.iter().any(|w| w.x0 < g && g < w.x1)
                || line
                    .w
                    .windows(2)
                    .any(|p| p[0].cx < g && g <= p[1].cx && p[1].x0 - p[0].x1 < GUTTER_MIN_GAP);
            if !crossing {
                let left = line.w.iter().filter(|w| w.cx < g).count();
                let right = line.w.len() - left;
                if left > 0 {
                    lw.push(left as f64);
                }
                if right > 0 {
                    rw.push(right as f64);
                }
            }
        }
        lw.len().min(rw.len()) >= COLS_ABOVE_LINES
            && median(&lw).max(median(&rw)) >= PROSE_SIDE_WORDS
    })
}
