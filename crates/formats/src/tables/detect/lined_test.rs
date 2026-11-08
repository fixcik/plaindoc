//! Lined candidates: word index, A2/A3 flags, stacked verticals, first column, `used_lined_only`.

use super::bounds::inside;
use super::helpers_test::*;
use super::lined::{WordIndex, detect_lined};
use super::text::detect_text;
use super::{CandidateKind, candidates};
use crate::tables::prims::{Region, Word, prims, regions, words_of};
use crate::tables::testkit::input_word;
use crate::tables::{MAX_HEADER_LINES, PageInput, TableConfig};

#[test]
fn word_index_matches_the_linear_scan() {
    let mut seed = 0x2545_F491_4F6C_DD1D_u64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    let ws: Vec<Word> = (0..300)
        .map(|k| {
            let mut w = crate::tables::testkit::word(
                "ab",
                next(80) as f64 * 3.0,
                next(80) as f64 * 3.0,
                10.0,
            );
            match k % 41 {
                0 => w.cx = f64::NAN,
                1 => w.cy = f64::NAN,
                2 => w.cx = f64::INFINITY,
                _ => {}
            }
            w
        })
        .collect();
    let index = WordIndex::new(&ws);
    for case in 0..2000 {
        let (x0, y0) = (next(90) as f64 * 3.0 - 10.0, next(90) as f64 * 3.0 - 10.0);
        let mut r = Region {
            x0,
            y0,
            x1: x0 + next(120) as f64,
            y1: y0 + next(120) as f64,
            members: vec![],
        };
        match case % 50 {
            0 => r.x0 = f64::NAN,
            1 => r.y1 = f64::INFINITY,
            2 => r.x1 = f64::NEG_INFINITY,
            _ => {}
        }
        for m in [0.0, 1.0, 2.0] {
            let want: Vec<usize> = (0..ws.len()).filter(|&i| inside(&ws[i], &r, m)).collect();
            assert_eq!(index.inside(&ws, &r, m), want, "case {case} m {m}");
        }
    }
}

#[test]
fn a2_off_in_reference() {
    let r = TableConfig::reference();
    assert!(!(r.fill_groups || r.rg_extend || r.dense_valleys || r.rg_split || r.fill_split));
}

/// Outer 3x4 grid (rows 100/200/240/280) with a 3x2 grid inside the tall cell (0,1); the inner rulings
/// do not touch the outer ones.
fn nested_page() -> PageInput {
    use crate::tables::testkit::{grid_page, hline, input_word, vline};
    let mut p = grid_page(
        &[50.0, 150.0, 250.0, 350.0, 450.0],
        &[100.0, 200.0, 240.0, 280.0],
        &[],
    );
    for y in [115.0, 145.0, 175.0] {
        p.paths.push(hline(165.0, 235.0, y));
    }
    for x in [165.0, 200.0, 235.0] {
        p.paths.push(vline(x, 115.0, 175.0));
    }
    for (i, t) in ["i00", "i01", "i10", "i11"].iter().enumerate() {
        let (c, r) = (i % 2, i / 2);
        p.words.push(input_word(
            t,
            170.0 + 35.0 * c as f64,
            125.0 + 30.0 * r as f64,
            10.0,
        ));
    }
    for r in 0..3 {
        for c in 0..4 {
            if (r, c) == (0, 1) {
                continue;
            }
            let top = [140.0, 215.0, 255.0][r];
            p.words.push(input_word(
                &format!("r{r}c{c}"),
                60.0 + 100.0 * c as f64,
                top,
                10.0,
            ));
        }
    }
    p
}

#[test]
fn inner_grid_lines_do_not_split_outer_rows() {
    let page = nested_page();
    let ws = words_of(&page);
    let p = prims(&page);
    let shape = |cfg: &TableConfig| {
        let c = candidates(&page, &ws, &p, cfg).unwrap();
        let l = c.iter().find(|c| c.kind == CandidateKind::Lined).unwrap();
        (l.rows.len(), l.rows[0].len())
    };
    assert_eq!(shape(&cfg_with(|c| c.comp_lines = true)), (3, 4));
    assert_ne!(shape(&cfg_with(|c| c.comp_lines = false)), (3, 4));
}

#[test]
fn comp_lines_off_in_reference() {
    assert!(!TableConfig::reference().comp_lines);
}

/// Frame of 3 columns × 4 rows whose left border is missing, row labels in a column left of it.
fn labelled_grid() -> PageInput {
    let mut p = crate::tables::testkit::grid_page(
        &[250.0, 350.0, 450.0, 550.0],
        &[100.0, 140.0, 180.0, 220.0, 260.0],
        &[],
    );
    for r in 0..4 {
        let top = 110.0 + 40.0 * r as f64;
        p.words
            .push(input_word(&format!("Row{r}"), 180.0, top, 10.0));
        for c in 0..3 {
            p.words.push(input_word(
                &format!("v{r}{c}"),
                260.0 + 100.0 * c as f64,
                top,
                10.0,
            ));
        }
    }
    p
}

fn lined_cols(page: &PageInput, cfg: &TableConfig) -> usize {
    let ws = words_of(page);
    let c = candidates(page, &ws, &prims(page), cfg).unwrap();
    let l = c.iter().find(|c| c.kind == CandidateKind::Lined).unwrap();
    l.rows[0].len()
}

/// Region 300×100 (x 50..350, y 100..200) framed by segments; inner columns at x = 100/200/300 are pairs
/// of 12-pt pieces 8 pt apart (24 % of the height without the gap, 32 % with it), the lower piece
/// touching nothing (a stray one-primitive component).
fn stacked_columns(raster: bool) -> PageInput {
    let mut p = PageInput {
        width: 600.0,
        height: 800.0,
        ..Default::default()
    };
    let mut seg = |x1: f64, y1: f64, x2: f64, y2: f64| {
        p.segments.push(crate::tables::InputSegment {
            x1,
            y1,
            x2,
            y2,
            raster,
        })
    };
    for y in [100.0, 200.0] {
        seg(50.0, y, 350.0, y);
    }
    for x in [50.0, 350.0] {
        seg(x, 100.0, x, 200.0);
    }
    for x in [100.0, 200.0, 300.0] {
        seg(x, 100.0, x, 112.0);
        seg(x, 120.0, x, 132.0);
    }
    for r in 0..3 {
        for (c, x) in [60.0, 140.0, 240.0, 320.0].into_iter().enumerate() {
            p.words.push(input_word(
                &format!("v{r}{c}"),
                x,
                140.0 + 20.0 * r as f64,
                10.0,
            ));
        }
    }
    p
}

fn stacked_colb(page: &PageInput, cfg: &TableConfig) -> Vec<f64> {
    let ws = words_of(page);
    let p = crate::tables::prims::prims_with(page, cfg.short_vertical_stacks);
    let raws = detect_lined(&ws, &p, &regions(&p), cfg).unwrap();
    assert_eq!(raws.len(), 1);
    raws[0].colb.clone()
}

#[test]
fn short_vertical_stacks_make_columns() {
    let on = cfg_with(|c| c.short_vertical_stacks = true);
    let off = cfg_with(|c| c.short_vertical_stacks = false);
    assert_eq!(
        stacked_colb(&stacked_columns(true), &on),
        vec![50.0, 100.0, 200.0, 300.0, 350.0]
    );
    // Without the flag: no column ruling covers 30 %, so the edges and the text valleys.
    let valleys = stacked_colb(&stacked_columns(true), &off);
    assert_eq!((valleys[0], valleys[valleys.len() - 1]), (50.0, 350.5));
    assert!(
        !valleys.contains(&100.0) && !valleys.contains(&200.0),
        "{valleys:?}"
    );
    // PDF (non-raster) segments: the flag changes nothing.
    assert_eq!(stacked_colb(&stacked_columns(false), &on), valleys);
}

#[test]
fn extend_first_column_takes_left_labels() {
    let page = labelled_grid();
    assert_eq!(
        lined_cols(&page, &cfg_with(|c| c.extend_first_column = true)),
        4
    );
    assert_eq!(
        lined_cols(&page, &cfg_with(|c| c.extend_first_column = false)),
        3
    );
}

#[test]
fn running_text_at_the_label_margin_blocks_the_extension() {
    // Known limitation of the page-wide x-band rule (plan wording: >= 80 % of the page words in the
    // labels' x-band lie inside the region's y-band): paragraphs above and below the table start at
    // the same left margin as the row labels, so their words fall into the band outside the table and
    // the share drops under 0.8. The labels of `labelled_grid` are not taken once the page has text.
    let mut page = labelled_grid();
    for k in 0..10 {
        for (top, n) in [(20.0 + 6.0 * k as f64, k), (300.0 + 6.0 * k as f64, 10 + k)] {
            page.words
                .push(input_word(&format!("Text{n}"), 180.0, top, 4.0));
        }
    }
    assert_eq!(
        lined_cols(&page, &cfg_with(|c| c.extend_first_column = true)),
        3
    );
}

#[test]
fn line_numbers_in_the_margin_are_not_a_column() {
    // Legal-document line numbers 1..30 at x = 40 beside a 3-column table starting at x = 80: three of
    // the table's text lines carry a number, but the numbers run far above and below its band.
    let mut p = crate::tables::testkit::grid_page(
        &[80.0, 180.0, 280.0, 380.0],
        &[95.0, 135.0, 175.0, 215.0],
        &[],
    );
    for r in 0..3 {
        for c in 0..3 {
            p.words.push(input_word(
                &format!("v{r}{c}"),
                90.0 + 100.0 * c as f64,
                100.0 + 40.0 * r as f64,
                10.0,
            ));
        }
    }
    for k in 0..30 {
        p.words.push(input_word(
            &format!("{}", k + 1),
            40.0,
            20.0 + 20.0 * k as f64,
            10.0,
        ));
    }
    assert_eq!(
        lined_cols(&p, &cfg_with(|c| c.extend_first_column = true)),
        3
    );
}

/// `used_lined_only`: a component of invisible rectangles (stroke and fill off) that yields no lined
/// candidate no longer silences the text detector over the 5x3 text table inside it.
#[test]
fn used_lined_only_frees_text_under_invisible_rects() {
    let mut p = PageInput {
        width: 600.0,
        height: 800.0,
        ..Default::default()
    };
    // Three touching invisible boxes around the table.
    for k in 0..3 {
        p.paths.push(crate::tables::InputPath {
            rect: crate::tables::Rect::new(80.0 + 100.0 * k as f64, 90.0, 110.0, 80.0),
            stroke: false,
            fill: false,
        });
    }
    text_grid(&mut p, 100.0);
    // A caption line across the whole component closes every column valley, so no lined candidate
    // (one column) is built; it is no table row (one segment).
    let mut x = 95.0;
    for t in [
        "Quarterly",
        "totals",
        "by",
        "region",
        "and",
        "item",
        "for",
        "the",
        "whole",
        "year",
        "under",
        "review",
    ] {
        p.words.push(input_word(t, x, 160.0, 10.0));
        x += 6.0 * t.chars().count() as f64 + 3.0;
    }
    let ws = words_of(&p);
    let pr = prims(&p);
    assert!(!regions(&pr).is_empty());
    let kinds = |cfg: &TableConfig| -> Vec<CandidateKind> {
        candidates(&p, &ws, &pr, cfg)
            .unwrap()
            .iter()
            .map(|c| c.kind)
            .collect()
    };
    assert!(TableConfig::default().used_lined_only);
    let off = TableConfig {
        used_lined_only: false,
        ..TableConfig::default()
    };
    let k_off = kinds(&off);
    assert!(!k_off.contains(&CandidateKind::Lined));
    assert!(!k_off.contains(&CandidateKind::Text));
    let on = TableConfig {
        used_lined_only: true,
        ..TableConfig::default()
    };
    let k_on = kinds(&on);
    assert!(k_on.contains(&CandidateKind::Text));
    assert!(!k_on.contains(&CandidateKind::Lined));
}

/// A 3-column, 5-row text table, column `j` at `x0 + 100 j`.
fn text_grid(p: &mut PageInput, x0: f64) {
    let cells = [
        ["Name", "Qty", "Price"],
        ["alpha", "12", "3.50"],
        ["beta", "7", "14.00"],
        ["gamma", "150", "0.25"],
        ["delta", "33", "9.99"],
    ];
    for (i, row) in cells.iter().enumerate() {
        for (j, t) in row.iter().enumerate() {
            p.words.push(input_word(
                t,
                x0 + 100.0 * j as f64,
                100.0 + 14.0 * i as f64,
                10.0,
            ));
        }
    }
}

/// `used_lined_only`: three separate stroked boxes (one text column each, so no lined candidate) with
/// aligned text across them are three freed components; the `T` candidate over all of them straddles
/// them and is not built, with or without the flag.
#[test]
fn used_lined_only_does_not_build_text_across_several_freed_boxes() {
    let mut p = PageInput {
        width: 600.0,
        height: 800.0,
        ..Default::default()
    };
    for k in 0..3 {
        p.paths.push(crate::tables::InputPath {
            rect: crate::tables::Rect::new(90.0 + 100.0 * k as f64, 90.0, 60.0, 80.0),
            stroke: true,
            fill: false,
        });
    }
    text_grid(&mut p, 100.0);
    let ws = words_of(&p);
    let pr = prims(&p);
    assert!(regions(&pr).len() >= 3);
    // Without any zone the text detector does see the table (the boxes are what hides it).
    assert!(
        !detect_text(&ws, &[], &TableConfig::default())
            .unwrap()
            .is_empty()
    );
    for flag in [false, true] {
        let cfg = TableConfig {
            used_lined_only: flag,
            ..TableConfig::default()
        };
        let k: Vec<_> = candidates(&p, &ws, &pr, &cfg)
            .unwrap()
            .iter()
            .map(|c| c.kind)
            .collect();
        assert!(!k.contains(&CandidateKind::Text), "flag {flag}: {k:?}");
    }
}

/// `used_lined_only`: a real lined candidate still blocks the text detector over its box.
#[test]
fn used_lined_only_keeps_lined_candidate_blocking_text() {
    let mut p = PageInput {
        width: 600.0,
        height: 800.0,
        ..Default::default()
    };
    for i in 0..5 {
        for j in 0..3 {
            p.paths.push(crate::tables::InputPath {
                rect: crate::tables::Rect::new(
                    90.0 + 100.0 * j as f64,
                    96.0 + 14.0 * i as f64,
                    100.0,
                    14.0,
                ),
                stroke: true,
                fill: false,
            });
        }
    }
    text_grid(&mut p, 100.0);
    let ws = words_of(&p);
    let pr = prims(&p);
    for flag in [false, true] {
        let cfg = TableConfig {
            used_lined_only: flag,
            ..TableConfig::default()
        };
        let k: Vec<_> = candidates(&p, &ws, &pr, &cfg)
            .unwrap()
            .iter()
            .map(|c| c.kind)
            .collect();
        assert!(k.contains(&CandidateKind::Lined), "flag {flag}: {k:?}");
        assert!(!k.contains(&CandidateKind::Text), "flag {flag}: {k:?}");
    }
}

#[test]
fn t15_off_in_reference_on_by_default() {
    let (r, d) = (TableConfig::reference(), TableConfig::default());
    assert_eq!(
        (r.text_longseg, r.text_dense, r.gutter_prose),
        (false, false, 0)
    );
    assert_eq!(
        (d.text_longseg, d.text_dense, d.gutter_prose),
        (true, true, 3)
    );
    assert_eq!(
        (d.row_newcols, d.header_lines, d.header_split),
        (2, MAX_HEADER_LINES, true)
    );
    assert!(d.frame_prose && !r.frame_prose);
}
