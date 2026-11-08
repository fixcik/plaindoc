//! Text blocks: `fs_tolerances`, wide blocks, `gmin_adapt`, `text_join`, numeric tokens, dense valleys.

use super::bounds::{adapt_valleys, region_lines, region_of};
use super::helpers_test::*;
use super::rules::detect_rules;
use super::text::detect_text;
use super::{CandidateKind, VALLEY_FS, VALLEY_MIN, candidates};
use crate::tables::grid::{text_lines, valleys};
use crate::tables::prims::{Region, Word, prims, words_of};
use crate::tables::testkit::{hline, input_word};
use crate::tables::{PageInput, TableConfig};

/// Task 22a: a 3-column, 4-row text grid as a scan gives it, word boxes jittered by ±0.25 fs (2.5 pt at
/// 10 pt). Uniform jitter alone stays within the 0.5-fs merge, so the value column carries the scan's
/// ink font size too: digits without ascenders give fs = 0.7 of the text (REPORT §8 (г): 20-30 % low),
/// boxes on the text baseline. On their own 0.7 fs a value 2.5 pt low misses its line; on the median
/// fs of the region it does not.
#[test]
fn noisy_word_boxes_group_into_rows() {
    let jitter = [2.5, -2.5, 2.5, -1.5];
    let mut ws = Vec::new();
    for (r, (name, unit, val)) in [
        ("Apples", "kg", "12.5"),
        ("Pears", "box", "7.25"),
        ("Plums", "kg", "30.0"),
        ("Grapes", "box", "4.10"),
    ]
    .into_iter()
    .enumerate()
    {
        let top = 100.0 + 20.0 * r as f64;
        let j = jitter[r];
        ws.push(input_word(name, 10.0, top - j / 2.0, 10.0));
        ws.push(input_word(unit, 150.0, top + j / 5.0, 10.0));
        // Ink fs 7 with the bottom on the text baseline (top + 3), shifted by the jitter.
        ws.push(input_word(val, 290.0, top + 3.0 + j, 7.0));
    }
    let page = PageInput {
        width: 600.0,
        height: 800.0,
        words: ws,
        ..Default::default()
    };
    let ws = words_of(&page);
    let refs: Vec<&Word> = ws.iter().collect();
    let off = TableConfig::default();
    let on = cfg_with(|c| c.fs_tolerances = true);
    assert!(!off.fs_tolerances && !TableConfig::reference().fs_tolerances);
    assert_eq!(region_lines(&refs, &on).len(), 4);
    assert!(region_lines(&refs, &off).len() > 4, "own ink fs splits");
    let t = detect_text(&ws, &[], &on).unwrap();
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].rows.len(), 4, "{:?}", t[0].rows);
    assert_eq!(t[0].rows[0], ["Apples", "kg", "12.5"]);
}

/// Text block of `ncols` one-token columns × 5 rows, 16 pt gaps, size 10.
fn wide_text(ncols: usize, t: &str) -> PageInput {
    let mut p = PageInput {
        width: 1200.0,
        height: 800.0,
        ..Default::default()
    };
    let w = 6.0 * t.chars().count() as f64;
    for r in 0..5 {
        for c in 0..ncols {
            p.words.push(input_word(
                t,
                20.0 + (w + 16.0) * c as f64,
                100.0 + 14.0 * r as f64,
                10.0,
            ));
        }
    }
    p
}

#[test]
fn text_blocks_up_to_text_max_cols() {
    let page = wide_text(20, "Abc");
    let ws = words_of(&page);
    let ncols = |cfg: &TableConfig| -> Vec<usize> {
        candidates(&page, &ws, &prims(&page), cfg)
            .unwrap()
            .iter()
            .filter(|c| c.kind == CandidateKind::Text)
            .map(|c| c.rows[0].len())
            .collect()
    };
    assert!(
        ncols(&TableConfig::reference()).is_empty(),
        "cut at 15 columns"
    );
    assert_eq!(
        ncols(&cfg_with(
            |c| c.text_max_cols = crate::tables::WIDE_TEXT_MAX_COLS
        )),
        vec![20]
    );
    assert!(ncols(&cfg_with(|c| c.text_max_cols = 19)).is_empty());
}

/// Top and bottom rules (two, so `rg_split` does not see the solid lines as captions) around a table at
/// 6.5 pt with 3-pt column gaps (`syn_343`): `ncols` columns of two-char words.
fn narrow_booktabs(ncols: usize) -> PageInput {
    let mut p = PageInput {
        width: 600.0,
        height: 800.0,
        ..Default::default()
    };
    let x1 = 60.0 + 15.0 * ncols as f64 - 3.0;
    for y in [100.0, 150.0] {
        p.paths.push(hline(60.0, x1, y));
    }
    for (r, top) in [103.0, 116.0, 126.0, 136.0].into_iter().enumerate() {
        for c in 0..ncols {
            let t = if r == 0 { "Hd" } else { "12" };
            p.words
                .push(input_word(t, 60.0 + 15.0 * c as f64, top, 6.5));
        }
    }
    p
}

#[test]
fn narrow_gaps_give_columns_with_gmin_adapt() {
    let page = narrow_booktabs(12);
    let ws = words_of(&page);
    let p = prims(&page);
    let off = detect_rules(&ws, &p, &[], 800.0, &cfg_with(|c| c.gmin_adapt = false)).unwrap();
    assert!(off.is_empty(), "3-pt gaps are below max(4, 0.7 fs)");
    let on = detect_rules(&ws, &p, &[], 800.0, &cfg_with(|c| c.gmin_adapt = true)).unwrap();
    assert_eq!(on.len(), 1);
    assert_eq!(on[0].rows.len(), 4);
    assert!(on[0].rows.iter().all(|r| r.len() == 12), "{:?}", on[0].rows);
}

#[test]
fn narrow_valleys_need_confirming_lines() {
    use crate::tables::testkit::word;
    let fs = 6.5;
    // A staircase of sparse lines (two adjacent cells each) tiles 12 narrow columns but no line
    // confirms them; full lines do.
    let stair: Vec<Word> = (0..6)
        .flat_map(|r| {
            (0..2).map(move |k| {
                word(
                    "12",
                    60.0 + 15.0 * (2 * r + k) as f64,
                    100.0 + 10.0 * r as f64,
                    fs,
                )
            })
        })
        .collect();
    let full: Vec<Word> = (0..6)
        .flat_map(|r| {
            (0..12).map(move |c| word("12", 60.0 + 15.0 * c as f64, 100.0 + 10.0 * r as f64, fs))
        })
        .collect();
    for (ws, want) in [(&stair, 0), (&full, 11)] {
        let rw: Vec<&Word> = ws.iter().collect();
        let lines = text_lines(&rw);
        assert!(valleys(&rw, VALLEY_MIN.max(VALLEY_FS * fs), 0).is_empty());
        assert_eq!(
            adapt_valleys(Vec::new(), &rw, &lines, fs, 0, false).len(),
            want
        );
    }
}

#[test]
fn wide_axis_labels_are_a_chart() {
    use crate::tables::RejectReason;
    let charts = |page: &PageInput, guard: bool| {
        let cfg = cfg_with(|c| {
            c.text_max_cols = crate::tables::WIDE_TEXT_MAX_COLS;
            c.axis_guard = guard;
        });
        crate::tables::detect_tables(page, &cfg)
            .rejected
            .iter()
            .filter(|r| r.reason == RejectReason::Chart)
            .count()
    };
    let axis = wide_text(18, "201");
    assert_eq!(charts(&axis, true), 1);
    assert_eq!(charts(&axis, false), 0);
    assert_eq!(
        charts(&wide_text(18, "Год"), true),
        0,
        "tokens of three letters"
    );
    assert_eq!(charts(&wide_text(15, "201"), true), 0, "up to 15 columns");
}

#[test]
fn t14_off_in_reference() {
    let r = TableConfig::reference();
    assert_eq!(r.text_max_cols, 15);
    assert!(!(r.gmin_adapt || r.axis_guard));
}

/// Borderless 3-column table (header + 3 rows, 10 pt, 12-pt line step) whose last cell wraps onto
/// `hang` hanging lines under the column start.
fn hanging_table(hang: usize) -> PageInput {
    let mut p = PageInput {
        width: 600.0,
        height: 800.0,
        ..Default::default()
    };
    let mut top = 100.0;
    for r in 0..4 {
        let row = if r == 0 {
            ["Name", "Qty", "Notes"]
        } else {
            ["Item", "12", "note"]
        };
        for (c, t) in row.iter().enumerate() {
            p.words
                .push(input_word(t, 60.0 + 140.0 * c as f64, top, 10.0));
        }
        top += 12.0;
        for _ in 0..if r == 0 { 0 } else { hang } {
            p.words.push(input_word("wrap", 340.0, top, 10.0));
            top += 12.0;
        }
    }
    p
}

#[test]
fn hanging_cell_lines_join_the_text_block() {
    let page = hanging_table(3);
    assert!(text_rows(&page, &cfg_with(|c| c.text_join = false)).is_empty());
    let on = text_rows(&page, &cfg_with(|c| c.text_join = true));
    assert_eq!(on.len(), 1, "{on:?}");
    assert_eq!(on[0].len(), 4);
    assert!(on[0].iter().all(|r| r.len() == 3));
    assert_eq!(on[0][1][2], "note wrap wrap wrap");
}

#[test]
fn at_most_six_hanging_lines() {
    let on = cfg_with(|c| c.text_join = true);
    let six = text_rows(&hanging_table(6), &on);
    assert_eq!(six.len(), 1);
    assert_eq!(six[0].len(), 4);
    assert!(text_rows(&hanging_table(7), &on).is_empty());
}

/// Six 3-segment lines with aligned starts, `step` pt apart (10 pt type).
fn padded_rows(step: f64) -> Vec<Word> {
    use crate::tables::testkit::word;
    (0..6)
        .flat_map(|r| {
            ["Item", "12", "3.50"]
                .into_iter()
                .enumerate()
                .map(move |(c, t)| word(t, 60.0 + 140.0 * c as f64, 100.0 + step * r as f64, 10.0))
        })
        .collect()
}

#[test]
fn aligned_rows_join_up_to_five_font_sizes_and_break_on_regions() {
    let nrows = |ws: &[Word], used: &[Region], join: bool| -> Vec<usize> {
        detect_text(ws, used, &text_cfg(join))
            .unwrap()
            .iter()
            .map(|t| t.rows.len())
            .collect()
    };
    let ws = padded_rows(45.0);
    assert!(nrows(&ws, &[], false).is_empty(), "4.5 fs is over 3 fs");
    assert_eq!(nrows(&ws, &[], true), vec![6]);
    assert!(
        nrows(&padded_rows(50.0), &[], true).is_empty(),
        "under 5 fs only"
    );
    // A region found earlier between rows 3 and 4 cuts the block.
    let reg = region_of([50.0, 200.0, 400.0, 225.0]);
    assert_eq!(nrows(&ws, &[reg], true), vec![3, 3]);
}

#[test]
fn trailing_prose_line_is_trimmed_from_the_block() {
    use crate::tables::testkit::word;
    let mut ws: Vec<Word> = (0..3)
        .flat_map(|r| {
            ["Item", "12", "3.50"]
                .into_iter()
                .enumerate()
                .map(move |(c, t)| word(t, 60.0 + 140.0 * c as f64, 100.0 + 12.0 * r as f64, 10.0))
        })
        .collect();
    // A wide prose line right under the rows, then a candidate line 4 fs below it (5.2 fs from the
    // last row: over the aligned step).
    ws.extend((0..8).map(|k| word("lorem", 60.0 + 33.0 * k as f64, 136.0, 10.0)));
    ws.extend(
        ["Total", "99", "x"]
            .into_iter()
            .enumerate()
            .map(|(c, t)| word(t, 60.0 + 140.0 * c as f64, 176.0, 10.0)),
    );
    let on = detect_text(&ws, &[], &text_cfg(true)).unwrap();
    assert_eq!(on.len(), 1);
    assert_eq!(on[0].rows.len(), 3);
    assert!(on[0].words.iter().all(|w| w.t != "lorem"));
    let off = detect_text(&ws, &[], &text_cfg(false)).unwrap();
    assert!(off.iter().all(|t| t.words.iter().any(|w| w.t == "lorem")));
}

/// Hostile input: one block of `MAX_WORDS` words, candidate rows with distinct segment starts each
/// followed by 5 hanging lines, so the start set grows to thousands of entries and every hanging
/// line queries it.
#[test]
fn text_join_is_fast_on_a_20k_word_block() {
    use crate::tables::MAX_WORDS;
    let mut p = PageInput {
        width: 612.0,
        height: 200_000.0,
        ..Default::default()
    };
    let mut top = 4.0;
    let mut r = 0usize;
    while p.words.len() < MAX_WORDS {
        let jit = (r as f64 * 0.0137) % 5.0;
        for c in 0..3 {
            p.words
                .push(input_word("ab", 20.0 + jit + 150.0 * c as f64, top, 10.0));
        }
        top += 12.0;
        for _ in 0..5 {
            p.words.push(input_word("cd", 320.0 + jit, top, 10.0));
            top += 12.0;
        }
        r += 1;
    }
    p.words.truncate(MAX_WORDS);
    let cfg = cfg_with(|c| c.text_join = true);
    let ws = words_of(&p);
    let t = std::time::Instant::now();
    let n = detect_text(&ws, &[], &text_cfg(true)).map_or(0, |v| v.len());
    let ms = t.elapsed().as_secs_f64() * 1e3;
    let t2 = std::time::Instant::now();
    let r = crate::tables::detect_tables(&p, &cfg);
    let ms2 = t2.elapsed().as_secs_f64() * 1e3;
    eprintln!(
        "detect_text (join) 20000 words: {ms:.2} ms, {n} blocks; detect_tables {ms2:.2} ms, skipped {:?}",
        r.skipped.is_some()
    );
    assert_eq!(n, 1, "one joined block");
    assert!(r.skipped.is_none(), "{:?}", r.skipped);
    if !cfg!(debug_assertions) {
        assert!(ms <= 400.0, "detect_text took {ms} ms");
        assert!(ms2 <= 1000.0, "detect_tables took {ms2} ms");
    }
}

#[test]
fn text_join_off_in_reference() {
    assert!(!TableConfig::reference().text_join);
}

/// Four rows `Item | 6 numeric tokens | note` (10 pt type): the middle segment has 6 tokens.
fn digit_groups() -> PageInput {
    let mut p = page();
    for r in 0..4 {
        let top = 100.0 + 12.0 * r as f64;
        p.words.push(input_word("Item", 60.0, top, 10.0));
        for k in 0..6 {
            p.words
                .push(input_word("120", 160.0 + 22.0 * k as f64, top, 10.0));
        }
        p.words.push(input_word("note", 360.0, top, 10.0));
    }
    p
}

#[test]
fn numeric_tokens_do_not_count_against_the_segment_limit() {
    let page = digit_groups();
    assert!(text_rows(&page, &cfg_with(|c| c.text_longseg = false)).is_empty());
    // `text_dense` off: with `gmin_adapt` its narrower retry would split the 4-pt token gaps.
    let on = text_rows(
        &page,
        &cfg_with(|c| {
            c.text_longseg = true;
            c.text_dense = false;
        }),
    );
    assert_eq!(on.len(), 1);
    assert_eq!((on[0].len(), on[0][0].len()), (4, 3));
    assert_eq!(on[0][0][1], "120 120 120 120 120 120");
}

/// Five rows of 4 columns (x 60, 160, 260, 360); the middle row is `Subtotal` plus one wide word
/// covering the gap between columns 2 and 3.
fn spanning_row() -> PageInput {
    let mut p = page();
    for r in 0..5 {
        let top = 100.0 + 12.0 * r as f64;
        if r == 2 {
            p.words.push(input_word("Subtotal", 60.0, top, 10.0));
            p.words
                .push(input_word("xxxxxxxxxxxxxxxxx", 165.0, top, 10.0));
            continue;
        }
        for (c, t) in ["Item", "12", "34", "56"].into_iter().enumerate() {
            p.words
                .push(input_word(t, 60.0 + 100.0 * c as f64, top, 10.0));
        }
    }
    p
}

#[test]
fn dense_valleys_in_text_blocks_keep_columns_under_a_spanning_row() {
    let page = spanning_row();
    let off = text_rows(&page, &cfg_with(|c| c.text_dense = false));
    assert_eq!(off.len(), 1);
    assert_eq!(off[0][0].len(), 3);
    let on = text_rows(&page, &cfg_with(|c| c.text_dense = true));
    assert_eq!(on.len(), 1);
    assert_eq!(on[0][0].len(), 4);
    // Without `dense_valleys` the text block takes the valleys of all its lines, as proto2's
    // `col_valleys`.
    let plain = text_rows(
        &page,
        &cfg_with(|c| {
            c.text_dense = true;
            c.dense_valleys = false;
        }),
    );
    assert_eq!(plain[0][0].len(), 3);
}
