//! Header rows above stacks (T17) and the R4 rule-group flags (`band_split`, regular rules,
//! filled header, multi-segment captions).

use super::helpers_test::*;
use super::rules::{RULE_GROUP_MAX_H_FRAC, detect_rules};
use super::{CandidateKind, candidates};
use crate::tables::prims::{prims, words_of};
use crate::tables::testkit::input_word;
use crate::tables::{PageInput, TableConfig};

/// Zebra of 5 columns (80 pt from x 60), rows 0, 2, 4 of 5 filled per cell (16 pt from y 100), word
/// `rRcC` in each cell, and header lines `(top, words)` above the first band.
fn zebra5(header: &[(f64, &[(&str, f64)])]) -> PageInput {
    let mut p = page();
    for r in [0, 2, 4] {
        let top = 100.0 + 16.0 * r as f64;
        for c in 0..5 {
            let x = 60.0 + 80.0 * c as f64;
            p.paths.push(fill(x, top, x + 80.0, top + 16.0));
        }
    }
    for r in 0..5 {
        for c in 0..5 {
            let (x, top) = (70.0 + 80.0 * c as f64, 103.0 + 16.0 * r as f64);
            p.words.push(input_word(&format!("r{r}c{c}"), x, top, 10.0));
        }
    }
    for &(top, ws) in header {
        for &(t, x) in ws {
            p.words.push(input_word(t, x, top, 10.0));
        }
    }
    p
}

/// `Temp` and `Rain` over `max min` pairs (two-line grouped header).
const GROUPS: (f64, &[(&str, f64)]) = (73.0, &[("City", 70.0), ("Temp", 150.0), ("Rain", 310.0)]);
const SUBS: (f64, &[(&str, f64)]) = (
    87.0,
    &[
        ("max", 150.0),
        ("min", 230.0),
        ("max", 310.0),
        ("min", 390.0),
    ],
);

fn t17(lines: usize, split: bool) -> TableConfig {
    cfg_with(|c| {
        c.header_lines = lines;
        c.header_split = split;
    })
}

#[test]
fn header_of_up_to_three_lines_above_a_stack() {
    let page = zebra5(&[GROUPS, SUBS]);
    let one = stack(&page, &t17(1, false));
    assert_eq!((one.bbox[1], one.rows.len()), (86.0, 6), "one line (A2)");
    assert_eq!(one.rows[0], vec!["", "max", "min", "max", "min"]);
    let three = stack(&page, &t17(3, false));
    assert_eq!(
        (three.bbox[1], three.rows.len()),
        (72.0, 6),
        "both lines, one row"
    );
    assert_eq!(
        three.rows[0],
        vec!["City", "Temp max", "min", "Rain max", "min"]
    );
    assert_eq!(three.head, 0);
}

#[test]
fn split_header_gives_header_rows() {
    let page = zebra5(&[GROUPS, SUBS]);
    let cfg = t17(3, true);
    let t = stack(&page, &cfg);
    assert_eq!(t.rows.len(), 7);
    assert_eq!(t.rows[0], vec!["City", "Temp", "", "Rain", ""]);
    assert_eq!(t.rows[1], vec!["", "max", "min", "max", "min"]);
    assert_eq!(t.head, 2);
    // The header rows reach the table block (D5: header_rows < rows).
    let ws = words_of(&page);
    let cands = candidates(&page, &ws, &prims(&page), &cfg).unwrap();
    let sel = crate::tables::select::select(
        cands,
        &TableConfig {
            filters: false,
            ..cfg
        },
    );
    let g = sel.tables.iter().find(|g| g.bbox[1] == 72.0).unwrap();
    assert_eq!((g.header_rows, g.rows.len()), (2, 7));
    let off = stack(&page, &t17(3, false));
    assert_eq!(off.head, 0, "no split, no header count");
}

#[test]
fn second_header_line_may_sit_in_one_column() {
    // `Давление,` / `Па`: a one-segment line inside a column is taken after the first line only.
    let unit: (f64, &[(&str, f64)]) = (73.0, &[("Units", 310.0)]);
    let t = stack(&zebra5(&[unit, SUBS]), &t17(3, false));
    assert_eq!(t.bbox[1], 72.0);
    let alone = stack(&zebra5(&[(87.0, &[("Units", 310.0)])]), &t17(3, false));
    assert_eq!(
        alone.bbox[1], 100.0,
        "a one-segment line right above the stack is a caption"
    );
    let caption: (f64, &[(&str, f64)]) = (
        73.0,
        &[
            ("Table", 70.0),
            ("of", 100.0),
            ("prices", 116.0),
            ("in", 158.0),
            ("towns", 174.0),
        ],
    );
    let t = stack(&zebra5(&[caption, SUBS]), &t17(3, false));
    assert_eq!(t.bbox[1], 86.0, "a caption across columns is not taken");
}

#[test]
fn header_lines_are_bounded_by_three() {
    let lines: Vec<(f64, &[(&str, f64)])> =
        (0..5).map(|k| (31.0 + 14.0 * k as f64, SUBS.1)).collect();
    let page = zebra5(&lines);
    let three = stack(&page, &t17(3, false));
    assert_eq!(three.bbox[1], 58.0);
    assert_eq!(stack(&page, &t17(100, false)).bbox, three.bbox);
}

#[test]
fn t17_off_in_reference() {
    let r = TableConfig::reference();
    assert_eq!(
        (r.row_newcols, r.header_lines, r.header_split),
        (0, 1, false)
    );
}

/// Words of a header and `body` rows of 4 columns (pitch 14 pt from `top`).
fn band_words(top: f64, body: usize) -> Vec<(String, f64, f64)> {
    let mut v = Vec::new();
    for i in 0..=body {
        for j in 0..4 {
            let t = if i == 0 {
                format!("H{j}")
            } else {
                format!("r{i}c{j}")
            };
            v.push((t, 100.0 + 100.0 * j as f64, top + 14.0 * i as f64));
        }
    }
    v
}

fn lined_rows(p: &PageInput, cfg: &TableConfig) -> Vec<usize> {
    let ws = words_of(p);
    let pr = prims(p);
    candidates(p, &ws, &pr, cfg)
        .unwrap()
        .iter()
        .filter(|c| c.kind == CandidateKind::Lined)
        .map(|c| c.rows.len())
        .collect()
}

/// `band_split`: a booktabs table (rules above, under the header and below) gets its 6 body rows from
/// the text; without the flag the body is one row. A full grid is unchanged.
#[test]
fn band_split_cuts_booktabs_body_by_text() {
    let on = TableConfig {
        band_split: true,
        ..TableConfig::default()
    };
    let off = TableConfig::default();
    assert!(!off.band_split);
    let w = band_words(96.0, 6);
    let wr: Vec<(&str, f64, f64)> = w.iter().map(|(t, x, y)| (t.as_str(), *x, *y)).collect();
    let mut booktabs = crate::tables::testkit::grid_page(&[], &[], &wr);
    for y in [90.0, 112.0, 190.0] {
        booktabs
            .paths
            .push(crate::tables::testkit::hline(90.0, 490.0, y));
    }
    // One side rule joins the three rules into a region (rules alone make a rule group, kind `T`).
    booktabs
        .paths
        .push(crate::tables::testkit::vline(90.0, 90.0, 190.0));
    // 3 rules -> two bands: the header and the body (one row per band without the flag).
    assert_eq!(lined_rows(&booktabs, &off), vec![2]);
    assert_eq!(lined_rows(&booktabs, &on), vec![7]);
    // A full grid keeps its rows (the planar partition applies, nothing to split).
    let ys = [90.0, 112.0, 126.0, 140.0, 154.0, 168.0, 182.0, 196.0];
    let grid = crate::tables::testkit::grid_page(&[90.0, 190.0, 290.0, 390.0, 490.0], &ys, &wr);
    assert_eq!(lined_rows(&grid, &on), lined_rows(&grid, &off));
}

/// SP4-R4: a table of rules between x 60 and 420 only (no verticals): rules at `ys`, the given number
/// of text lines (12 pt step, 4 pt under the rule) in 3 columns in each interval.
fn ruled_table(ys: &[f64], lines_per_gap: &[usize]) -> PageInput {
    let mut p = page();
    for &y in ys {
        p.paths.push(crate::tables::testkit::hline(60.0, 420.0, y));
    }
    for (i, &n) in lines_per_gap.iter().enumerate() {
        for l in 0..n {
            for c in 0..3 {
                p.words.push(input_word(
                    &format!("r{i}l{l}c{c}"),
                    70.0 + 120.0 * c as f64,
                    ys[i] + 4.0 + 12.0 * l as f64,
                    10.0,
                ));
            }
        }
    }
    p
}

/// `(rows, columns)` of every rule-group candidate.
fn rules_of(p: &PageInput, cfg: &TableConfig) -> Vec<(usize, usize)> {
    let ws = words_of(p);
    detect_rules(&ws, &prims(p), &[], 800.0, cfg)
        .unwrap()
        .iter()
        .map(|c| (c.rows.len(), c.rows[0].len()))
        .collect()
}

/// `rg_tall_regular`: 19 rules on 0.7 of the page height, a line in each gap, 3 columns.
#[test]
fn tall_regular_rule_group_is_a_table() {
    let ys: Vec<f64> = (0..19).map(|i| 100.0 + 31.0 * i as f64).collect();
    let page = ruled_table(&ys, &[1; 18]);
    assert!(ys[18] - ys[0] > RULE_GROUP_MAX_H_FRAC * 800.0);
    let on = r4_on(|c| c.rg_tall_regular = true);
    assert_eq!(rules_of(&page, &on), vec![(18, 3)]);
    assert!(!TableConfig::default().rg_tall_regular);
    assert!(rules_of(&page, &r4_off()).is_empty());
}

/// A page frame (4 full-width rules) with prose lines between them is not a table with A2 either.
#[test]
fn page_frame_is_still_not_a_table() {
    let mut p = page();
    for y in [40.0, 270.0, 520.0, 760.0] {
        p.paths.push(crate::tables::testkit::hline(30.0, 570.0, y));
    }
    let prose = "the quick brown fox jumps over the lazy dog and runs far away from here";
    for (g, y0) in [40.0, 270.0, 520.0].into_iter().enumerate() {
        for l in 0..(if g == 1 { 5 } else { 8 }) {
            for (k, w) in prose.split(' ').enumerate() {
                let x = 40.0 + 36.0 * k as f64;
                p.words
                    .push(input_word(w, x, y0 + 8.0 + 14.0 * l as f64, 10.0));
            }
        }
    }
    let on = r4_on(|c| c.rg_tall_regular = true);
    // Off: no candidate. On: `detect_rules` does produce the frame candidate (so the flag lets it
    // through), and `select` in `detect_tables` rejects it.
    assert!(rules_of(&p, &r4_off()).is_empty());
    assert_eq!(rules_of(&p, &on), vec![(21, 15)]);
    let tables = crate::tables::detect_tables(&p, &on);
    assert!(tables.tables.is_empty(), "{tables:?}");
}

/// `rg_regular_rows`: 5 intervals holding 2-4 text lines each give 5 rows; the line count alone does
/// not (fewer than `RULE_ROW_SHARE` of the lines have a rule above).
#[test]
fn regular_rules_bound_multiline_rows() {
    let ys = [100.0, 160.0, 220.0, 280.0, 340.0, 400.0];
    let page = ruled_table(&ys, &[2, 3, 4, 2, 3]);
    let on = r4_on(|c| c.rg_regular_rows = true);
    let off = r4_off();
    assert_eq!(rules_of(&page, &on), vec![(5, 3)]);
    assert_eq!(rules_of(&page, &off), vec![(14, 3)]);
    // An interval without a text line is not regular: the old rows.
    let gappy = ruled_table(&ys, &[2, 3, 0, 2, 3]);
    assert_eq!(rules_of(&gappy, &on), rules_of(&gappy, &off));
}

/// A 4-row [`ruled_table`] under a filled header band (words `Hd0..Hd2`) on its first rule.
fn filled_header_table() -> PageInput {
    let mut page = ruled_table(&[100.0, 130.0, 160.0, 190.0, 220.0], &[1, 1, 1, 1]);
    page.paths.push(fill(60.0, 55.0, 420.0, 100.0));
    for c in 0..3 {
        let x = 70.0 + 120.0 * c as f64;
        page.words
            .push(input_word(&format!("Hd{c}"), x, 65.0, 10.0));
    }
    page
}

/// `rg_fill_header`: a filled band on the first rule is the header row.
#[test]
fn filled_header_on_first_rule() {
    let page = filled_header_table();
    let ws = words_of(&page);
    let pr = prims(&page);
    let on = r4_on(|c| c.rg_fill_header = true);
    let got = detect_rules(&ws, &pr, &[], 800.0, &on).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].rows.len(), 5);
    assert_eq!(got[0].rows[0], vec!["Hd0", "Hd1", "Hd2"]);
    let off = detect_rules(&ws, &pr, &[], 800.0, &r4_off()).unwrap();
    assert!(off.iter().all(|c| c.rows[0][0] != "Hd0"));
}

/// Two 4-rule tables of one width with the text `gap` `(word, x)` between them (one line at 215).
fn two_tables(gap: &[(&str, f64)]) -> PageInput {
    let mut ys: Vec<f64> = (0..4).map(|i| 100.0 + 30.0 * i as f64).collect();
    ys.extend((0..4).map(|i| 300.0 + 30.0 * i as f64));
    let mut p = ruled_table(&ys, &[1, 1, 1, 0, 1, 1, 1]);
    for &(t, x) in gap {
        p.words.push(input_word(t, x, 215.0, 10.0));
    }
    p
}

/// `rg_split_multiseg`: a gap line of two segments (a wrapped cell of the second column) next to a
/// wide line does not cut; a wide line alone (a caption) does.
#[test]
fn multisegment_gap_line_is_not_a_caption() {
    let on = r4_on(|c| c.rg_split_multiseg = true);
    let caption = two_tables(&[
        ("TableTwoCaption", 70.0),
        ("textAcross", 166.0),
        ("pageEnd", 232.0),
    ]);
    let mut wrapped = caption.clone();
    wrapped.words.push(input_word("wrap", 190.0, 227.0, 10.0));
    wrapped.words.push(input_word("cell", 330.0, 227.0, 10.0));
    let n = |p: &PageInput, c: &TableConfig| rules_of(p, c).len();
    assert_eq!(n(&caption, &on), 2, "a real caption cuts");
    assert_eq!(n(&caption, &r4_off()), 2);
    assert_eq!(n(&wrapped, &r4_off()), 2);
    assert_eq!(n(&wrapped, &on), 1, "a wrapped cell does not cut");
}

/// With the R4 flags off the output is the pre-change one, pinned on pages that each need one flag
/// (and differ with it on, so the pages do exercise the flags).
#[test]
fn without_flags_nothing_changes() {
    let d = r4_off();
    assert!(!(d.rg_tall_regular || d.rg_regular_rows || d.rg_fill_header || d.rg_split_multiseg));
    let dflt = TableConfig::default();
    assert!(!dflt.rg_tall_regular);
    assert!(dflt.rg_regular_rows && dflt.rg_fill_header && dflt.rg_split_multiseg);
    let ys: Vec<f64> = (0..19).map(|i| 100.0 + 31.0 * i as f64).collect();
    let multi = ruled_table(
        &[100.0, 160.0, 220.0, 280.0, 340.0, 400.0],
        &[2, 3, 4, 2, 3],
    );
    let filled = filled_header_table();
    let mut wrapped = two_tables(&[("TableTwoCaption", 70.0), ("textAcross", 166.0)]);
    wrapped.words.push(input_word("wrap", 190.0, 227.0, 10.0));
    wrapped.words.push(input_word("cell", 330.0, 227.0, 10.0));
    // (page, default config, the flag on)
    let cases = [
        (ruled_table(&ys, &[1; 18]), vec![], vec![(18, 3)]),
        (multi, vec![(14, 3)], vec![(5, 3)]),
        (filled, vec![(4, 3)], vec![(5, 3)]),
        (wrapped, vec![(3, 3), (3, 3)], vec![(7, 3)]),
    ];
    let ons: [fn(&mut TableConfig); 4] = [
        |c| c.rg_tall_regular = true,
        |c| c.rg_regular_rows = true,
        |c| c.rg_fill_header = true,
        |c| c.rg_split_multiseg = true,
    ];
    for ((page, off, on), set) in cases.iter().zip(ons) {
        assert_eq!(&rules_of(page, &d), off);
        assert_eq!(&rules_of(page, &r4_on(set)), on);
    }
}
