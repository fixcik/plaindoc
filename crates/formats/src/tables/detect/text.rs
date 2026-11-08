//! Text-aligned candidates (`T`): blocks of segmented lines (`text_join`) and their grid.

use super::bounds::{col_valleys, fs_pt, region_fs, too_wordy};
use super::lined::WordIndex;
use super::model::{EDGE_MAX_VALLEYS, EDGE_MIN_WORDS, edge_bounds, fill_rows};
use super::{
    BLOCK_MAX_GAP_FS, BLOCK_SKIP_GAP_FS, CandidateKind, Raw, SEG_GAP_FS, SEG_MAX_WORDS,
    TEXT_MIN_ROWS, VALLEY_TOL_TEXT,
};
use crate::tables::features::is_num;
use crate::tables::grid::{Line, build_cells, finalize, row_bounds_from_text, text_lines_at};
use crate::tables::prims::{PRect, Region, Word};
use crate::tables::stats::median;
use crate::tables::{LimitExceeded, TableConfig};

struct TLine<'a> {
    line: Line<'a>,
    fs: f64,
    nsegs: usize,
    cand: bool,
    /// Segment starts (ascending) and the end of the last segment (`text_join`).
    seg_x0: Vec<f64>,
    x1: f64,
}

/// `text_join` (T16, proto2.py `tx_join`): candidate lines this many font sizes apart still join when at
/// least [`JOIN_ALIGNED_MIN`] of their segment starts lie within [`JOIN_ALIGN_FS`] font sizes of the
/// previous candidate line's (row padding).
const JOIN_ALIGNED_GAP_FS: f64 = 5.0;
const JOIN_ALIGN_FS: f64 = 0.7;
const JOIN_ALIGNED_MIN: usize = 2;
/// A hanging cell line (a non-candidate line under the block) joins when it lies under this many font
/// sizes below the previous line, is at most this share of the block width, starts within
/// [`HANG_START_FS`] font sizes of a segment start of the block and stays inside the block's x range
/// (± one font size); at most [`MAX_HANG`] non-candidate lines in a row, the one-line skip included
/// (proto2.py `tx_join`).
const HANG_GAP_FS: f64 = 1.8;
const HANG_MAX_W: f64 = 0.5;
const HANG_START_FS: f64 = 1.0;
const MAX_HANG: usize = 6;

/// Order-preserving `u64` key of an `f64` (sign-magnitude to unsigned), for a sorted set of floats.
fn fkey(x: f64) -> u64 {
    let b = x.to_bits();
    if b >> 63 == 1 { !b } else { b | 1 << 63 }
}

fn funkey(k: u64) -> f64 {
    f64::from_bits(if k >> 63 == 1 { k & !(1 << 63) } else { !k })
}

/// Segment starts of a text block: a sorted set filled lazily (starts of joined lines wait in `pending`
/// until a hanging line queries them, so blocks of candidate lines only pay for pushes).
#[derive(Default)]
struct Starts {
    set: std::collections::BTreeSet<u64>,
    pending: Vec<u64>,
}

impl Starts {
    fn add(&mut self, xs: &[f64]) {
        self.pending.extend(xs.iter().map(|&x| fkey(x)));
    }

    /// Some start lies within `tol` of `x` (`any(abs(x - v) <= tol for v in starts)`): subtraction is
    /// monotone, so the neighbours of `x` decide it.
    fn near(&mut self, x: f64, tol: f64) -> bool {
        self.set.extend(self.pending.drain(..));
        let k = fkey(x);
        let below = self.set.range(..=k).next_back();
        let above = self.set.range(k..).next();
        below
            .into_iter()
            .chain(above)
            .any(|&v| (x - funkey(v)).abs() <= tol)
    }
}

/// At least [`JOIN_ALIGNED_MIN`] segment starts of `b` lie within `0.7 fs` of a start of `a` (ascending).
fn starts_aligned(a: &[f64], b: &[f64], fs: f64) -> bool {
    let tol = JOIN_ALIGN_FS * fs;
    let hit = |x: f64| {
        let k = a.partition_point(|&v| v < x);
        (k < a.len() && (x - a[k]).abs() <= tol) || (k > 0 && (x - a[k - 1]).abs() <= tol)
    };
    b.iter().filter(|&&x| hit(x)).take(JOIN_ALIGNED_MIN).count() >= JOIN_ALIGNED_MIN
}

/// Last line of the text block starting at candidate line `i` with `text_join` (proto2.py `tx_join`):
/// hanging cell lines, aligned candidate lines up to [`JOIN_ALIGNED_GAP_FS`] apart, a break on a found
/// region, trailing lines that are not hanging trimmed. Segment starts are kept in a sorted set, so a
/// long block stays O(L log L).
fn join_block(tl: &[TLine], i: usize, used: &[Region]) -> usize {
    let n = tl.len();
    let mut j = i;
    let mut lastc = i;
    let mut starts = Starts::default();
    starts.add(&tl[i].seg_x0);
    let (mut bx0, mut bx1) = (tl[i].seg_x0[0], tl[i].x1);
    let mut hang = 0usize;
    while j + 1 < n {
        let l = &tl[j + 1];
        let gap = l.line.cy - tl[j].line.cy;
        let f = tl[j].fs;
        if used
            .iter()
            .any(|u| u.y0 < l.line.cy && u.y1 > tl[j].line.cy && u.x0 < bx1 && u.x1 > bx0)
        {
            break;
        }
        if l.cand {
            let gapc = l.line.cy - tl[lastc].line.cy;
            if gap < BLOCK_MAX_GAP_FS * f
                || (gapc < JOIN_ALIGNED_GAP_FS * f
                    && starts_aligned(&tl[lastc].seg_x0, &l.seg_x0, f))
            {
                j += 1;
                lastc = j;
                hang = 0;
                starts.add(&l.seg_x0);
                bx0 = bx0.min(l.seg_x0[0]);
                bx1 = bx1.max(l.x1);
                continue;
            }
            break;
        }
        let (lx0, lx1) = (l.seg_x0[0], l.x1);
        if gap < HANG_GAP_FS * f
            && hang < MAX_HANG
            && lx1 - lx0 <= HANG_MAX_W * (bx1 - bx0)
            && starts.near(lx0, HANG_START_FS * f)
            && lx0 >= bx0 - f
            && lx1 <= bx1 + f
        {
            j += 1;
            hang += 1;
            continue;
        }
        if j + 2 < n && tl[j + 2].cand && gap < BLOCK_SKIP_GAP_FS * f && hang == 0 {
            j += 1;
            hang += 1;
            continue;
        }
        break;
    }
    while j > lastc
        && !(tl[j].x1 - tl[j].seg_x0[0] <= HANG_MAX_W * (bx1 - bx0)
            && starts.near(tl[j].seg_x0[0], HANG_START_FS * tl[j].fs))
    {
        j -= 1;
    }
    j
}

pub(crate) fn detect_text<'a>(
    ws: &'a [Word],
    used: &[Region],
    cfg: &TableConfig,
) -> Result<Vec<Raw<'a>>, LimitExceeded> {
    let index = WordIndex::new(ws);
    let mut taken = vec![false; ws.len()];
    for b in used {
        for i in index.inside(ws, b, 1.0) {
            taken[i] = true;
        }
    }
    let free: Vec<&Word> = ws
        .iter()
        .zip(&taken)
        .filter_map(|(w, &t)| (!t).then_some(w))
        .collect();
    let rfs = region_fs(&free, cfg);
    let lines = text_lines_at(&free, rfs);
    if lines.len() < 3 {
        return Ok(Vec::new());
    }
    let tl: Vec<TLine> = lines
        .into_iter()
        .map(|l| {
            let fs = median(&l.w.iter().map(|w| w.fs).collect::<Vec<_>>());
            let sfs = rfs.unwrap_or(fs);
            let mut segs: Vec<(f64, f64, usize)> = Vec::new(); // (x0, x1, words)
            for w in &l.w {
                // `text_longseg` (proto2 `tx_longseg=3`): numeric tokens are not words of the segment.
                let k = usize::from(!(cfg.text_longseg && is_num(&w.t)));
                match segs.last_mut() {
                    Some(s) if w.x0 - s.1 < SEG_GAP_FS * sfs => *s = (s.0, w.x1, s.2 + k),
                    _ => segs.push((w.x0, w.x1, k)),
                }
            }
            let cand = segs.len() >= 2 && segs.iter().all(|s| s.2 <= SEG_MAX_WORDS);
            TLine {
                line: l,
                fs,
                nsegs: segs.len(),
                cand,
                seg_x0: segs.iter().map(|s| s.0).collect(),
                x1: segs.last().map_or(f64::MIN, |s| s.1),
            }
        })
        .collect();
    let n = tl.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if !tl[i].cand {
            i += 1;
            continue;
        }
        let mut j = i;
        if cfg.text_join {
            j = join_block(&tl, i, used);
        } else {
            while j + 1 < n
                && (tl[j + 1].cand
                    || (j + 2 < n
                        && tl[j + 2].cand
                        && tl[j + 1].line.cy - tl[j].line.cy < BLOCK_SKIP_GAP_FS * tl[j].fs))
                && tl[j + 1].line.cy - tl[j].line.cy < BLOCK_MAX_GAP_FS * tl[j].fs
            {
                j += 1;
            }
        }
        let blk = &tl[i..=j];
        i = j + 1;
        if blk.iter().filter(|l| l.nsegs >= 2).count() < 3 {
            continue;
        }
        let bw: Vec<&Word> = blk.iter().flat_map(|l| l.line.w.iter().copied()).collect();
        let fs = median(&blk.iter().map(|l| l.fs).collect::<Vec<_>>());
        let blines: Vec<Line> = blk.iter().map(|l| l.line.clone()).collect();
        out.extend(text_grid(CandidateKind::Text, bw, &blines, fs, None, cfg)?);
    }
    Ok(out)
}

/// Grid of a text block (`detect_text`) or of a snapped model region (SP4-R1) from the words `bw` of its
/// lines `blines` (`fs`: median line font size): columns from text valleys, rows from the text; `None` with
/// fewer than 2 or more than `text_max_cols` columns, fewer than [`TEXT_MIN_ROWS`] rows or, for text blocks
/// only, too wordy cells (a model region keeps multi-line cells). With `fills` (`inner_bounds`, SP4-R3, model
/// regions only) rows come from the filled bands across the block ([`fill_rows`]) and columns from word edge
/// clusters ([`edge_bounds`]) when the valleys fail.
pub(super) fn text_grid<'a>(
    kind: CandidateKind,
    bw: Vec<&'a Word>,
    blines: &[Line],
    fs: f64,
    fills: Option<&[&PRect]>,
    cfg: &TableConfig,
) -> Result<Option<Raw<'a>>, LimitExceeded> {
    let inner = fills.is_some();
    // proto2 `tx_dense`: `col_valleys(bw, blk, fs, 0.1)` with the global dense/adapt switches; with
    // both off it is the plain `valleys` pass of the prototype's text blocks.
    let v = col_valleys(
        &bw,
        blines,
        fs,
        VALLEY_TOL_TEXT,
        cfg.text_dense && cfg.dense_valleys,
        cfg.text_dense && cfg.gmin_adapt,
        cfg.fs_tolerances,
    );
    let m = fs_pt(3.0, cfg.fs_tolerances, fs);
    let bx0 = bw.iter().map(|w| w.x0).fold(f64::MAX, f64::min);
    let bx1 = bw.iter().map(|w| w.x1).fold(f64::MIN, f64::max);
    let interior = |v: Vec<f64>| -> Vec<f64> {
        v.into_iter()
            .filter(|&c| bx0 + m < c && c < bx1 - m)
            .collect()
    };
    let mut v = interior(v);
    if inner
        && v.len() < EDGE_MAX_VALLEYS
        && median(&blines.iter().map(|l| l.w.len() as f64).collect::<Vec<_>>()) >= EDGE_MIN_WORDS
    {
        let e = interior(edge_bounds(blines, fs));
        if e.len() > v.len() {
            v = e;
        }
    }
    let colb: Vec<f64> = std::iter::once(bx0 - 1.0)
        .chain(v)
        .chain(std::iter::once(bx1 + 1.0))
        .collect();
    if colb.len() < 3 || colb.len() - 1 > cfg.text_max_cols {
        return Ok(None);
    }
    let (y0, y1) = (blines[0].cy - 20.0, blines[blines.len() - 1].cy + 20.0);
    let top = bw.iter().map(|w| w.top).fold(f64::MAX, f64::min);
    let bot = bw.iter().map(|w| w.bot).fold(f64::MIN, f64::max);
    let rowb = fills
        .and_then(|f| fill_rows(f, [bx0, top, bx1, bot], y0, y1, blines.len()))
        .unwrap_or_else(|| row_bounds_from_text(blines, &colb, y0, y1, cfg.row_newcols));
    let Some(rows) = finalize(build_cells(&bw, &colb, &rowb)?) else {
        return Ok(None);
    };
    if rows.len() < TEXT_MIN_ROWS {
        return Ok(None);
    }
    if kind == CandidateKind::Text && too_wordy(&rows) {
        return Ok(None);
    }
    Ok(Some(Raw {
        kind,
        bbox: [bx0, top, bx1, bot],
        rows,
        words: bw,
        colb,
        rowb,
        sub: None,
        head: 0,
        text_rows: false,
    }))
}
