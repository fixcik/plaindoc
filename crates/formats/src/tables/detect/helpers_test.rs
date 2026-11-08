//! Shared fixtures of the detector tests.

use super::fills::detect_fill_groups;
use super::{CandidateKind, Raw, candidates};
use crate::tables::prims::{Word, prims, words_of};
use crate::tables::testkit::{hline, input_word};
use crate::tables::{PageInput, TableConfig};

/// Booktabs: top, header and bottom rules, no verticals, 3 columns × (header + 3 rows).
pub(super) fn booktabs() -> PageInput {
    let mut p = PageInput {
        width: 600.0,
        height: 800.0,
        ..Default::default()
    };
    for y in [100.0, 118.0, 172.0] {
        p.paths.push(hline(60.0, 400.0, y));
    }
    let rows = [
        ["Name", "Qty", "Price"],
        ["Apples", "12", "3.50"],
        ["Pears", "7", "4.10"],
        ["Plums", "30", "1.25"],
    ];
    for (r, row) in rows.iter().enumerate() {
        let top = if r == 0 {
            103.0
        } else {
            108.0 + 15.0 * r as f64
        };
        for (c, t) in row.iter().enumerate() {
            p.words
                .push(input_word(t, 70.0 + 120.0 * c as f64, top, 10.0));
        }
    }
    p
}

pub(super) fn page() -> PageInput {
    PageInput {
        width: 600.0,
        height: 800.0,
        ..Default::default()
    }
}

pub(super) fn fill(x0: f64, top: f64, x1: f64, bot: f64) -> crate::tables::InputPath {
    crate::tables::InputPath {
        rect: crate::tables::Rect::new(x0, top, x1 - x0, bot - top),
        stroke: false,
        fill: true,
    }
}

/// Zebra: rows 0, 2, 4 of a 5 × 3 table (rows 16 pt from y 100, columns 120 pt from x 60) filled per
/// cell; word `rc` in each cell.
pub(super) fn zebra(filled_rows: &[usize]) -> PageInput {
    let mut p = page();
    for &r in filled_rows {
        let top = 100.0 + 16.0 * r as f64;
        for c in 0..3 {
            let x = 60.0 + 120.0 * c as f64;
            p.paths.push(fill(x, top, x + 120.0, top + 16.0));
        }
    }
    for r in 0..5 {
        for c in 0..3 {
            p.words.push(input_word(
                &format!("r{r}c{c}"),
                70.0 + 120.0 * c as f64,
                103.0 + 16.0 * r as f64,
                10.0,
            ));
        }
    }
    p
}

/// The prototype's text path (15 columns, no row rule) with `text_join` set.
pub(super) fn text_cfg(join: bool) -> TableConfig {
    TableConfig {
        text_join: join,
        ..TableConfig::reference()
    }
}

pub(super) fn cfg_with(f: impl FnOnce(&mut TableConfig)) -> TableConfig {
    let mut c = TableConfig::default();
    f(&mut c);
    c
}

/// The default with the four R4 rule-group flags off (the baseline of their on/off tests).
pub(super) fn r4_off() -> TableConfig {
    cfg_with(|c| {
        c.rg_tall_regular = false;
        c.rg_regular_rows = false;
        c.rg_fill_header = false;
        c.rg_split_multiseg = false;
    })
}

/// [`r4_off`] with one flag turned on by `f`.
pub(super) fn r4_on(f: impl FnOnce(&mut TableConfig)) -> TableConfig {
    let mut c = r4_off();
    f(&mut c);
    c
}

/// Rules at `ys` from 60 to 400 and rows of words `(text, x)` at the given tops (size 10).
pub(super) fn ruled(ys: &[f64], rows: &[(f64, &[(&str, f64)])]) -> PageInput {
    let mut p = page();
    p.paths.extend(ys.iter().map(|&y| hline(60.0, 400.0, y)));
    for &(top, row) in rows {
        for &(t, x) in row {
            p.words.push(input_word(t, x, top, 10.0));
        }
    }
    p
}

/// hlines_all: a rule under every row, none above the header; `above` is the line over the first rule.
pub(super) fn hlines_all(above: &[(&str, f64)]) -> PageInput {
    ruled(
        &[118.0, 133.0, 148.0, 163.0],
        &[
            (105.0, above),
            (120.5, &[("Apples", 70.0), ("12", 190.0), ("3.50", 310.0)]),
            (135.5, &[("Pears", 70.0), ("7", 190.0), ("4.10", 310.0)]),
            (150.5, &[("Plums", 70.0), ("30", 190.0), ("1.25", 310.0)]),
        ],
    )
}

pub(super) fn text_rows(page: &PageInput, cfg: &TableConfig) -> Vec<Vec<Vec<String>>> {
    let ws = words_of(page);
    candidates(page, &ws, &prims(page), cfg)
        .unwrap()
        .into_iter()
        .filter(|c| c.kind == CandidateKind::Text)
        .map(|c| c.rows)
        .collect()
}

pub(super) fn stack(page: &PageInput, cfg: &TableConfig) -> Raw<'static> {
    let ws: &'static [Word] = Box::leak(words_of(page).into_boxed_slice());
    let mut fg = detect_fill_groups(ws, &prims(page), &[], 600.0, 800.0, cfg).unwrap();
    assert_eq!(fg.len(), 1);
    fg.remove(0)
}

/// Neutral Russian words for the frame tests (generated, not from any real document).
pub(super) const PROSE: [&str; 24] = [
    "дом",
    "река",
    "город",
    "время",
    "дерево",
    "окно",
    "утро",
    "путь",
    "свет",
    "слово",
    "книга",
    "в",
    "на",
    "и",
    "по",
    "при",
    "поле",
    "небо",
    "работа",
    "вопрос",
    "старый",
    "новый",
    "большой",
    "простой",
];

/// One greedy line of generated words in `[x0, x1]` at `y`; `k` is the word sequence state.
pub(super) fn prose_line(p: &mut PageInput, x0: f64, x1: f64, y: f64, k: &mut usize) {
    let mut x = x0;
    loop {
        let t = PROSE[*k % PROSE.len()];
        let w = 6.0 * t.chars().count() as f64;
        if x + w > x1 {
            break;
        }
        *k = (*k * 7 + 3) % 1009;
        p.words.push(input_word(t, x, y, 8.0));
        x += w + 4.0;
    }
}
