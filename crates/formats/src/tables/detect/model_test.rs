//! Layout-model regions (SP4-R1/R3).

use super::helpers_test::*;
use super::{Candidate, CandidateKind, candidates};
use crate::tables::prims::{prims, words_of};
use crate::tables::testkit::input_word;
use crate::tables::{PageInput, TableConfig};

/// SP4-R1: a borderless 3 x 5 table (words of size 10 at x 60/160/260, rows 14 pt apart from top 200)
/// under a prose line; `region` boxes are model regions.
fn borderless_with_regions(regions: &[[f64; 4]]) -> PageInput {
    let mut p = PageInput {
        width: 600.0,
        height: 800.0,
        ..Default::default()
    };
    p.words.push(input_word(
        "Prose line above the table that runs wide",
        60.0,
        170.0,
        10.0,
    ));
    let rows = [
        ["Name", "Qty", "Price"],
        ["apple", "3", "1.20"],
        ["pear", "12", "0.80"],
        ["plum", "7", "2.10"],
        ["fig", "5", "3.00"],
    ];
    for (r, row) in rows.iter().enumerate() {
        for (c, t) in row.iter().enumerate() {
            p.words.push(input_word(
                t,
                60.0 + 100.0 * c as f64,
                200.0 + 14.0 * r as f64,
                10.0,
            ));
        }
    }
    p.model_regions = regions.iter().map(|&b| model_region(b)).collect();
    p
}

fn with_model() -> TableConfig {
    TableConfig {
        region_model: Some(Default::default()),
        ..TableConfig::default()
    }
}

fn model_cands(p: &PageInput) -> Vec<Candidate> {
    model_cands_with(p, &with_model())
}

#[test]
fn model_region_snaps_to_text_lines() {
    // Loose box: 8 pt outside the table at the left and below, cutting "Price" (x 260..290) at the right
    // and stopping 3 pt (< 0.5 fs) short of the first line's centre, the prose line (centre 175) outside.
    let c = model_cands(&borderless_with_regions(&[[52.0, 208.0, 280.0, 264.0]]));
    assert_eq!(c.len(), 1, "one model candidate");
    assert_eq!(
        c[0].bbox,
        [60.0, 200.0, 290.0, 266.0],
        "snapped to the words"
    );
    assert_eq!((c[0].rows.len(), c[0].rows[0].len()), (5, 3));
    assert_eq!(c[0].rows[0], ["Name", "Qty", "Price"]);
    // No line centre inside: the region is dropped.
    assert!(model_cands(&borderless_with_regions(&[[300.0, 400.0, 500.0, 500.0]])).is_empty());
    // Without the switch the regions are ignored.
    let p = borderless_with_regions(&[[52.0, 208.0, 280.0, 264.0]]);
    let ws = words_of(&p);
    let plain = candidates(&p, &ws, &prims(&p), &TableConfig::default()).unwrap();
    assert!(plain.iter().all(|c| c.kind != CandidateKind::Model));
}

#[test]
fn region_replaces_text_candidate_inside() {
    let p = borderless_with_regions(&[]);
    let base = crate::tables::detect_tables(&p, &with_model()).tables;
    assert_eq!(base.len(), 1);
    assert_eq!(base[0].kind, CandidateKind::Text);
    let p = borderless_with_regions(&[[55.0, 195.0, 295.0, 270.0]]);
    let t = crate::tables::detect_tables(&p, &with_model()).tables;
    assert_eq!(t.len(), 1, "the text table gives way to the region: {t:?}");
    assert_eq!(t[0].kind, CandidateKind::Model);
    assert_eq!(t[0].bbox, [60.0, 200.0, 290.0, 266.0]);
    assert_eq!(
        (t[0].rows.len(), t[0].rows[0].len(), t[0].header_rows),
        (5, 3, 1)
    );
}

#[test]
fn lined_candidate_wins_over_overlapping_region() {
    let ys = [100.0, 120.0, 140.0, 160.0, 180.0];
    let cells = [
        "Name", "Qty", "Price", "apple", "3", "1.20", "pear", "12", "0.80", "plum", "7", "2.10",
    ];
    let words: Vec<(&str, f64, f64)> = cells
        .iter()
        .enumerate()
        .map(|(i, &t)| {
            (
                t,
                65.0 + 100.0 * (i % 3) as f64,
                105.0 + 20.0 * (i / 3) as f64,
            )
        })
        .collect();
    let mut p = crate::tables::testkit::grid_page(&[60.0, 160.0, 260.0, 360.0], &ys, &words);
    let base = crate::tables::detect_tables(&p, &with_model()).tables;
    assert_eq!(base.len(), 1);
    assert_eq!(base[0].kind, CandidateKind::Lined);
    p.model_regions.push(super::super::ModelRegion::new(
        super::super::Rect::new(62.0, 102.0, 290.0, 70.0),
        0.9,
    ));
    assert_eq!(model_cands(&p).len(), 1, "the region gives a candidate");
    let t = crate::tables::detect_tables(&p, &with_model()).tables;
    assert_eq!(t, base, "the lined table wins, the region is dropped");
}

/// A rule group (kind `T`, not a text block) wins over an overlapping region like a lined table (SP4-R1).
#[test]
fn rule_group_wins_over_overlapping_region() {
    let mut p = booktabs();
    let base = crate::tables::detect_tables(&p, &with_model()).tables;
    assert_eq!(base.len(), 1);
    assert_eq!(base[0].kind, CandidateKind::Text);
    p.model_regions
        .push(model_region([60.0, 100.0, 400.0, 172.0]));
    assert_eq!(model_cands(&p).len(), 1, "the region gives a candidate");
    assert_eq!(crate::tables::detect_tables(&p, &with_model()).tables, base);
}

fn model_region(b: [f64; 4]) -> super::super::ModelRegion {
    super::super::ModelRegion::new(
        super::super::Rect::new(b[0], b[1], b[2] - b[0], b[3] - b[1]),
        1.0,
    )
}

/// SP4-R3: a model region over a 5 x 3 zebra table (rows 28 pt from top 100, rows 1 and 3 filled across
/// x 55..420); the first cell of rows 1-4 wraps onto two lines.
fn zebra_region() -> PageInput {
    let mut p = page();
    for r in [1, 3] {
        let top = 100.0 + 28.0 * r as f64;
        p.paths.push(fill(55.0, top, 420.0, top + 28.0));
    }
    for r in 0..5 {
        let top = 100.0 + 28.0 * r as f64;
        let row: [&str; 3] = if r == 0 {
            ["Head", "Qty", "Val"]
        } else {
            ["long", "12", "3.5"]
        };
        for (c, t) in row.iter().enumerate() {
            p.words
                .push(input_word(t, 60.0 + 140.0 * c as f64, top + 3.0, 10.0));
        }
        if r > 0 {
            p.words.push(input_word("name", 60.0, top + 15.0, 10.0));
        }
    }
    p.model_regions
        .push(model_region([50.0, 95.0, 430.0, 245.0]));
    p
}

/// SP4-R3: a model region over 20 left-aligned columns 27 pt apart (x 30..), 6 rows 14 pt apart, no
/// rulings; words alternate 24 and 6 pt wide, so the gap between columns is 3 pt (no valley).
fn twenty_columns() -> PageInput {
    let mut p = page();
    for r in 0..6 {
        for c in 0..20 {
            let t = if (r + c) % 2 == 0 {
                format!("c{c:02}{r}")
            } else {
                format!("{r}")
            };
            p.words.push(input_word(
                &t,
                30.0 + 27.0 * c as f64,
                100.0 + 14.0 * r as f64,
                10.0,
            ));
        }
    }
    p.model_regions
        .push(model_region([25.0, 95.0, 580.0, 190.0]));
    p
}

fn with_bounds() -> TableConfig {
    TableConfig {
        region_model: Some(super::super::RegionModelConfig {
            inner_bounds: true,
            ..Default::default()
        }),
        ..TableConfig::default()
    }
}

fn model_cands_with(p: &PageInput, cfg: &TableConfig) -> Vec<Candidate> {
    let ws = words_of(p);
    candidates(p, &ws, &prims(p), cfg)
        .unwrap()
        .into_iter()
        .filter(|c| c.kind == CandidateKind::Model)
        .collect()
}

#[test]
fn zebra_rows_bound_rows() {
    let p = zebra_region();
    let c = model_cands_with(&p, &with_bounds());
    assert_eq!(c.len(), 1);
    assert_eq!(
        c[0].rows.len(),
        5,
        "rows from the filled bands: {:?}",
        c[0].rows
    );
    assert_eq!(c[0].rows[0], ["Head", "Qty", "Val"]);
    assert!(
        c[0].rows[1..]
            .iter()
            .all(|r| r == &["long name", "12", "3.5"])
    );
    // Without the rule a wrapped first cell starts a row.
    let off = model_cands_with(&p, &with_model());
    assert_eq!(off.len(), 1);
    assert_eq!(off[0].rows.len(), 9);
}

/// A shaded header and a shaded total row are not zebra: the four body rows between them stay rows.
#[test]
fn shaded_header_and_total_keep_body_rows() {
    let mut p = page();
    for r in [0, 5] {
        let top = 100.0 + 16.0 * r as f64;
        p.paths.push(fill(55.0, top, 420.0, top + 16.0));
    }
    for r in 0..6 {
        for c in 0..3 {
            p.words.push(input_word(
                &format!("r{r}c{c}"),
                60.0 + 140.0 * c as f64,
                103.0 + 16.0 * r as f64,
                10.0,
            ));
        }
    }
    p.model_regions
        .push(model_region([50.0, 95.0, 430.0, 200.0]));
    let c = model_cands_with(&p, &with_bounds());
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].rows.len(), 6, "body rows survive: {:?}", c[0].rows);
}

#[test]
fn many_columns_from_edge_clusters() {
    let p = twenty_columns();
    let c = model_cands_with(&p, &with_bounds());
    assert_eq!(c.len(), 1);
    assert_eq!((c[0].rows.len(), c[0].rows[0].len()), (6, 20));
    assert_eq!(c[0].rows[0][0], "c000");
    assert_eq!(c[0].rows[0][1], "0");
    assert_eq!(c[0].rows[1][19], "c191");
    // Valleys find no column: without the rule the region gives no grid.
    assert!(model_cands_with(&p, &with_model()).is_empty());
}

#[test]
fn without_model_flag_nothing_changes() {
    for p in [zebra_region(), twenty_columns()] {
        let mut bare = p.clone();
        bare.model_regions.clear();
        assert!(model_cands_with(&p, &TableConfig::default()).is_empty());
        let want = crate::tables::detect_tables(&bare, &TableConfig::default());
        assert_eq!(
            crate::tables::detect_tables(&p, &TableConfig::default()),
            want
        );
        // The switch on with the R3 rules gives the same output on a page without regions.
        assert_eq!(crate::tables::detect_tables(&bare, &with_bounds()), want);
    }
}
