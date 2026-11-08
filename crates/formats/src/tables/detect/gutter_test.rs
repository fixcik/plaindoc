//! Two-column prose guards: `gutter_prose`, `page_columns`, `uncertain_page_columns`, `frame_prose`.

use super::gutter::{page_gutters, prose_across_gutter};
use super::helpers_test::*;
use super::{CandidateKind, candidates};
use crate::tables::prims::{Word, prims, words_of};
use crate::tables::stats::median;
use crate::tables::testkit::{hline, input_word, vline};
use crate::tables::{PageInput, TableConfig, detect_tables};

/// Lines of 3-letter words in groups of 4 (84 pt): `left` lines with groups at x 60 and 160, `right`
/// lines with groups at x 340 and 440, on a 640-pt page (gutter at 292 in the middle half).
fn prose_columns(lines: &[(bool, bool)]) -> PageInput {
    let mut p = PageInput {
        width: 640.0,
        ..page()
    };
    for (r, &(left, right)) in lines.iter().enumerate() {
        let top = 100.0 + 12.0 * r as f64;
        let groups = [(left, 60.0), (left, 160.0), (right, 340.0), (right, 440.0)];
        for (_, x) in groups.into_iter().filter(|g| g.0) {
            for k in 0..4 {
                p.words
                    .push(input_word("abc", x + 22.0 * k as f64, top, 10.0));
            }
        }
    }
    p
}

#[test]
fn prose_across_the_gutter_is_rejected_as_two_columns() {
    let page = prose_columns(&[(true, true); 6]);
    let ws = words_of(&page);
    let run = |mode: u8| {
        let cfg = cfg_with(|c| c.gutter_prose = mode);
        let cands = candidates(&page, &ws, &prims(&page), &cfg).unwrap();
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].kind, CandidateKind::Text);
        crate::tables::select::select(cands, &cfg)
    };
    let off = run(0);
    assert!(
        off.rejected
            .iter()
            .all(|r| r.reason != crate::tables::RejectReason::TwoColumnProse)
    );
    let on = run(1);
    assert!(on.tables.is_empty());
    assert_eq!(
        on.rejected[0].reason,
        crate::tables::RejectReason::TwoColumnProse
    );
}

/// A 640-pt page: the `above` lines (0: words of two page columns at x 60 and 340; 1: one line across
/// the page; 2: column words plus a pair at 221/243 sitting 4 pt apart around the gutter 242) in the
/// 8-pt slots before y 178 (all inside the 100-pt band above the table, a 60-pt gap to it), a 5-row table of two 4-word cells (x 60 and 340) from 240; with `pad`
/// 110 more lines of two-column prose below, so that the page gutter survives 3 crossing lines.
fn text_over_cells(above: &[u8], pad: bool) -> PageInput {
    let mut p = PageInput {
        width: 640.0,
        ..page()
    };
    let mut word = |x: f64, top: f64| p.words.push(input_word("abc", x, top, 10.0));
    for (r, &kind) in above.iter().enumerate() {
        let top = 178.0 - 8.0 * (above.len() - r) as f64;
        let mut xs: Vec<f64> = match kind {
            1 => (0..21).map(|k| 60.0 + 22.0 * k as f64).collect(),
            _ => (0..4)
                .flat_map(|k| [60.0 + 22.0 * k as f64, 340.0 + 22.0 * k as f64])
                .collect(),
        };
        if kind == 2 {
            xs.extend([221.0, 243.0]);
        }
        xs.into_iter().for_each(|x| word(x, top));
    }
    let blocks = [(240.0, 5), (400.0, if pad { 110 } else { 0 })];
    for (y0, n) in blocks {
        for r in 0..n {
            let top = y0 + 12.0 * r as f64;
            for k in 0..4 {
                word(60.0 + 22.0 * k as f64, top);
                word(340.0 + 22.0 * k as f64, top);
            }
        }
    }
    p
}

/// `page_columns`: the bottom of two prose columns (their last lines happen to align) is not a table;
/// with a real page gutter and the text above inside the band, a table under fewer than 3 column
/// lines, or under column lines plus lines crossing the gutter, stays one; off, all stay as before.
#[test]
fn page_columns_reject_aligned_column_bottoms() {
    let reasons = |p: &PageInput, on: bool| {
        let cfg = cfg_with(|c| {
            c.gutter_prose = 0;
            c.page_columns = on;
            c.uncertain_page_columns = false;
            c.wide_gap_columns = false;
        });
        let r = detect_tables(p, &cfg);
        // Only the block at y 240 (the lines above are prose in all pages).
        let mid = |b: &[f64; 4]| (200.0..300.0).contains(&b[1]);
        let twocol = r
            .rejected
            .iter()
            .any(|j| j.reason == crate::tables::RejectReason::TwoColumnProse && mid(&j.bbox));
        (r.tables.iter().filter(|t| mid(&t.bbox)).count(), twocol)
    };
    let check = |above: &[u8], pad: bool, rejected: bool| {
        let p = text_over_cells(above, pad);
        assert!(page_gutters(&words_of(&p), 640.0, 10.0).contains(&242.0));
        assert_eq!(reasons(&p, false), (1, false), "{above:?} off");
        let on = if rejected { (0, true) } else { (1, false) };
        assert_eq!(reasons(&p, true), on, "{above:?} on");
    };
    check(&[0; 5], false, true);
    check(&[0; 2], false, false);
    // Crossing lines are not column lines: 3 + 2 would reach 3 per side if they were counted.
    check(&[1, 1, 1, 0, 0], true, false);
    check(&[2, 2, 2, 0, 0], true, false);
}

#[test]
fn shifted_prose_columns_are_caught_by_mode_3_only() {
    // 4 lines on the left, 3 on the right, never on the same line: the median right count is 0.
    let page = prose_columns(&[
        (true, false),
        (false, true),
        (true, false),
        (false, true),
        (true, false),
        (false, true),
        (true, false),
    ]);
    let ws = words_of(&page);
    let all: Vec<&Word> = ws.iter().collect();
    let gut = page_gutters(&ws, page.width, 10.0);
    // Gaps 244-340 and 424-440 lie in the middle half of the page (160-480); 144-160 does not.
    assert_eq!(gut, vec![292.0, 432.0]);
    let bbox = [60.0, 100.0, 524.0, 182.0];
    assert!(!prose_across_gutter(&all, bbox, &gut, 1));
    assert!(prose_across_gutter(&all, bbox, &gut, 3));
    // A gutter outside the middle half of the candidate does not count.
    assert!(!prose_across_gutter(
        &all,
        [-1000.0, 100.0, 524.0, 182.0],
        &gut,
        3
    ));
    // Lines crossing the gutter (20% or more) are not shifted columns.
    let both = prose_columns(&[
        (true, true),
        (true, false),
        (false, true),
        (false, true),
        (false, true),
    ]);
    let wb = words_of(&both);
    let ab: Vec<&Word> = wb.iter().collect();
    assert!(!prose_across_gutter(&ab, bbox, &gut, 3));
}

/// T15 hostile input: the page gutters and the guard over a whole page of `MAX_WORDS` words of
/// two-column prose (all words one candidate).
#[test]
fn gutter_scan_is_fast_on_20k_words() {
    let p = crate::tables::tests::two_column_prose();
    let ws = words_of(&p);
    let all: Vec<&Word> = ws.iter().collect();
    let t = std::time::Instant::now();
    let gut = page_gutters(&ws, p.width, 10.0);
    let hit = prose_across_gutter(&all, [60.0, 0.0, 540.0, 30_000.0], &gut, 3);
    let ms = t.elapsed().as_secs_f64() * 1e3;
    eprintln!("gutter scan 20000 words: {ms:.2} ms, gutters {gut:?}, prose {hit}");
    assert!(hit && gut.len() == 1);
    // Measured ~3 ms; the bound leaves 10x for a loaded CI box.
    if !cfg!(debug_assertions) {
        assert!(ms <= 50.0, "gutter scan took {ms} ms");
    }
}

/// Greedy lines of generated prose in `[x0, x1]` from `top` at a 10-pt pitch down to `bottom`; every
/// 12th line is a numbered section heading followed by an extra half line (column baselines drift).
fn prose_column(p: &mut PageInput, x0: f64, x1: f64, top: f64, bottom: f64, seed: usize) {
    let (mut y, mut k, mut line, mut sec) = (top, seed, 0usize, 1usize);
    while y + 8.0 < bottom {
        if line % 12 == 0 {
            let mut x = x0;
            for t in [format!("{sec}."), "РАЗДЕЛ".into(), "ПРИМЕРА".into()] {
                p.words.push(input_word(&t, x, y, 8.0));
                x += 6.0 * t.chars().count() as f64 + 4.0;
            }
            sec += 1;
            y += 15.0;
        } else {
            prose_line(p, x0, x1, y, &mut k);
            y += 10.0;
        }
        line += 1;
    }
}

/// A4 page of two columns of prose inside a page frame: top and bottom rules per half, verticals at
/// the left edge, the middle and the right edge, no rule between rows.
fn framed_prose() -> PageInput {
    let mut p = PageInput {
        width: 595.0,
        height: 842.0,
        ..Default::default()
    };
    let (l, m, r, top, bot) = (20.0, 297.5, 575.0, 60.0, 830.0);
    for (a, b) in [(l, m), (m, r)] {
        p.paths.push(hline(a, b, top));
        p.paths.push(hline(a, b, bot));
    }
    p.paths.extend([l, m, r].map(|x| vline(x, top, bot)));
    prose_column(&mut p, 26.0, 291.0, 66.0, 824.0, 1);
    prose_column(&mut p, 303.0, 569.0, 71.0, 824.0, 5);
    p
}

#[test]
fn two_column_prose_in_a_page_frame_is_not_a_table() {
    let page = framed_prose();
    let ws = words_of(&page);
    let cands = candidates(&page, &ws, &prims(&page), &TableConfig::default()).unwrap();
    // The frame gives a lined candidate whose rows come from the text lines.
    assert!(cands.iter().any(|c| c.kind == CandidateKind::Lined));
    let r = detect_tables(&page, &TableConfig::default());
    assert!(
        r.tables.is_empty(),
        "{:?}",
        r.tables
            .iter()
            .map(|t| (t.kind, t.rows.len()))
            .collect::<Vec<_>>()
    );
    assert!(
        r.rejected
            .iter()
            .any(|j| j.reason == crate::tables::RejectReason::TwoColumnProse)
    );
    // The reference (Python parity) keeps the prototype's behaviour.
    assert_eq!(
        detect_tables(&page, &TableConfig::reference()).tables.len(),
        1
    );
}

/// Two-column lined table with its column rule at mid-page (frame 20 / 297.5 / 575 from top 100), the
/// only text of the page, so the column split is a page gutter in the middle half of the table. Each
/// of the 6 rows has a one-word cell on the left (an amount with `numeric`, otherwise a term) and a
/// 6-line text on the right (a glossary). `row_rules` draws a full-width rule between rows.
fn mid_rule_table(row_rules: bool, numeric: bool) -> PageInput {
    let mut p = PageInput {
        width: 595.0,
        height: 842.0,
        ..Default::default()
    };
    let (l, m, r, top) = (20.0, 297.5, 575.0, 100.0);
    let (mut y, mut k) = (106.0, 7usize);
    let mut ys = vec![top];
    for i in 0..6 {
        let short = if numeric {
            format!("{}50,00", 12 + 7 * i)
        } else {
            PROSE[16 + i].to_string()
        };
        p.words.push(input_word(&short, 26.0, y, 8.0));
        for _ in 0..6 {
            prose_line(&mut p, 303.0, 569.0, y, &mut k);
            y += 10.0;
        }
        y += 8.0;
        ys.push(y - 4.0);
    }
    let bot = *ys.last().unwrap();
    let hs: Vec<f64> = if row_rules { ys } else { vec![top, bot] };
    p.paths.extend(hs.iter().map(|&y| hline(l, r, y)));
    p.paths.extend([l, m, r].map(|x| vline(x, top, bot)));
    p
}

/// The new scope of the gutter guard on tables whose column rule sits on a page gutter: rows split by
/// rulings or a numeric column keep the table; a frame-only text glossary is rejected (the documented
/// `frame_prose` trade-off: mode 3, long definitions put words on both sides in < 20% of the lines).
#[test]
fn mid_page_column_rule_tables_under_the_frame_guard() {
    let gutters = |p: &PageInput| {
        let ws = words_of(p);
        let fs = median(&ws.iter().map(|w| w.fs).collect::<Vec<_>>());
        page_gutters(&ws, p.width, fs)
    };
    let run = |p: &PageInput| detect_tables(p, &TableConfig::default());
    // (a) Glossary with row rules: rows come from the rulings, the guard does not apply.
    let ruled = mid_rule_table(true, false);
    assert!(!gutters(&ruled).is_empty());
    let a = run(&ruled);
    assert_eq!(a.tables.len(), 1, "{:?}", a.rejected);
    assert_eq!(a.tables[0].kind, CandidateKind::Lined);
    // (b) The same frame-only layout with a numeric first column: `numcols > 0` keeps it.
    let num = mid_rule_table(false, true);
    assert!(!gutters(&num).is_empty());
    let b = run(&num);
    assert_eq!(b.tables.len(), 1, "{:?}", b.rejected);
    assert!(b.tables[0].features.numcols > 0.0);
    // (c) Frame-only text glossary: rejected as two-column prose (trade-off); kept without the guard.
    let gloss = mid_rule_table(false, false);
    assert!(!gutters(&gloss).is_empty());
    let c = run(&gloss);
    assert!(c.tables.is_empty());
    assert!(
        c.rejected
            .iter()
            .any(|j| j.reason == crate::tables::RejectReason::TwoColumnProse)
    );
    let off = detect_tables(
        &gloss,
        &TableConfig {
            frame_prose: false,
            ..TableConfig::default()
        },
    );
    assert_eq!(off.tables.len(), 1, "{:?}", off.rejected);
}

/// "Code | description" table: short codes at x 30, descriptions of `lines` lines at x 100 (column
/// rule at 90), 4 rows; `row_rules` draws a rule under each row, otherwise only the frame.
fn code_table(row_rules: bool) -> PageInput {
    let mut p = PageInput {
        width: 595.0,
        height: 842.0,
        ..Default::default()
    };
    let (l, m, r) = (22.0, 90.0, 570.0);
    let (top, mut y, mut k) = (100.0, 104.0, 3usize);
    let mut ys = vec![top];
    for (i, lines) in [2usize, 4, 3, 5].into_iter().enumerate() {
        p.words
            .push(input_word(&format!("К-{}", i + 1), 30.0, y, 8.0));
        for _ in 0..lines {
            prose_line(&mut p, 100.0, r - 6.0, y, &mut k);
            y += 10.0;
        }
        y += 8.0;
        ys.push(y - 4.0);
    }
    let bot = *ys.last().unwrap();
    let hs: Vec<f64> = if row_rules { ys } else { vec![top, bot] };
    p.paths.extend(hs.iter().map(|&y| hline(l, r, y)));
    p.paths.extend([l, m, r].map(|x| vline(x, top, bot)));
    // A prose line above the table.
    let mut x = 22.0;
    for t in PROSE.iter().take(8) {
        p.words.push(input_word(t, x, 70.0, 8.0));
        x += 6.0 * t.chars().count() as f64 + 4.0;
    }
    p
}

#[test]
fn code_description_tables_stay_tables() {
    for row_rules in [true, false] {
        let r = detect_tables(&code_table(row_rules), &TableConfig::default());
        assert_eq!(r.tables.len(), 1, "row rules {row_rules}: {:?}", r.rejected);
        assert_eq!(r.tables[0].kind, CandidateKind::Lined);
        assert_eq!(r.tables[0].features.ncols, 2.0);
    }
}

/// `n` five-letter words (30 pt wide, 4 pt apart) from `x` at `top`, 9-pt type.
fn run_of(p: &mut PageInput, x: f64, top: f64, n: usize) {
    for k in 0..n {
        p.words
            .push(input_word("текст", x + 34.0 * k as f64, top, 9.0));
    }
}

/// A résumé page: a main column at x 50 (lines of 10 words, to x 386) and a sidebar at x 420 (lines of
/// 3 words) down the page, then their last lines side by side, `(main words, sidebar words)` per line.
fn sidebar_tail(tail: &[(usize, usize)]) -> PageInput {
    let mut p = PageInput {
        width: 595.0,
        height: 842.0,
        ..Default::default()
    };
    let mut y = 80.0;
    while y < 640.0 {
        run_of(&mut p, 50.0, y, 10);
        run_of(&mut p, 420.0, y + 3.0, 3);
        y += 12.0;
    }
    for &(l, r) in tail {
        y += 12.0;
        run_of(&mut p, 50.0, y, l);
        run_of(&mut p, 420.0, y, r);
    }
    p
}

/// `uncertain_page_columns`: the last lines of a résumé's main column and sidebar side by side (two
/// columns split on the page gutter, both page columns continue above) are not a table when the model
/// doubts them (p < 0.30); a confident block in the same layout stays (`page_columns` would drop it).
#[test]
fn doubtful_sidebar_tail_is_not_a_table() {
    let doubtful = sidebar_tail(&[(1, 3), (3, 1), (10, 2), (5, 3), (0, 2)]);
    let off = detect_tables(&doubtful, &cfg_with(|c| c.uncertain_page_columns = false));
    assert_eq!(off.tables.len(), 1);
    assert_eq!(off.tables[0].features.ncols, 2.0);
    assert!(off.tables[0].probability < 0.30);
    let on = detect_tables(&doubtful, &TableConfig::default());
    assert!(on.tables.is_empty());
    assert!(
        on.rejected
            .iter()
            .any(|j| j.reason == crate::tables::RejectReason::TwoColumnProse)
    );
    let confident = sidebar_tail(&[(3, 3), (10, 3), (3, 3), (10, 3), (3, 3), (10, 3)]);
    let kept = detect_tables(&confident, &TableConfig::default());
    assert_eq!(kept.tables.len(), 1);
    assert!(kept.tables[0].probability >= 0.30);
    let strict = detect_tables(&confident, &cfg_with(|c| c.page_columns = true));
    assert!(strict.tables.is_empty());
}

/// `wide_gap_columns`: short last lines of a résumé's main column beside the sidebar (a gap of about 34
/// font sizes holding the page gutter, both page columns above) are not a table at any probability; the
/// same rows without the page columns above stay a table.
#[test]
fn wide_gap_sidebar_tail_is_not_a_table() {
    let tail = [(2, 3); 6];
    let off = detect_tables(
        &sidebar_tail(&tail),
        &cfg_with(|c| c.wide_gap_columns = false),
    );
    assert_eq!(off.tables.len(), 1);
    assert!(off.tables[0].probability >= 0.30);
    let on = detect_tables(&sidebar_tail(&tail), &TableConfig::default());
    assert!(on.tables.is_empty());
    assert!(
        on.rejected
            .iter()
            .any(|j| j.reason == crate::tables::RejectReason::TwoColumnProse)
    );
    let mut alone = PageInput {
        width: 595.0,
        height: 842.0,
        ..Default::default()
    };
    for r in 0..6 {
        run_of(&mut alone, 50.0, 300.0 + 12.0 * r as f64, 2);
        run_of(&mut alone, 420.0, 300.0 + 12.0 * r as f64, 3);
    }
    assert_eq!(
        detect_tables(&alone, &TableConfig::default()).tables.len(),
        1
    );
}

/// Words from `x` at `top`, 4 pt apart (6 pt per char).
fn words_at(p: &mut PageInput, x: f64, top: f64, ws: &[&str], fs: f64) {
    let mut x = x;
    for w in ws {
        p.words.push(input_word(w, x, top, fs));
        x += 6.0 * w.chars().count() as f64 + 4.0;
    }
}

/// A form of `n` stacked key-value sections (heading, 3 rows: key at x 50, value at x 300, then `notes`
/// left-only note lines in smaller type): the key/value split is the page's only vertical gap.
fn stacked_form(n: usize, notes: usize) -> PageInput {
    let rows: [(&[&str], &[&str]); 3] = [
        (&["Заявитель"], &["Иванов", "Пётр", "Сергеевич"]),
        (&["Представитель"], &["не", "назначен"]),
        (&["Способ", "получения"], &["лично", "в", "отделении"]),
    ];
    let mut p = PageInput {
        width: 595.0,
        height: 842.0,
        ..Default::default()
    };
    let mut y = 60.0;
    for s in 0..n {
        words_at(&mut p, 50.0, y, &[&format!("{}.", s + 1), "Сведения"], 10.0);
        y += 20.0;
        for (k, v) in rows {
            words_at(&mut p, 50.0, y, k, 10.0);
            words_at(&mut p, 300.0, y, v, 10.0);
            y += 14.0;
        }
        for _ in 0..notes {
            y += 4.0;
            let note = ["заполняется", "при", "наличии", "сведений"];
            words_at(&mut p, 50.0, y, &note, 8.0);
            y += 10.0;
        }
        y += 22.0;
    }
    p
}

/// Review regression: the sections above a section of a form are keys and values, not two page columns
/// of prose, so neither `uncertain_page_columns` (the blocks are below p 0.30) nor `page_columns` drops the
/// sections after the first (6 sections with one note line each: p 0.26, 5 of 6 were lost).
#[test]
fn stacked_form_sections_stay_tables() {
    for (n, notes) in [(6, 1), (5, 2), (4, 3)] {
        let p = stacked_form(n, notes);
        let off = detect_tables(&p, &cfg_with(|c| c.uncertain_page_columns = false));
        assert_eq!(off.tables.len(), n);
        // The first form is in the doubtful band, so the default config would apply the guard to it.
        assert!(notes > 1 || off.tables.iter().any(|t| t.probability < 0.30));
        assert!(!page_gutters(&words_of(&p), p.width, 10.0).is_empty());
        for cfg in [TableConfig::default(), cfg_with(|c| c.page_columns = true)] {
            let r = detect_tables(&p, &cfg);
            assert_eq!(r.tables.len(), n, "{n}x{notes}: {:?}", r.rejected);
        }
    }
}
