//! `box_flow`: tag chips in rows of separate boxes are not a table; spaced cell boxes in columns are.

use super::flow::box_flow;
use super::helpers_test::cfg_with;
use crate::tables::prims::{PRect, Word};
use crate::tables::testkit::{input_word, word};
use crate::tables::{InputPath, PageInput, Rect, RejectReason, TableConfig, detect_tables};

/// Chips of the synthetic résumé r20 (stroked boxes 15.8 pt high, 4 pt apart), `(word, x0, width)` per
/// row; rows 19.5 pt apart from y 202.5.
const CHIPS: [[(&str, f64, f64); 3]; 3] = [
    [
        ("Rust", 419.2, 34.5),
        ("Python", 457.5, 43.5),
        ("SQL", 504.7, 33.8),
    ],
    [
        ("Linux", 419.2, 37.5),
        ("Docker", 460.5, 44.3),
        ("Git", 508.5, 27.0),
    ],
    [
        ("REST", 419.2, 39.8),
        ("Kafka", 462.8, 39.0),
        ("CI/CD", 505.5, 39.8),
    ],
];

/// Boxes `[x0, top, x1, bottom]` of the rows and their words (centred in the box, 9-pt type).
fn layout(rows: &[[(&str, f64, f64); 3]]) -> (Vec<[f64; 4]>, Vec<Word>) {
    let (mut bs, mut ws) = (Vec::new(), Vec::new());
    for (r, row) in rows.iter().enumerate() {
        let top = 202.5 + 19.5 * r as f64;
        for &(t, x, w) in row {
            bs.push([x, top, x + w, top + 15.8]);
            let tw = 6.0 * t.chars().count() as f64;
            ws.push(word(t, x + (w - tw) / 2.0, top + 3.4, 9.0));
        }
    }
    (bs, ws)
}

fn prect(b: [f64; 4], stroke: bool) -> PRect {
    PRect {
        x0: b[0],
        t: b[1],
        x1: b[2],
        b: b[3],
        fill: !stroke,
        stroke,
        background: false,
    }
}

/// Each box twice, as the PDF draws a chip: a fill and a stroke 0.4 pt around it.
fn chip_rects(bs: &[[f64; 4]]) -> Vec<PRect> {
    bs.iter()
        .flat_map(|&b| {
            let fill = [b[0] + 0.4, b[1] + 0.4, b[2] - 0.4, b[3] - 0.4];
            [prect(fill, false), prect(b, true)]
        })
        .collect()
}

fn bbox(bs: &[[f64; 4]]) -> [f64; 4] {
    bs.iter()
        .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        })
}

fn flow(rows: &[[(&str, f64, f64); 3]]) -> bool {
    let (bs, ws) = layout(rows);
    let refs: Vec<&Word> = ws.iter().collect();
    box_flow(&refs, &chip_rects(&bs), bbox(&bs))
}

#[test]
fn chips_wrapped_by_width_are_a_box_flow() {
    assert!(flow(&CHIPS));
}

#[test]
fn spaced_cells_in_columns_are_not_a_box_flow() {
    // The same boxes with the widths of the first row: every column edge lines up.
    let aligned: Vec<[(&str, f64, f64); 3]> = CHIPS
        .iter()
        .map(|row| [0, 1, 2].map(|k| (row[k].0, CHIPS[0][k].1, CHIPS[0][k].2)))
        .collect();
    assert!(!flow(&aligned));
    // Touching boxes are cells of a grid, not chips.
    let touching: Vec<[(&str, f64, f64); 3]> = CHIPS
        .iter()
        .map(|row| {
            let x1 = row[0].1 + row[0].2;
            let x2 = row[1].1 + row[1].2;
            [
                row[0],
                (row[1].0, x1, row[1].2 + row[1].1 - x1),
                (row[2].0, x2, row[2].2 + row[2].1 - x2),
            ]
        })
        .collect();
    assert!(!flow(&touching));
    // One row of chips is not enough.
    assert!(!flow(&CHIPS[..1]));
}

#[test]
fn a_word_outside_the_boxes_is_not_a_box_flow() {
    let (bs, mut ws) = layout(&CHIPS);
    ws.push(word("note", 470.0, 262.0, 9.0));
    let refs: Vec<&Word> = ws.iter().collect();
    let mut b = bbox(&bs);
    b[3] = 275.0;
    assert!(!box_flow(&refs, &chip_rects(&bs), b));
}

/// On a résumé page (a main column of 50 lines of 8 words beside it) the chip grid is a lined candidate
/// the model doubts: `box_flow` rejects it.
#[test]
fn chip_grid_on_a_page_is_rejected() {
    let (bs, _) = layout(&CHIPS);
    let mut p = PageInput {
        width: 595.0,
        height: 842.0,
        ..Default::default()
    };
    for row in CHIPS.iter().enumerate() {
        for &(t, x, w) in row.1 {
            let top = 202.5 + 19.5 * row.0 as f64;
            let tw = 6.0 * t.chars().count() as f64;
            p.words
                .push(input_word(t, x + (w - tw) / 2.0, top + 3.4, 9.0));
        }
    }
    for l in 0..50 {
        for k in 0..8 {
            let top = 60.0 + 14.0 * l as f64;
            p.words
                .push(input_word("text", 40.0 + 40.0 * k as f64, top, 10.0));
        }
    }
    for r in chip_rects(&bs) {
        p.paths.push(InputPath {
            rect: Rect::new(r.x0, r.t, r.x1 - r.x0, r.b - r.t),
            stroke: r.stroke,
            fill: r.fill,
        });
    }
    let off = detect_tables(&p, &cfg_with(|c| c.box_flow = false));
    assert_eq!(off.tables.len(), 1, "{off:?}");
    assert!(
        off.tables[0].probability < 0.30,
        "{}",
        off.tables[0].probability
    );
    let on = detect_tables(&p, &TableConfig::default());
    assert!(on.tables.is_empty());
    assert_eq!(on.rejected[0].reason, RejectReason::BoxFlow);
}
