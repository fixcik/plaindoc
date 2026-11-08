//! SP8 text-layer quality on generated PDFs: false gutter in a two-column table, column order,
//! headings, running headers/footers.
use pdf_oxide::writer::DocumentBuilder;
use plaindoc_core::*;
use plaindoc_formats::pdf::{NoTables, PdfConfig, PdfParser, ReadingOrderMode};
use std::sync::Arc;

/// Table detection off: SP4 takes this label/value grid as a table, the gate is for the text path
/// (grids the table detector rejects, `NoTables` configurations).
fn parse_without_tables(b: Vec<u8>) -> Document {
    let mut c = PdfConfig::default();
    c.tables = Arc::new(NoTables);
    PdfParser::with_config(c)
        .parse(Input::Bytes(b), Format::Pdf, &ParseOptions::default())
        .unwrap()
}

fn parse(b: Vec<u8>) -> Document {
    PdfParser::new()
        .parse(Input::Bytes(b), Format::Pdf, &ParseOptions::default())
        .unwrap()
}

fn texts(d: &Document) -> Vec<String> {
    d.sections
        .iter()
        .flat_map(|s| &s.blocks)
        .filter_map(|b| match b {
            Block::Heading { level, text, .. } => Some(format!("#{level} {text}")),
            Block::Paragraph { text, .. } => Some(text.clone()),
            Block::Note { kind, text, .. } => Some(format!("{kind:?}: {text}")),
            _ => None,
        })
        .collect()
}

fn pos(all: &str, needle: &str) -> usize {
    all.find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in {all:?}"))
}

/// Label/value table with a wide empty corridor between the columns (the fork finds a gutter).
fn two_column_table() -> Vec<u8> {
    let labels = [
        "Revenue",
        "Operating costs of the group",
        "Staff",
        "Net result for the year",
        "Tax",
        "Capital expenditure",
        "Dividends",
        "Cash",
    ];
    let mut b = DocumentBuilder::new();
    let mut p = b.letter_page().font("Helvetica", 11.0);
    for (i, l) in labels.iter().enumerate() {
        let y = 700.0 - 16.0 * i as f32;
        p = p
            .at(72.0, y)
            .text(l)
            .at(400.0, y)
            .text(&format!("{} EUR", 100 + i));
    }
    p.done();
    b.build().unwrap()
}

#[test]
fn two_column_table_is_read_row_by_row() {
    let all = texts(&parse_without_tables(two_column_table())).join("\n");
    assert!(pos(&all, "Revenue") < pos(&all, "100 EUR"));
    assert!(pos(&all, "100 EUR") < pos(&all, "Operating costs"), "{all}");
    assert!(pos(&all, "Tax") < pos(&all, "104 EUR"), "{all}");
}

/// Two justified columns of numbered lines; the right column starts 6 pt higher, so the fork
/// creates its region first.
fn two_column_prose() -> Vec<u8> {
    let mut b = DocumentBuilder::new();
    let mut p = b.letter_page().font("Helvetica", 10.0);
    for i in 0..20 {
        let y = 700.0 - 12.0 * i as f32;
        p = p
            .at(72.0, y)
            .text(&format!(
                "left column line {i:02} with enough words to fill it"
            ))
            .at(320.0, y + 6.0)
            .text(&format!(
                "right column line {i:02} with enough words to fill it"
            ));
    }
    p.done();
    b.build().unwrap()
}

#[test]
fn two_column_prose_reads_left_column_first() {
    let all = texts(&parse(two_column_prose())).join("\n");
    assert!(
        pos(&all, "left column line 19") < pos(&all, "right column line 00"),
        "{all}"
    );
}

fn parse_regions(b: Vec<u8>) -> Document {
    let mut c = PdfConfig::default();
    c.layout.reading_order = ReadingOrderMode::Regions;
    PdfParser::with_config(c)
        .parse(Input::Bytes(b), Format::Pdf, &ParseOptions::default())
        .unwrap()
}

fn parse_xy_cut(b: Vec<u8>) -> Document {
    let mut c = PdfConfig::default();
    c.layout.reading_order = ReadingOrderMode::XyCut;
    PdfParser::with_config(c)
        .parse(Input::Bytes(b), Format::Pdf, &ParseOptions::default())
        .unwrap()
}

/// Two columns of prose on shared baselines around a caption whose pieces cross the gutter in the
/// middle of the page: the fork finds no corridor over the whole page and reads the page row by
/// row.
fn two_column_prose_around_a_caption() -> Vec<u8> {
    let mut b = DocumentBuilder::new();
    let mut p = b.letter_page().font("Helvetica", 10.0);
    for i in 0..21 {
        let y = 700.0 - 12.0 * i as f32;
        p = if i == 10 {
            // a caption in pieces (as math or italics split it): one piece spans the gutter
            p.at(72.0, y)
                .text("Figure 1: a caption in pieces")
                .at(240.0, y)
                .text("with x = 3 across")
                .at(340.0, y)
                .text("the rest of the caption")
        } else {
            p.at(72.0, y)
                .text(&format!("left column line {i:02} with enough words"))
                .at(320.0, y)
                .text(&format!("right column line {i:02} with enough words"))
        };
    }
    p.done();
    b.build().unwrap()
}

#[test]
fn xy_cut_reads_columns_the_fork_left_in_one_region() {
    let pdf = two_column_prose_around_a_caption();
    let all = texts(&parse_regions(pdf.clone())).join("\n");
    assert!(
        pos(&all, "right column line 00") < pos(&all, "left column line 09"),
        "region order reads rows: {all}"
    );
    let all = texts(&parse_xy_cut(pdf)).join("\n");
    let order = [
        "left column line 00",
        "left column line 09",
        "right column line 00",
        "right column line 09",
        "Figure 1",
        "left column line 11",
        "left column line 20",
        "right column line 11",
        "right column line 20",
    ];
    assert!(
        order.windows(2).all(|w| pos(&all, w[0]) < pos(&all, w[1])),
        "{all}"
    );
}

/// A Word-style form: labels, '_' runs, values printed over the runs 1.5 pt higher (drawn last).
fn form_with_underscore_fields() -> Vec<u8> {
    let mut b = DocumentBuilder::new();
    let mut p = b.letter_page().font("Helvetica", 9.0);
    let fields = [
        ("Name:", "Ann Lee"),
        ("Phone:", ""),
        ("City:", "Springfield"),
    ];
    for (i, (label, _)) in fields.iter().enumerate() {
        let y = 700.0 - 20.0 * i as f32;
        p = p
            .at(72.0, y)
            .text(label)
            .at(110.0, y)
            .text("______________________");
    }
    for (i, (_, value)) in fields.iter().enumerate() {
        if !value.is_empty() {
            p = p.at(113.0, 701.5 - 20.0 * i as f32).text(value);
        }
    }
    p.done();
    b.build().unwrap()
}

#[test]
fn form_values_follow_their_labels_without_underscore_runs() {
    let t = texts(&parse(form_with_underscore_fields()));
    for want in ["Name: Ann Lee", "Phone: ___", "City: Springfield"] {
        assert!(t.iter().any(|x| x == want), "{want:?} not in {t:?}");
    }
}

/// A one-word bold line of body size between two paragraphs: a heading on the neighbour score.
fn bold_body_size_heading() -> Vec<u8> {
    let mut b = DocumentBuilder::new();
    let mut p = b.letter_page().font("Times-Roman", 11.0);
    for (i, t) in [
        "The first paragraph describes the method that we used",
        "and it goes on for a second line of ordinary text.",
    ]
    .iter()
    .enumerate()
    {
        p = p.at(72.0, 700.0 - 13.0 * i as f32).text(t);
    }
    p = p
        .at(72.0, 660.0)
        .font("Times-Bold", 11.0)
        .text("Conclusion");
    p = p.font("Times-Roman", 11.0);
    for (i, t) in [
        "We have shown that the method works well on all of",
        "the corpora that we tried during the whole study.",
    ]
    .iter()
    .enumerate()
    {
        p = p.at(72.0, 640.0 - 13.0 * i as f32).text(t);
    }
    p.done();
    b.build().unwrap()
}

#[test]
fn bold_body_size_heading_is_detected() {
    let all = texts(&parse(bold_body_size_heading()));
    // the only heading style of the document, body-sized: a section, level 2 (SP8b size rank)
    assert!(all.iter().any(|t| t == "#2 Conclusion"), "{all:?}");
}

/// Two pages with the running head "Annual report 2025 …" and the folio "Page N".
fn annual_report() -> Vec<u8> {
    let mut b = DocumentBuilder::new();
    for i in 0..2 {
        b.letter_page()
            .font("Helvetica", 8.0)
            .at(72.0, 760.0)
            .text("Annual report 2025 of the example company")
            .font("Helvetica", 11.0)
            .at(72.0, 700.0)
            .text(&format!("Body text of page {i} with a few words in it."))
            .at(72.0, 686.0)
            .text("It goes on for another line of ordinary text.")
            .font("Helvetica", 8.0)
            .at(300.0, 30.0)
            .text(&format!("Page {}", i + 1))
            .done();
    }
    b.build().unwrap()
}

#[test]
fn running_header_after_first_page_and_every_folio_become_notes() {
    let d = parse(annual_report());
    let notes = |p: usize| -> Vec<String> {
        d.sections[p]
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Note { kind, text, .. } => Some(format!("{kind:?}: {text}")),
                _ => None,
            })
            .collect()
    };
    // SP8 ruling 1: the head's first occurrence stays body text, page numbers are marked from page 1
    assert_eq!(notes(0), ["Footer: Page 1"]);
    assert_eq!(
        notes(1),
        [
            "Header: Annual report 2025 of the example company",
            "Footer: Page 2"
        ]
    );
}

#[test]
fn running_notes_are_not_in_default_markdown() {
    let d = parse(annual_report());
    let head = "Annual report 2025 of the example company";
    // the first occurrence of the head is body text, the rest and every folio are notes
    let md = d.to_markdown();
    assert_eq!(md.matches(head).count(), 1, "{md}");
    assert!(!md.contains("Page 2"), "{md}");
    let mut o = MarkdownOptions::default();
    o.headers_footers = true;
    let md = d.to_markdown_with(&o);
    assert_eq!(md.matches(head).count(), 2, "{md}");
    assert!(md.contains("<!-- footer: Page 2 -->"), "{md}");
}

/// A 12-page deck: header "Acme / Quarterly review: <part>" with part A on pages 1 and 10–12 and B
/// on pages 2–9, footer "<dates> N / 12" on every page.
fn deck() -> Vec<u8> {
    let mut b = DocumentBuilder::new();
    for i in 0..12 {
        let sub = if i == 0 || i >= 9 {
            "Part one"
        } else {
            "Notes"
        };
        b.letter_page()
            .font("Helvetica", 8.0)
            .at(72.0, 760.0)
            .text(&format!("Acme / Quarterly review: {sub}"))
            .font("Helvetica", 11.0)
            .at(72.0, 700.0)
            .text(&format!("Item {i} has its own body text with a few words."))
            .at(72.0, 686.0)
            .text("It goes on for another line of ordinary text.")
            .font("Helvetica", 8.0)
            .at(72.0, 30.0)
            .text(&format!(
                "Prepared 01.02.2026, rev. 03.02.2026 {} / 12",
                i + 1
            ))
            .done();
    }
    b.build().unwrap()
}

#[test]
fn deck_header_variants_and_numbered_footer_are_notes_on_every_page() {
    let d = parse(deck());
    let md = d.to_markdown();
    assert!(!md.contains("Acme"), "{md}");
    assert!(!md.contains("Prepared"), "{md}");
    assert_eq!(md.matches("has its own body text").count(), 12, "{md}");
    let mut o = MarkdownOptions::default();
    o.headers_footers = true;
    let md = d.to_markdown_with(&o);
    assert_eq!(
        md.matches("<!-- header: Acme / Quarterly review").count(),
        12,
        "{md}"
    );
    assert_eq!(md.matches("<!-- footer: Prepared").count(), 12, "{md}");
}
