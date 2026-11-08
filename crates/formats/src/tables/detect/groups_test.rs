//! Rule groups and fill stacks: booktabs, backgrounds, zebra stacks, captions, limits.

use super::fills::detect_fill_groups;
use super::helpers_test::*;
use super::rules::detect_rules;
use super::{Candidate, CandidateKind, candidates};
use crate::tables::prims::{prims, words_of};
use crate::tables::testkit::{hline, input_word};
use crate::tables::{PageInput, TableConfig};

#[test]
fn rule_groups_find_booktabs_with_rows_from_text() {
    let page = booktabs();
    let ws = words_of(&page);

    let c = candidates(&page, &ws, &prims(&page), &TableConfig::default()).unwrap();
    assert_eq!(c.len(), 1);
    assert_eq!(
        c[0].rows,
        vec![
            vec!["Name", "Qty", "Price"],
            vec!["Apples", "12", "3.50"],
            vec!["Pears", "7", "4.10"],
            vec!["Plums", "30", "1.25"],
        ]
    );
    assert_eq!(c[0].kind, CandidateKind::Text);
    assert_eq!(
        c[0].bbox,
        [60.0, 100.0, 400.0, 172.0],
        "bounded by the rules"
    );
}

#[test]
fn page_frame_rules_are_not_a_table() {
    // Journal header and footer rules of the same width enclose the whole page of prose.
    let mut page = PageInput {
        width: 600.0,
        height: 800.0,
        ..Default::default()
    };
    page.paths.push(hline(60.0, 540.0, 50.0));
    page.paths.push(hline(60.0, 540.0, 760.0));
    for i in 0..40 {
        for j in 0..8 {
            page.words.push(input_word(
                "word",
                60.0 + 58.0 * j as f64,
                60.0 + 17.0 * i as f64,
                10.0,
            ));
        }
    }
    let ws = words_of(&page);
    assert!(
        detect_rules(
            &ws,
            &prims(&page),
            &[],
            page.height,
            &TableConfig::default()
        )
        .unwrap()
        .is_empty()
    );
}

/// Ruled 3x3 table at (100..400, 100..190).
fn grid_3x3_page() -> PageInput {
    let cell = |r: usize, c: usize| {
        (
            format!("r{r}c{c}"),
            110.0 + 100.0 * c as f64,
            108.0 + 30.0 * r as f64,
        )
    };
    let cells: Vec<_> = (0..3)
        .flat_map(|r| (0..3).map(move |c| cell(r, c)))
        .collect();
    crate::tables::testkit::grid_page(
        &[100.0, 200.0, 300.0, 400.0],
        &[100.0, 130.0, 160.0, 190.0],
        &cells
            .iter()
            .map(|(t, x, y)| (t.as_str(), *x, *y))
            .collect::<Vec<_>>(),
    )
}

/// [`grid_3x3_page`] under a fill without stroke `fill` = `(x, top, w, h)`; other text lies inside a
/// wide fill but outside the table.
fn lined_candidates_under_fill(fill: [f64; 4], cfg: &TableConfig) -> Vec<Candidate> {
    let mut page = grid_3x3_page();
    for (i, t) in ["some", "free", "text", "lines", "below"]
        .iter()
        .enumerate()
    {
        page.words
            .push(input_word(t, 70.0 + 90.0 * i as f64, 300.0, 10.0));
    }
    page.paths.insert(
        0,
        crate::tables::InputPath {
            rect: crate::tables::Rect::new(fill[0], fill[1], fill[2], fill[3]),
            stroke: false,
            fill: true,
        },
    );
    let ws = words_of(&page);
    let mut p = prims(&page);
    if cfg.lined_skip_background {
        crate::tables::prims::mark_backgrounds(&mut p);
    }
    candidates(&page, &ws, &p, cfg).unwrap()
}

fn has_lined_table(c: &[Candidate]) -> bool {
    // The candidate is the table itself, not a region grown by the fills around it.
    c.iter().any(|c| {
        c.kind == CandidateKind::Lined
            && c.bbox[0] >= 99.0
            && c.bbox[1] >= 99.0
            && c.bbox[2] <= 401.0
            && c.bbox[3] <= 191.0
    })
}

#[test]
fn background_fill_is_left_out_of_lined_regions() {
    let on = cfg_with(|c| c.lined_skip_background = true);
    let off = cfg_with(|c| c.lined_skip_background = false);
    let bg = [60.0, 60.0, 480.0, 300.0];
    assert!(has_lined_table(&lined_candidates_under_fill(bg, &on)), "on");
    assert!(
        !has_lined_table(&lined_candidates_under_fill(bg, &off)),
        "off"
    );
    // The fill is the table frame (area < 2x): not a background, the table stays as it is.
    let frame = [100.0, 100.0, 300.0, 90.0];
    assert!(
        has_lined_table(&lined_candidates_under_fill(frame, &on)),
        "frame on"
    );
    assert!(
        has_lined_table(&lined_candidates_under_fill(frame, &off)),
        "frame off"
    );
}

#[test]
fn stacked_cell_fills_above_a_grid_are_backgrounds() {
    let mut page = grid_3x3_page();
    // Unstroked cell fills abutting each other, chaining up from the top ruling.
    for k in 0..6 {
        page.paths.push(crate::tables::InputPath {
            rect: crate::tables::Rect::new(40.0, 100.0 - 30.0 * (k + 1) as f64, 480.0, 30.0),
            stroke: false,
            fill: true,
        });
    }
    for k in 0..4 {
        page.words
            .push(input_word("lbl", 120.0, 70.0 - 30.0 * k as f64, 10.0));
    }
    let run = |on: bool| {
        let cfg = cfg_with(|c| c.lined_skip_background = on);
        let ws = words_of(&page);
        let mut p = prims(&page);
        if on {
            crate::tables::prims::mark_backgrounds(&mut p);
        }
        candidates(&page, &ws, &p, &cfg).unwrap()
    };
    assert!(has_lined_table(&run(true)), "on");
    assert!(!has_lined_table(&run(false)), "off");
}

#[test]
fn zebra_fill_stack_is_a_table() {
    let page = zebra(&[0, 2, 4]);
    let ws = words_of(&page);
    let p = prims(&page);
    let on = cfg_with(|c| c.fill_groups = true);
    let fg = detect_fill_groups(&ws, &p, &[], 600.0, 800.0, &on).unwrap();
    assert_eq!(fg.len(), 1);
    assert_eq!(fg[0].bbox, [60.0, 100.0, 420.0, 180.0]);
    assert_eq!(
        fg[0].colb,
        vec![60.0, 180.0, 300.0, 420.0],
        "columns from fill borders"
    );
    assert_eq!(
        fg[0].rowb,
        vec![100.0, 116.0, 132.0, 148.0, 164.0, 180.0],
        "rows from bands"
    );
    let want: Vec<Vec<String>> = (0..5)
        .map(|r| (0..3).map(|c| format!("r{r}c{c}")).collect())
        .collect();
    assert_eq!(fg[0].rows, want);
    let c = candidates(&page, &ws, &p, &on).unwrap();
    assert!(c.iter().any(|c| c.bbox == [60.0, 100.0, 420.0, 180.0]));
    let off = candidates(&page, &ws, &p, &cfg_with(|c| c.fill_groups = false)).unwrap();
    assert!(off.iter().all(|c| c.bbox != [60.0, 100.0, 420.0, 180.0]));
}

#[test]
fn single_filled_band_is_not_a_stack() {
    let page = zebra(&[2]);
    let ws = words_of(&page);
    let fg = detect_fill_groups(
        &ws,
        &prims(&page),
        &[],
        600.0,
        800.0,
        &TableConfig::default(),
    );
    assert!(fg.unwrap().is_empty());
}

#[test]
fn too_many_fills_skip_fill_groups() {
    let mut page = zebra(&[0, 2, 4]);
    // 2 000 small swatches of a chart legend elsewhere on the page: 2 009 fills, over the limit.
    page.paths.extend((0..2000).map(|k| {
        let (x, y) = (
            20.0 + 5.0 * (k % 100) as f64,
            400.0 + 5.0 * (k / 100) as f64,
        );
        fill(x, y, x + 3.0, y + 4.0)
    }));
    let ws = words_of(&page);
    let fg = detect_fill_groups(
        &ws,
        &prims(&page),
        &[],
        600.0,
        800.0,
        &TableConfig::default(),
    );
    assert!(fg.unwrap().is_empty());
}

#[test]
fn header_row_above_first_rule_is_taken() {
    let page = hlines_all(&[("Name", 70.0), ("Qty", 190.0), ("Price", 310.0)]);
    let ws = words_of(&page);
    let p = prims(&page);
    let on = detect_rules(&ws, &p, &[], 800.0, &TableConfig::default()).unwrap();
    assert_eq!(on.len(), 1);
    assert_eq!(on[0].bbox, [60.0, 104.0, 400.0, 163.0]);
    assert_eq!(on[0].rows[0], vec!["Name", "Qty", "Price"]);
    assert_eq!(on[0].rows.len(), 4);
    let off = detect_rules(&ws, &p, &[], 800.0, &cfg_with(|c| c.rg_extend = false)).unwrap();
    assert_eq!(off[0].bbox, [60.0, 118.0, 400.0, 163.0]);
    assert_eq!(off[0].rows.len(), 3);
}

#[test]
fn caption_above_is_not_taken() {
    let caption = [
        ("Table", 70.0),
        ("1:", 104.0),
        ("Prices", 120.0),
        ("of", 160.0),
        ("fruit", 176.0),
    ];
    let page = hlines_all(&caption);
    let ws = words_of(&page);
    let on = detect_rules(&ws, &prims(&page), &[], 800.0, &TableConfig::default()).unwrap();
    assert_eq!(on.len(), 1);
    assert_eq!(on[0].bbox, [60.0, 118.0, 400.0, 163.0]);
    assert_eq!(on[0].rows.len(), 3);
}

#[test]
fn total_row_does_not_kill_a_column() {
    // The total label runs across the gap between the number and name columns.
    let page = ruled(
        &[100.0, 118.0, 196.0],
        &[
            (
                103.0,
                &[
                    ("No", 70.0),
                    ("Name", 110.0),
                    ("Qty", 250.0),
                    ("Price", 330.0),
                ],
            ),
            (
                121.0,
                &[
                    ("1", 70.0),
                    ("Apples", 110.0),
                    ("12", 250.0),
                    ("3.50", 330.0),
                ],
            ),
            (
                136.0,
                &[("2", 70.0), ("Pears", 110.0), ("7", 250.0), ("4.10", 330.0)],
            ),
            (
                151.0,
                &[
                    ("3", 70.0),
                    ("Plums", 110.0),
                    ("30", 250.0),
                    ("1.25", 330.0),
                ],
            ),
            (
                166.0,
                &[
                    ("4", 70.0),
                    ("Grapes", 110.0),
                    ("5", 250.0),
                    ("2.00", 330.0),
                ],
            ),
            (
                181.0,
                &[
                    ("Total", 70.0),
                    ("for", 104.0),
                    ("all", 126.0),
                    ("items", 148.0),
                    ("10.85", 330.0),
                ],
            ),
        ],
    );
    let ws = words_of(&page);
    let p = prims(&page);
    let on = detect_rules(&ws, &p, &[], 800.0, &TableConfig::default()).unwrap();
    assert_eq!(on.len(), 1);
    assert_eq!(on[0].rows[0], vec!["No", "Name", "Qty", "Price"]);
    // gmin_adapt off as well: its 3.5-pt retry finds the 4-pt Total|for gap (proto2 does the same).
    let off = detect_rules(
        &ws,
        &p,
        &[],
        800.0,
        &cfg_with(|c| {
            c.dense_valleys = false;
            c.gmin_adapt = false;
        }),
    )
    .unwrap();
    assert_eq!(
        off[0].rows[0].len(),
        3,
        "the total row closes the No | Name valley"
    );
}

/// Word `t` of size `fs` and width `fs * 1.2` per char at (`x`, `top`).
fn wide_word(t: &str, x: f64, top: f64, fs: f64) -> crate::tables::InputWord {
    let mut w = input_word(t, x, top, fs);
    w.rect.w = 1.2 * fs * t.chars().count() as f64;
    w
}

#[test]
fn two_same_width_booktabs_with_caption_split() {
    // Two identical booktabs tables at y 100 and 300 (rules 60-400 pt) with a caption between them:
    // the six rules are one aligned group without the split (Task 5 review finding).
    let mut page = booktabs();
    let second = booktabs();
    page.paths.extend(second.paths.iter().map(|q| {
        let mut q = *q;
        q.rect.top += 200.0;
        q
    }));
    page.words.extend(second.words.iter().map(|w| {
        let mut w = w.clone();
        w.rect.top += 200.0;
        w
    }));
    for (t, x) in [("Table", 70.0), ("2:", 148.0), ("Prices", 184.0)] {
        page.words.push(wide_word(t, x, 230.0, 12.0));
    }
    let ws = words_of(&page);
    let p = prims(&page);
    let on = detect_rules(&ws, &p, &[], 800.0, &TableConfig::default()).unwrap();
    let boxes: Vec<[f64; 4]> = on.iter().map(|t| t.bbox).collect();
    assert_eq!(
        boxes,
        vec![[60.0, 100.0, 400.0, 172.0], [60.0, 300.0, 400.0, 372.0]]
    );
    assert!(on.iter().all(|t| t.rows.len() == 4));
    let off = detect_rules(&ws, &p, &[], 800.0, &cfg_with(|c| c.rg_split = false)).unwrap();
    assert_eq!(off.len(), 1, "without the split the tables merge");
}

#[test]
fn two_zebra_tables_with_caption_split() {
    // Two cell-filled zebra tables 45 pt apart (within the 3-band stack gap) with a caption between.
    let mut page = zebra(&[0, 2, 4]);
    let second = zebra(&[0, 2, 4]);
    page.paths.extend(second.paths.iter().map(|q| {
        let mut q = *q;
        q.rect.top += 125.0;
        q
    }));
    page.words.extend(second.words.iter().map(|w| {
        let mut w = w.clone();
        w.rect.top += 125.0;
        w
    }));
    for (t, x) in [("Table", 70.0), ("2:", 148.0), ("Prices", 184.0)] {
        page.words.push(wide_word(t, x, 186.0, 12.0));
    }
    let ws = words_of(&page);
    let p = prims(&page);
    let on = cfg_with(|c| c.fill_groups = true);
    let fg = detect_fill_groups(&ws, &p, &[], 600.0, 800.0, &on).unwrap();
    let boxes: Vec<[f64; 4]> = fg.iter().map(|t| t.bbox).collect();
    assert_eq!(
        boxes,
        vec![[60.0, 100.0, 420.0, 180.0], [60.0, 225.0, 420.0, 305.0]]
    );
    let off = cfg_with(|c| {
        c.fill_groups = true;
        c.fill_split = false;
    });
    let fg = detect_fill_groups(&ws, &p, &[], 600.0, 800.0, &off).unwrap();
    assert_eq!(fg.len(), 1, "without the cut the tables stack");
}

#[test]
fn illustration_page_gets_no_groups_but_keeps_lined_tables() {
    // A ruled 3 × 3 table above a drawing of 3 000 filled swatches and 6 000 short strokes: over both
    // A2 limits; the page is not skipped and the table is still found.
    let mut page = crate::tables::testkit::grid_page(
        &[50.0, 150.0, 250.0, 350.0],
        &[50.0, 70.0, 90.0, 110.0],
        &[],
    );
    page.width = 1000.0;
    page.height = 1400.0;
    for (r, y) in [52.0, 72.0, 92.0].into_iter().enumerate() {
        for (c, x) in [55.0, 155.0, 255.0].into_iter().enumerate() {
            page.words.push(input_word(&format!("{r}{c}"), x, y, 10.0));
        }
    }
    page.paths.extend((0..3000).map(|k| {
        let (x, y) = (20.0 + 8.0 * (k % 60) as f64, 200.0 + 8.0 * (k / 60) as f64);
        fill(x, y, x + 4.0, y + 4.0)
    }));
    page.paths.extend((0..6000).map(|k| {
        let (x, y) = (
            20.0 + 9.5 * (k % 100) as f64,
            650.0 + 10.0 * (k / 100) as f64,
        );
        hline(x, x + 6.0, y)
    }));
    let ws = words_of(&page);
    let p = prims(&page);
    let cfg = cfg_with(|c| c.fill_groups = true);
    assert!(
        detect_fill_groups(&ws, &p, &[], 1000.0, 1400.0, &cfg)
            .unwrap()
            .is_empty()
    );
    assert!(detect_rules(&ws, &p, &[], 1400.0, &cfg).unwrap().is_empty());
    let r = crate::tables::detect_tables(&page, &cfg);
    assert!(r.skipped.is_none());
    assert_eq!(r.tables.len(), 1);
}
