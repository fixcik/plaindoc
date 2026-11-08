//! Builders of `office_oxide` IR for tests (`Element` has `Drop`: use references and `.clone()` only).

use office_oxide::ir::{
    Element, Heading, Image, InlineContent, List, ListItem, Paragraph, Section, Table, TableCell,
    TableRow, TextSpan,
};

pub fn para(t: &str) -> Element {
    Element::Paragraph(Paragraph {
        content: vec![InlineContent::Text(TextSpan::plain(t))],
        ..Default::default()
    })
}

pub fn heading(level: u8, t: &str) -> Element {
    Element::Heading(Heading {
        level,
        content: vec![InlineContent::Text(TextSpan::plain(t))],
        ..Default::default()
    })
}

pub fn cell(t: &str, cs: u32, rs: u32) -> TableCell {
    TableCell {
        content: vec![para(t)],
        col_span: cs,
        row_span: rs,
        ..Default::default()
    }
}

pub fn row(cells: Vec<TableCell>, header: bool) -> TableRow {
    TableRow {
        cells,
        is_header: header,
        ..Default::default()
    }
}

pub fn table(rows: Vec<TableRow>) -> Element {
    Element::Table(Table {
        rows,
        ..Default::default()
    })
}

pub fn item(t: &str, nested: Option<List>) -> ListItem {
    ListItem {
        content: vec![para(t)],
        nested,
    }
}

pub fn list(ordered: bool, items: Vec<ListItem>) -> Element {
    Element::List(List {
        ordered,
        items,
        ..Default::default()
    })
}

/// Clone of the `List` inside an `Element::List`.
pub fn ir_list(e: &Element) -> List {
    match e {
        Element::List(l) => l.clone(),
        _ => panic!("not a list"),
    }
}

// Used by later tasks' tests.
#[allow(dead_code)]
pub fn section(title: Option<&str>, elements: Vec<Element>) -> Section {
    Section {
        title: title.map(String::from),
        elements,
        ..Default::default()
    }
}

pub fn image(alt: Option<&str>) -> Element {
    Element::Image(Image {
        alt_text: alt.map(String::from),
        ..Default::default()
    })
}

/// Image element that carries encoded bytes (what OCR reads).
#[cfg(feature = "ocr")]
pub fn image_with_data(data: Vec<u8>) -> Element {
    Element::Image(Image {
        data: Some(data),
        ..Default::default()
    })
}
