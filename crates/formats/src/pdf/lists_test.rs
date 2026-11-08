use super::*;
use pdf_oxide::geometry::Rect;

fn line(text: &str, x: f32, y: f32) -> StyledLine {
    StyledLine {
        text: text.into(),
        size: 10.0,
        bold: false,
        font: String::new(),
        color: (0, 0, 0),
        bbox: Rect::new(x, y, 200.0, 10.0),
        gap: 0.0,
        words: None,
    }
}

/// One piece per line (as if every line were cut).
fn run(rows: &[(&str, f32)]) -> Vec<Block> {
    let lines: Vec<StyledLine> = rows
        .iter()
        .enumerate()
        .map(|(i, (t, x))| line(t, *x, 700.0 - 14.0 * i as f32))
        .collect();
    let pieces = lines
        .iter()
        .enumerate()
        .map(|(i, l)| Piece {
            block: Block::paragraph(
                l.text.clone(),
                Some(BBox::new(l.bbox.x, l.bbox.y, 200.0, 10.0)),
            ),
            lines: Some(i..i + 1),
        })
        .collect();
    group(pieces, &lines, &[])
}

fn texts(items: &[ListItem]) -> Vec<String> {
    items
        .iter()
        .map(|i| match i.blocks.first() {
            Some(Block::Paragraph { text, .. }) => text.clone(),
            _ => String::new(),
        })
        .collect()
}

fn list(b: &Block) -> (bool, &[ListItem]) {
    match b {
        Block::List { ordered, items, .. } => (*ordered, items),
        other => panic!("not a list: {other:?}"),
    }
}

#[test]
fn markers_are_recognised() {
    fn k(t: &str) -> Option<(Kind, char, &str)> {
        marker(t).map(|m| (m.kind, m.sep, &t[m.len..]))
    }
    assert_eq!(k("• item"), Some((Kind::Symbol, ' ', "item")));
    assert_eq!(k("•item"), Some((Kind::Symbol, ' ', "item")));
    assert_eq!(k("▪ item"), Some((Kind::Symbol, ' ', "item")));
    assert_eq!(k("◦ item"), Some((Kind::Symbol, ' ', "item")));
    assert_eq!(k("– item"), Some((Kind::Dash, ' ', "item")));
    assert_eq!(k("— item"), Some((Kind::Dash, ' ', "item")));
    assert_eq!(k("- item"), Some((Kind::Dash, ' ', "item")));
    assert_eq!(k("* item"), Some((Kind::Dash, ' ', "item")));
    assert_eq!(k("· item"), Some((Kind::Dash, ' ', "item")));
    assert_eq!(k("1. item"), Some((Kind::Number(1), '.', "item")));
    assert_eq!(k("12) item"), Some((Kind::Number(12), ')', "item")));
    assert_eq!(k("a) item"), Some((Kind::Letter('a'), ')', "item")));
    assert_eq!(k("б. пункт"), Some((Kind::Letter('б'), '.', "пункт")));
    for no in [
        "-item",
        "—",
        "• ",
        "••",
        "1.5 m",
        "2022 — н. в.",
        "02.2020 — 2022",
        "1.",
        "Ab) x",
        "A. Name",
        "1234. x",
        "word - x",
    ] {
        assert_eq!(marker(no), None, "{no}");
    }
}

#[test]
fn bullets_make_a_tight_unordered_list() {
    let b = run(&[
        ("Intro text:", 72.0),
        ("• first", 72.0),
        ("• second", 72.0),
        ("After.", 72.0),
    ]);
    assert_eq!(b.len(), 3);
    let (ordered, items) = list(&b[1]);
    assert!(!ordered);
    assert_eq!(texts(items), ["first", "second"]);
    assert!(matches!(&b[2], Block::Paragraph { text, .. } if text == "After."));
}

#[test]
fn one_symbol_bullet_is_a_list_one_dash_is_not() {
    assert!(matches!(run(&[("• only", 72.0)])[0], Block::List { .. }));
    let b = run(&[("Go · Rust ·", 72.0), ("· Python", 72.0)]);
    assert!(b.iter().all(|b| matches!(b, Block::Paragraph { .. })));
    let b = run(&[("Text", 72.0), ("— это продолжение", 72.0)]);
    assert!(b.iter().all(|b| matches!(b, Block::Paragraph { .. })));
    let b = run(&[("- one", 72.0), ("- two", 72.0)]);
    assert_eq!(texts(list(&b[0]).1), ["one", "two"]);
}

#[test]
fn numbers_from_one_are_ordered_and_stripped() {
    let b = run(&[("1. alpha", 72.0), ("2. beta", 72.0), ("3. gamma", 72.0)]);
    let (ordered, items) = list(&b[0]);
    assert!(ordered);
    assert_eq!(texts(items), ["alpha", "beta", "gamma"]);
}

#[test]
fn numbers_not_from_one_and_letters_keep_their_markers() {
    let b = run(&[("4. alpha", 72.0), ("5. beta", 72.0)]);
    let (ordered, items) = list(&b[0]);
    assert!(!ordered);
    assert_eq!(texts(items), ["4. alpha", "5. beta"]);
    let b = run(&[("а) один", 72.0), ("б) два", 72.0)]);
    assert_eq!(texts(list(&b[0]).1), ["а) один", "б) два"]);
}

#[test]
fn lone_or_broken_numbering_stays_paragraphs() {
    for rows in [
        vec![("1. Введение", 72.0)],
        vec![("1. one", 72.0), ("4. four", 72.0)],
        vec![("1. one", 72.0), ("3. three", 72.0), ("5. five", 72.0)],
        vec![("1. one", 72.0), ("2) two", 72.0)],
        vec![("a) one", 72.0), ("d) four", 72.0)],
    ] {
        let b = run(&rows);
        assert!(
            b.iter().all(|b| matches!(b, Block::Paragraph { .. })),
            "{rows:?}"
        );
        assert_eq!(b.len(), rows.len());
    }
}

#[test]
fn nesting_by_marker_x_up_to_three_levels() {
    let b = run(&[
        ("• top", 72.0),
        ("◦ child", 90.0),
        ("▪ grandchild", 108.0),
        ("▪ too deep", 126.0),
        ("◦ child two", 90.0),
        ("• top two", 72.0),
    ]);
    assert_eq!(b.len(), 1);
    let (_, top) = list(&b[0]);
    assert_eq!(texts(top), ["top", "top two"]);
    let (_, kids) = list(&top[0].blocks[1]);
    assert_eq!(texts(kids), ["child", "child two"]);
    let (_, grand) = list(&kids[0].blocks[1]);
    assert_eq!(texts(grand), ["grandchild", "too deep"]);
}

#[test]
fn hanging_continuation_joins_the_item() {
    let b = run(&[
        ("• first item", 72.0),
        ("its second paragraph", 82.0),
        ("• second", 72.0),
        ("Body again", 72.0),
    ]);
    assert_eq!(b.len(), 2);
    let (_, items) = list(&b[0]);
    assert_eq!(items.len(), 2);
    assert!(
        matches!(&items[0].blocks[1], Block::Paragraph { text, .. } if text == "its second paragraph")
    );
}

#[test]
fn heading_ends_the_list() {
    let lines = vec![line("• a", 72.0, 700.0), line("• b", 72.0, 680.0)];
    let p = |i: usize, b: Block| Piece {
        block: b,
        lines: (i < 2).then_some(i..i + 1),
    };
    let b = group(
        vec![
            p(0, Block::paragraph("• a", None)),
            p(9, Block::heading(2, "H", None)),
            p(1, Block::paragraph("• b", None)),
        ],
        &lines,
        &[],
    );
    assert_eq!(b.len(), 3);
    assert!(matches!(b[1], Block::Heading { .. }));
}

#[test]
fn vector_bullet_marks_a_line_without_a_text_marker() {
    let lines = vec![line("first", 82.0, 700.0), line("second", 82.0, 680.0)];
    let pieces = (0..2)
        .map(|i| Piece {
            block: Block::paragraph(lines[i].text.clone(), None),
            lines: Some(i..i + 1),
        })
        .collect();
    let b = group(pieces, &lines, &[Some((72.0, 1)), Some((72.0, 1))]);
    assert_eq!(texts(list(&b[0]).1), ["first", "second"]);
    // legend swatches: one colour each, or a single one, are not bullets
    let pieces = |n: usize| {
        (0..n)
            .map(|i| Piece {
                block: Block::paragraph(lines[i].text.clone(), None),
                lines: Some(i..i + 1),
            })
            .collect()
    };
    let b = group(pieces(2), &lines, &[Some((72.0, 1)), Some((72.0, 2))]);
    assert!(b.iter().all(|b| matches!(b, Block::Paragraph { .. })));
    let b = group(pieces(1), &lines, &[Some((72.0, 1))]);
    assert!(matches!(b[0], Block::Paragraph { .. }));
}

#[test]
fn list_text_keeps_every_char_but_the_markers() {
    let b = run(&[("• a b", 72.0), ("- x", 72.0), ("1) y", 72.0)]);
    let all = super::super::layout::blocks_text(&b);
    assert_eq!(all.replace('\n', " "), "a b x 1) y");
}

#[test]
fn one_nested_dash_under_a_bullet_is_a_list() {
    let b = run(&[("• top", 72.0), ("– detail", 90.0), ("• next", 72.0)]);
    assert_eq!(b.len(), 1);
    let (_, top) = list(&b[0]);
    assert_eq!(texts(top), ["top", "next"]);
    assert_eq!(texts(list(&top[0].blocks[1]).1), ["detail"]);
}

#[test]
fn numbers_in_sequence_cut_one_line_items() {
    let lines = vec![
        line("1. first item without a stop", 72.0, 700.0),
        line("2. second item", 72.0, 686.0),
        line("3) not the same separator", 72.0, 672.0),
        line("б) a letter after nothing", 72.0, 658.0),
        line("3. third item", 72.0, 644.0),
        line("4. indented elsewhere", 140.0, 630.0),
    ];
    assert_eq!(sequence_cuts(&lines, &[], &[]), [1, 4]);
    assert_eq!(sequence_cuts(&lines, &[1, 3], &[]), [1, 3, 4]);
}

#[test]
fn a_vector_bullet_cuts_only_with_a_twin() {
    let lines: Vec<StyledLine> = (0..4)
        .map(|i| line("text", 82.0, 700.0 - 14.0 * i as f32))
        .collect();
    let v = |k: u64| Some((72.0, k));
    assert_eq!(
        sequence_cuts(&lines, &[], &[None, v(1), None, v(1)]),
        [1, 3]
    );
    assert_eq!(
        sequence_cuts(&lines, &[], &[None, v(1), None, v(2)]),
        Vec::<usize>::new()
    );
    assert_eq!(
        sequence_cuts(&lines, &[], &[None, v(1), None, None]),
        Vec::<usize>::new()
    );
}

/// A page with a lead line and three lines, each with a small filled box (`colors[i]`) left of it.
fn vector_page(media: &str, ox: f32, colors: [&str; 3]) -> plaindoc_core::Document {
    use plaindoc_core::{Format, Input, ParseOptions, Parser};
    let mut s = format!(
        "BT /F1 11 Tf {} 730 Td (Skills and duties:) Tj ET\n",
        72.0 + ox
    );
    for (i, (c, t)) in colors
        .iter()
        .zip(["Wrote the parser", "Ran the reviews", "Kept the docs"])
        .enumerate()
    {
        let y = 710.0 - 16.0 * i as f32;
        s += &format!("{c} rg {} {} 4 1 re f\n", 80.0 + ox, y + 3.0);
        s += &format!("0 0 0 rg BT /F1 11 Tf {} {y} Td ({t}) Tj ET\n", 90.0 + ox);
    }
    let pdf = crate::pdf::tests::raw_pdf(media, &s);
    crate::pdf::PdfParser::new()
        .parse(Input::Bytes(pdf), Format::Pdf, &ParseOptions::default())
        .unwrap()
}

#[test]
fn vector_bullets_in_a_pdf_make_a_list() {
    let black = ["0 0 0"; 3];
    for (media, ox) in [("0 0 612 792", 0.0), ("100 50 712 842", 100.0)] {
        let d = vector_page(media, ox, black);
        let lists: Vec<&Block> = d.sections[0]
            .blocks
            .iter()
            .filter(|b| matches!(b, Block::List { .. }))
            .collect();
        assert_eq!(lists.len(), 1, "{media}: {}", d.to_markdown());
        assert_eq!(
            texts(list(lists[0]).1),
            ["Wrote the parser", "Ran the reviews", "Kept the docs"],
            "{media}"
        );
    }
    // legend swatches: one colour per line
    let d = vector_page("0 0 612 792", 0.0, ["1 0 0", "0 1 0", "0 0 1"]);
    assert!(
        !d.sections[0]
            .blocks
            .iter()
            .any(|b| matches!(b, Block::List { .. })),
        "{}",
        d.to_markdown()
    );
}

#[test]
fn one_gap_keeps_the_list_with_its_numbers() {
    let b = run(&[
        ("1. one", 72.0),
        ("2. two", 72.0),
        ("3. three", 72.0),
        ("5. five", 72.0),
        ("6. six", 72.0),
    ]);
    let (ordered, items) = list(&b[0]);
    assert!(!ordered, "the renderer would renumber 5 to 4");
    assert_eq!(texts(items)[3], "5. five");
}

#[test]
fn gost_letters_skip_ze() {
    let rows: Vec<(String, f32)> = "абвгдежиклм"
        .chars()
        .map(|c| (format!("{c}) пункт"), 72.0))
        .collect();
    let rows: Vec<(&str, f32)> = rows.iter().map(|(t, x)| (t.as_str(), *x)).collect();
    let b = run(&rows);
    assert_eq!(b.len(), 1);
    assert_eq!(list(&b[0]).1.len(), 11);
    // the full alphabet too: ж) з) и)
    let b = run(&[("ж) a", 72.0), ("з) b", 72.0), ("и) c", 72.0)]);
    assert_eq!(list(&b[0]).1.len(), 3);
}

#[test]
fn check_boxes_stay_in_the_text() {
    let b = run(&[("■ Женат", 72.0), ("□ Холост", 72.0)]);
    assert_eq!(texts(list(&b[0]).1), ["■ Женат", "□ Холост"]);
    let b = run(&[("□ Да", 72.0)]);
    assert_eq!(texts(list(&b[0]).1), ["□ Да"]);
}

/// One piece per inner slice of `(line text, x)`; lines 14 pt apart.
fn run_pieces(pieces: &[&[(&str, f32)]]) -> Vec<Block> {
    let mut lines = Vec::new();
    let mut out = Vec::new();
    for p in pieces {
        let start = lines.len();
        for (t, x) in p.iter() {
            lines.push(line(t, *x, 700.0 - 14.0 * lines.len() as f32));
        }
        let text: Vec<&str> = p.iter().map(|(t, _)| *t).collect();
        out.push(Piece {
            block: Block::paragraph(text.join(" "), None),
            lines: Some(start..lines.len()),
        });
    }
    group(out, &lines, &[])
}

#[test]
fn a_paragraph_with_a_first_line_indent_ends_the_list() {
    // marker at 0.63 cm, first-line indent at 1.25 cm, the rest of the paragraph at the margin
    let b = run_pieces(&[
        &[("• one", 90.0)],
        &[("• two", 90.0)],
        &[
            ("Next paragraph starts indented", 108.0),
            ("and goes on at the margin.", 72.0),
        ],
    ]);
    assert_eq!(b.len(), 2);
    assert_eq!(list(&b[0]).1.len(), 2);
    assert!(matches!(&b[1], Block::Paragraph { .. }));
    // one indented line in upper case is not a continuation either
    let b = run_pieces(&[&[("• one", 90.0)], &[("• two", 90.0)], &[("Short.", 108.0)]]);
    assert_eq!(b.len(), 2);
}

#[test]
fn numbered_section_titles_with_indented_bodies_are_not_a_list() {
    let body: &[(&str, f32)] = &[
        ("Настоящий договор определяет", 108.0),
        ("порядок работ.", 72.0),
    ];
    let b = run_pieces(&[
        &[("1. Общие положения", 72.0)],
        body,
        &[("2. Права и обязанности", 72.0)],
        body,
        &[("3. Ответственность сторон", 72.0)],
        body,
    ]);
    assert!(
        b.iter().all(|b| matches!(b, Block::Paragraph { .. })),
        "{b:?}"
    );
}

#[test]
fn dialogue_lines_are_not_a_list() {
    let b = run(&[
        ("Он вошёл в комнату и огляделся.", 90.0),
        ("— Здравствуйте, — сказал он.", 90.0),
        ("— Добрый вечер, — ответила хозяйка.", 90.0),
        ("Хозяйка отложила книгу.", 90.0),
    ]);
    assert!(
        b.iter().all(|b| matches!(b, Block::Paragraph { .. })),
        "{b:?}"
    );
    // an enumeration after `:` or with items ending in `;` stays a list
    for lead in ["Требования к участникам:", "Требования к участникам."]
    {
        let b = run(&[
            (lead, 90.0),
            ("— наличие лицензии;", 90.0),
            ("— опыт работы.", 90.0),
        ]);
        assert_eq!(list(&b[1]).1.len(), 2, "{lead}");
    }
}

#[test]
fn a_dash_wrapped_from_a_full_line_is_not_an_item() {
    // `run` lines are 200 pt wide: line 0 is a full line without a stop
    let b = run(&[
        ("Москва насчитывает много веков", 72.0),
        ("— это известный факт", 72.0),
        ("— и это тоже", 72.0),
    ]);
    assert!(
        b.iter().all(|b| matches!(b, Block::Paragraph { .. })),
        "{b:?}"
    );
}

#[test]
fn a_lone_dash_under_a_rejected_item_is_not_a_list() {
    let b = run(&[
        ("1. Введение в предмет", 72.0),
        ("— примечание редактора", 100.0),
    ]);
    assert!(
        b.iter().all(|b| matches!(b, Block::Paragraph { .. })),
        "{b:?}"
    );
}

#[test]
fn a_one_line_continuation_at_the_item_text_joins_the_item() {
    // `run` lines are 200 pt wide: "• " is 2 of 15 chars, the item text starts near x = 99
    for more in ["Штраф 0,1 %.", "(см. приложение)", "10 дней", "«Цитата»"]
    {
        let b = run(&[
            ("• Срок 10 дней.", 72.0),
            (more, 99.0),
            ("• Второй пункт.", 72.0),
        ]);
        assert_eq!(b.len(), 1, "{more}: {b:?}");
        let (_, items) = list(&b[0]);
        assert_eq!(items.len(), 2, "{more}");
        assert!(
            matches!(&items[0].blocks[1], Block::Paragraph { text, .. } if text == more),
            "{more}"
        );
    }
}

#[test]
fn hyphen_items_after_a_sentence_stay_a_list() {
    for m in ["-", "*"] {
        let b = run(&[
            ("Документы подаются в таком порядке.", 90.0),
            (&format!("{m} заявление."), 90.0),
            (&format!("{m} паспорт."), 90.0),
        ]);
        assert_eq!(list(&b[1]).1.len(), 2, "{m}");
    }
}
