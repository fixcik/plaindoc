//! Cheap per-page gate before the region model (T18, `S/idea-tables2/scripts/gate_feats.py`, REPORT §7):
//! "may this page contain a table?". Rule G* plus the no-words rule: a candidate of the detector, or at
//! least 3 horizontal and 2 vertical rulings, or two neighbouring text lines aligned at an inner segment
//! start (after cutting the page at its layout gutters), or no words at all; and the image rule (an image
//! covering [`LARGE_IMAGE_SHARE`] of the page, when the source filled [`PageInput::image_boxes`]).

use super::detect::page_gutters;
use super::grid::{Line, text_lines};
use super::prims::{Word, merge_collinear, prims_with, words_of};
use super::stats::median;
use super::{MAX_PRIMS, MAX_WORDS, PageInput, PageTables, Rect, clip_to_page};

/// gate_feats.py `n_h`: horizontal rulings (after merging collinear pieces) at least this long, pt.
const GATE_MIN_HLINE: f64 = 20.0;
/// gate_feats.py `n_v`: vertical rulings at least this long, pt.
const GATE_MIN_VLINE: f64 = 8.0;
/// REPORT §7 rule G*: `n_h >= 3 and n_v >= 2`.
const GATE_HLINES: usize = 3;
const GATE_VLINES: usize = 2;
/// REPORT §7 rule G*: `run_g2 >= 2`.
const GATE_RUN: usize = 2;
/// gate_feats.py: words closer than this many font sizes (the line's median) form one segment.
const GATE_SEG_GAP_FS: f64 = 1.0;
/// gate_feats.py: neighbouring lines at most this many font sizes apart (`cy`) continue a run.
const GATE_RUN_GAP_FS: f64 = 3.5;
/// gate_feats.py: page cut bounds outside the gutters.
const GATE_CUT_INF: f64 = 1e9;
/// T18 image rule (first principles, not fitted): an image covering at least a tenth of the page can
/// hold a readable table (3 rows of 10-pt type over half a Letter page width is ~3 % of it; a tenth
/// leaves the thumbnails and logos out).
pub const LARGE_IMAGE_SHARE: f64 = 0.1;

/// Per-page quantities of the gate (measurement aid; [`may_contain_table`] computes only what it needs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GateFeatures {
    /// Words on the page (`nw`).
    pub words: usize,
    /// Horizontal rulings of at least 20 pt after merging collinear pieces (`n_h`).
    pub h_rulings: usize,
    /// Vertical rulings of at least 8 pt (`n_v`).
    pub v_rulings: usize,
    /// Longest run of neighbouring lines of at least 2 segments aligned at an inner segment start, in a
    /// part of the page between its layout gutters (`run_g2`).
    pub aligned_run: usize,
    /// An image covers at least [`LARGE_IMAGE_SHARE`] of the page.
    pub large_image: bool,
}

/// Features of a page; `None` over [`MAX_WORDS`] or [`MAX_PRIMS`] (as [`super::detect_tables`]).
pub fn gate_features(page: &PageInput) -> Option<GateFeatures> {
    let large_image = has_large_image(page);
    let clipped = clip_to_page(page, false);
    let page = clipped.as_ref().unwrap_or(page);
    // `rulings` checks the limits first.
    let (h_rulings, v_rulings) = rulings(page)?;
    let ws = words_of(page);
    Some(GateFeatures {
        words: ws.len(),
        h_rulings,
        v_rulings,
        aligned_run: aligned_run(&ws, page.width),
        large_image,
    })
}

/// T18 gate for SP5-R2: `true` when the region model should run on the page. `found` is
/// [`super::detect_tables`] of the same page without model regions (the pass the PDF path makes anyway);
/// its candidates, accepted or rejected, count as a table. A page skipped over a hostile-input limit gives
/// `false`: the detector skips it with model regions too. Otherwise `true` on a page without words (a
/// scan), with an image of at least [`LARGE_IMAGE_SHARE`] of the page ([`has_large_image`]), with at least
/// 3 horizontal and 2 vertical rulings, or with two neighbouring aligned text lines. Cheap checks first;
/// O(n log n) in words and primitives, bounded by [`MAX_WORDS`] and [`MAX_PRIMS`].
///
/// Contract for SP5-R2 (images cost a content-stream scan): call with [`PageInput::image_boxes`] empty;
/// only where it says no, fill them and ask [`has_large_image`]. The rules are ORed, so this equals one call
/// with the boxes filled.
pub fn may_contain_table(page: &PageInput, found: &PageTables) -> bool {
    if found.skipped.is_some() {
        return false;
    }
    if !found.tables.is_empty() || !found.rejected.is_empty() {
        return true;
    }
    // The image boxes are not clipped: read them from `page` itself.
    let clipped = clip_to_page(page, false);
    let (page, images) = (clipped.as_ref().unwrap_or(page), page);
    if page.words.len() > MAX_WORDS {
        return false;
    }
    if page.words.is_empty() || has_large_image(images) {
        return true;
    }
    let Some((h, v)) = rulings(page) else {
        return false;
    };
    if h >= GATE_HLINES && v >= GATE_VLINES {
        return true;
    }
    aligned_run(&words_of(page), page.width) >= GATE_RUN
}

/// `(n_h, n_v)` after merging collinear rulings; `None` over [`MAX_WORDS`] or the primitive limits.
fn rulings(page: &PageInput) -> Option<(usize, usize)> {
    if page.words.len() > MAX_WORDS || page.paths.len() + page.segments.len() > MAX_PRIMS {
        return None;
    }
    let mut pr = prims_with(page, false);
    if pr.hs.len() + pr.vs.len() + pr.rects.len() > MAX_PRIMS {
        return None;
    }
    merge_collinear(&mut pr);
    let h = pr.hs.iter().filter(|l| l.b - l.a >= GATE_MIN_HLINE).count();
    let v = pr.vs.iter().filter(|l| l.b - l.a >= GATE_MIN_VLINE).count();
    Some((h, v))
}

/// T18 image rule: an image of [`PageInput::image_boxes`] covers at least [`LARGE_IMAGE_SHARE`] of the page
/// (its part on the page). O(images).
pub fn has_large_image(page: &PageInput) -> bool {
    let area = page.width * page.height;
    page.image_boxes
        .iter()
        .any(|r| area_on_page(page, r).is_some_and(|a| a >= LARGE_IMAGE_SHARE * area))
}

/// Area of the part of `r` on the page; `None` when nothing of it is.
pub(crate) fn area_on_page(page: &PageInput, r: &Rect) -> Option<f64> {
    let w = (r.x + r.w).min(page.width) - r.x.max(0.0);
    let h = (r.top + r.h).min(page.height) - r.top.max(0.0);
    (w > 0.0 && h > 0.0).then_some(w * h)
}

/// gate_feats.py `run_g2`: the page is cut at its layout gutters ([`page_gutters`], the T15 guard's
/// valley); in each part, the longest run of lines of at least 2 segments where each line is within
/// [`GATE_RUN_GAP_FS`] of the previous one and shares an inner segment start with it (± its font size).
fn aligned_run(ws: &[Word], page_w: f64) -> usize {
    if ws.is_empty() {
        return 0;
    }
    let fs = median(&ws.iter().map(|w| w.fs).collect::<Vec<_>>());
    let mut cuts = page_gutters(ws, page_w, fs);
    cuts.sort_by(f64::total_cmp);
    cuts.insert(0, -GATE_CUT_INF);
    cuts.push(GATE_CUT_INF);
    let mut best = 0;
    for c in cuts.windows(2) {
        let part: Vec<&Word> = ws.iter().filter(|w| c[0] <= w.cx && w.cx < c[1]).collect();
        if !part.is_empty() {
            best = best.max(part_run(&text_lines(&part)));
        }
    }
    best
}

/// gate_feats.py `run2(2)` over the lines of one part.
fn part_run(lines: &[Line]) -> usize {
    let (mut best, mut cur) = (0usize, 0usize);
    // Previous line when it had at least 2 segments: its `cy` and finite inner starts (ascending).
    let mut prev: Option<(f64, Vec<f64>)> = None;
    for l in lines {
        let fs = median(&l.w.iter().map(|w| w.fs).collect::<Vec<_>>());
        let starts = segment_starts(l, fs);
        let ok = starts.len() >= 2;
        cur = match &prev {
            Some((cy, st)) if ok && l.cy - cy <= GATE_RUN_GAP_FS * fs => {
                if starts[1..].iter().any(|&s| near_any(st, s, fs)) {
                    cur + 1
                } else {
                    1
                }
            }
            _ => usize::from(ok),
        };
        best = best.max(cur);
        prev = ok.then(|| {
            (
                l.cy,
                starts[1..]
                    .iter()
                    .copied()
                    .filter(|s| s.is_finite())
                    .collect(),
            )
        });
    }
    best
}

/// Starts of the segments of a line (words closer than [`GATE_SEG_GAP_FS`] · `fs` join), in x order.
fn segment_starts(l: &Line, fs: f64) -> Vec<f64> {
    let (mut starts, mut end) = (Vec::new(), f64::NAN);
    for w in &l.w {
        if !starts.is_empty() && w.x0 - end < GATE_SEG_GAP_FS * fs {
            if w.x1 > end {
                end = w.x1;
            }
        } else {
            starts.push(w.x0);
            end = w.x1;
        }
    }
    starts
}

/// Some `x` of the ascending finite `st` has `|s - x| <= fs` (binary search; non-finite `s` never matches).
fn near_any(st: &[f64], s: f64, fs: f64) -> bool {
    if !s.is_finite() || fs.is_nan() {
        return false;
    }
    let i = st.partition_point(|&x| x < s - fs);
    st.get(i).is_some_and(|&x| x <= s + fs)
}

#[cfg(test)]
mod tests {
    use super::super::testkit::{grid_page, hline, input_word, vline};
    use super::super::{InputSegment, MAX_WORDS, TableConfig, detect_tables};
    use super::*;

    fn page() -> PageInput {
        PageInput {
            width: 600.0,
            height: 800.0,
            ..Default::default()
        }
    }

    fn gate(p: &PageInput) -> bool {
        may_contain_table(p, &detect_tables(p, &TableConfig::default()))
    }

    /// Prose: one segment per line, no rulings, no images.
    fn prose() -> PageInput {
        let mut p = page();
        for line in 0..20 {
            for k in 0..8 {
                p.words.push(input_word(
                    "lorem",
                    60.0 + 36.0 * k as f64,
                    80.0 + 14.0 * line as f64,
                    10.0,
                ));
            }
        }
        p
    }

    #[test]
    fn prose_page_is_not_passed() {
        let p = prose();
        let f = gate_features(&p).unwrap();
        assert_eq!((f.words, f.aligned_run, f.h_rulings), (160, 0, 0));
        assert!(!gate(&p));
    }

    #[test]
    fn page_without_words_is_passed() {
        assert!(gate(&page()));
    }

    #[test]
    fn candidate_of_the_detector_passes() {
        let mut p = grid_page(
            &[50.0, 150.0, 250.0, 350.0],
            &[100.0, 120.0, 140.0, 160.0],
            &[],
        );
        for (r, y) in [102.0, 122.0, 142.0].into_iter().enumerate() {
            for (c, x) in [55.0, 155.0, 255.0].into_iter().enumerate() {
                p.words.push(input_word(&format!("{r}{c}"), x, y, 10.0));
            }
        }
        let found = detect_tables(&p, &TableConfig::default());
        assert_eq!(found.tables.len(), 1);
        assert!(may_contain_table(&p, &found));
        // A rejected candidate counts too.
        let rejected = PageTables {
            rejected: found
                .tables
                .iter()
                .map(|t| super::super::Rejected {
                    kind: t.kind,
                    bbox: t.bbox,
                    reason: super::super::RejectReason::Tiny,
                    probability: t.probability,
                })
                .collect(),
            ..PageTables::default()
        };
        let words_only = prose();
        assert!(!may_contain_table(&words_only, &PageTables::default()));
        assert!(may_contain_table(&words_only, &rejected));
    }

    #[test]
    fn three_hlines_and_two_vlines_pass_without_a_candidate() {
        let mut p = prose();
        p.paths.extend([
            hline(60.0, 400.0, 500.0),
            hline(60.0, 400.0, 520.0),
            hline(60.0, 400.0, 540.0),
            vline(60.0, 600.0, 620.0),
            vline(400.0, 600.0, 620.0),
        ]);
        let f = gate_features(&p).unwrap();
        assert_eq!((f.h_rulings, f.v_rulings), (3, 2));
        // Without a candidate (no detection) the rulings alone decide.
        assert!(may_contain_table(&p, &PageTables::default()));
        p.paths.pop();
        assert!(!may_contain_table(&p, &PageTables::default()));
    }

    #[test]
    fn collinear_pieces_count_as_one_ruling() {
        let mut p = prose();
        // Three 15-pt pieces of one rule: one 45-pt ruling after the merge.
        p.paths.extend((0..3).map(|k| {
            let a = 60.0 + 15.0 * k as f64;
            hline(a, a + 15.0, 500.0)
        }));
        assert_eq!(gate_features(&p).unwrap().h_rulings, 1);
    }

    #[test]
    fn two_aligned_lines_pass() {
        let mut p = prose();
        // Two lines of two segments, the second segment starting at the same x.
        for (y, x2) in [(400.0, 300.0), (414.0, 304.0)] {
            p.words.push(input_word("key", 60.0, y, 10.0));
            p.words.push(input_word("12.5", x2, y, 10.0));
        }
        let f = gate_features(&p).unwrap();
        assert_eq!(f.aligned_run, 2);
        assert!(may_contain_table(&p, &PageTables::default()));
    }

    #[test]
    fn misaligned_or_distant_lines_do_not_pass() {
        let mut p = prose();
        // Inner starts 20 pt apart (> one font size), then aligned lines 50 pt apart (> 3.5 font sizes).
        for (y, x2) in [
            (400.0, 300.0),
            (414.0, 320.0),
            (600.0, 300.0),
            (650.0, 300.0),
        ] {
            p.words.push(input_word("key", 60.0, y, 10.0));
            p.words.push(input_word("12.5", x2, y, 10.0));
        }
        assert_eq!(gate_features(&p).unwrap().aligned_run, 1);
        assert!(!may_contain_table(&p, &PageTables::default()));
    }

    #[test]
    fn two_column_layout_is_cut_at_the_gutter() {
        // Two prose columns: across the page every line has 2 segments starting at the same x (aligned),
        // after the gutter cut each part has one segment per line.
        let mut p = page();
        for line in 0..30 {
            for (x0, n) in [(50.0, 6), (320.0, 6)] {
                for k in 0..n {
                    p.words.push(input_word(
                        "ipsum",
                        x0 + 38.0 * k as f64,
                        80.0 + 14.0 * line as f64,
                        10.0,
                    ));
                }
            }
        }
        assert_eq!(gate_features(&p).unwrap().aligned_run, 0);
        assert!(!may_contain_table(&p, &PageTables::default()));
    }

    #[test]
    fn large_image_passes_small_one_does_not() {
        let mut p = prose();
        p.image_boxes.push(Rect::new(50.0, 400.0, 100.0, 60.0));
        assert!(!gate_features(&p).unwrap().large_image);
        assert!(!may_contain_table(&p, &PageTables::default()));
        p.image_boxes.push(Rect::new(50.0, 400.0, 500.0, 200.0));
        assert!(gate_features(&p).unwrap().large_image);
        assert!(may_contain_table(&p, &PageTables::default()));
        // Only the part on the page counts.
        p.image_boxes = vec![Rect::new(-450.0, 400.0, 500.0, 200.0)];
        assert!(!may_contain_table(&p, &PageTables::default()));
    }

    #[test]
    fn skipped_page_is_not_passed() {
        let mut p = page();
        p.words = (0..=MAX_WORDS)
            .map(|i| input_word("ab", 10.0 + (i % 50) as f64 * 11.0, (i / 50) as f64, 1.0))
            .collect();
        let found = detect_tables(&p, &TableConfig::default());
        assert!(found.skipped.is_some());
        assert!(!may_contain_table(&p, &found));
        assert_eq!(gate_features(&p), None);
        let mut p = prose();
        p.segments = (0..=MAX_PRIMS)
            .map(|k| InputSegment {
                x1: 10.0,
                y1: k as f64 * 0.01,
                x2: 500.0,
                y2: k as f64 * 0.01,
                raster: false,
            })
            .collect();
        assert!(!may_contain_table(&p, &PageTables::default()));
    }

    /// Bounded: a page of `MAX_WORDS` words in 2-segment lines (every line a candidate for the run).
    #[test]
    fn gate_is_fast_on_a_20k_word_page() {
        let mut p = PageInput {
            width: 612.0,
            height: 30_000.0,
            ..Default::default()
        };
        p.words = (0..MAX_WORDS)
            .map(|i| {
                let x = 20.0 + 25.0 * (i % 20) as f64 + (i as f64 * 0.37) % 7.0;
                input_word("ab", x, 4.0 + 12.0 * (i / 20) as f64, 10.0)
            })
            .collect();
        let t = std::time::Instant::now();
        let f = gate_features(&p);
        let ms = t.elapsed().as_secs_f64() * 1e3;
        eprintln!("gate features 20000 words: {ms:.2} ms, {f:?}");
        assert!(f.is_some());
        if !cfg!(debug_assertions) {
            assert!(ms <= 50.0, "gate took {ms} ms");
        }
    }
}
