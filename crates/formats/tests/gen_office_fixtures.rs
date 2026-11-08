#![cfg(feature = "office")]
//! Regenerates `tests/fixtures/office/generated/*`.
//! Run: `cargo test -p plaindoc-formats --features office --test gen_office_fixtures -- --ignored`
mod common;

use office_oxide::create::{create_from_ir, create_from_ir_to_writer};
use office_oxide::format::DocumentFormat;
use office_oxide::ir::*;
use std::io::{Cursor, Read, Write};
use std::path::Path;

fn p(t: &str) -> Element {
    let content = if t.is_empty() {
        vec![]
    } else {
        vec![InlineContent::Text(TextSpan::plain(t))]
    };
    Element::Paragraph(Paragraph {
        content,
        ..Default::default()
    })
}

fn h(level: u8, t: &str) -> Element {
    Element::Heading(Heading {
        level,
        content: vec![InlineContent::Text(TextSpan::plain(t))],
        ..Default::default()
    })
}

fn cell(t: &str, cs: u32, rs: u32) -> TableCell {
    TableCell {
        content: vec![p(t)],
        col_span: cs,
        row_span: rs,
        ..Default::default()
    }
}

fn row(cells: Vec<TableCell>, hdr: bool) -> TableRow {
    TableRow {
        cells,
        is_header: hdr,
        ..Default::default()
    }
}

fn item(t: &str, nested: Option<List>) -> ListItem {
    ListItem {
        content: vec![p(t)],
        nested,
    }
}

fn table(rows: Vec<TableRow>) -> Element {
    Element::Table(Table {
        rows,
        ..Default::default()
    })
}

/// T1: `Name/Qty, apple/3, pear/5`.
fn simple_table() -> Element {
    table(vec![
        row(vec![cell("Name", 1, 1), cell("Qty", 1, 1)], true),
        row(vec![cell("apple", 1, 1), cell("3", 1, 1)], false),
        row(vec![cell("pear", 1, 1), cell("5", 1, 1)], false),
    ])
}

/// T2: `H1 H2 H3 / X(rowspan 2) Y(colspan 2) / b2 c2`.
fn merged_table() -> Element {
    table(vec![
        row(
            vec![cell("H1", 1, 1), cell("H2", 1, 1), cell("H3", 1, 1)],
            true,
        ),
        row(vec![cell("X", 1, 2), cell("Y", 2, 1)], false),
        row(vec![cell("b2", 1, 1), cell("c2", 1, 1)], false),
    ])
}

/// Ordered `one{-one.a,-one.b}, two` with a nested bulleted list.
fn nested_list() -> Element {
    Element::List(List {
        ordered: true,
        items: vec![
            item(
                "one",
                Some(List {
                    ordered: false,
                    items: vec![item("one.a", None), item("one.b", None)],
                    ..Default::default()
                }),
            ),
            item("two", None),
        ],
        ..Default::default()
    })
}

/// T3: one row; cell `[p "outer", table in1/in2, p "after"]`, cell `[nested list]`.
fn table_with_nested() -> Element {
    let inner = table(vec![row(vec![cell("in1", 1, 1), cell("in2", 1, 1)], false)]);
    table(vec![row(
        vec![
            TableCell {
                content: vec![p("outer"), inner, p("after")],
                col_span: 1,
                row_span: 1,
                ..Default::default()
            },
            TableCell {
                content: vec![nested_list()],
                col_span: 1,
                row_span: 1,
                ..Default::default()
            },
        ],
        false,
    )])
}

fn sec(title: Option<&str>, elements: Vec<Element>) -> Section {
    Section {
        title: title.map(String::from),
        elements,
        ..Default::default()
    }
}

fn ir(format: DocumentFormat, sections: Vec<Section>) -> DocumentIR {
    DocumentIR {
        metadata: Metadata {
            format,
            ..Default::default()
        },
        sections,
    }
}

fn tables_docx(path: &Path) {
    let mut s = sec(
        None,
        vec![
            h(1, "Report"),
            p("Intro text."),
            p("Привет, мир"),
            simple_table(),
            merged_table(),
            nested_list(),
            table_with_nested(),
            p("Body with note"),
            Element::Footnote(Note {
                id: 1,
                content: vec![p("Footnote text")],
                ..Default::default()
            }),
        ],
    );
    s.header = Some(HeaderFooter {
        content: vec![p("Header text")],
    });
    s.footer = Some(HeaderFooter {
        content: vec![p("Footer text")],
    });
    create_from_ir(
        &ir(DocumentFormat::Docx, vec![s]),
        DocumentFormat::Docx,
        path,
    )
    .unwrap();
}

fn values_xlsx(path: &Path) {
    let mut num = cell("42", 1, 1);
    num.data_type = Some(CellDataType::Number);
    num.raw_number = Some(42.0);
    let mut f = cell("84", 1, 1);
    f.data_type = Some(CellDataType::Number);
    f.raw_number = Some(84.0);
    f.formula = Some("B2*2".into());
    let data = table(vec![
        row(vec![cell("Item", 1, 1), cell("Value", 1, 1)], true),
        row(vec![cell("a", 1, 1), num], false),
        row(vec![cell("b", 1, 1), f], false),
        row(vec![cell("merged", 2, 1)], false),
    ]);
    let doc = ir(
        DocumentFormat::Xlsx,
        vec![
            sec(Some("Data"), vec![data]),
            sec(Some("Merged"), vec![merged_table()]),
        ],
    );
    create_from_ir(&doc, DocumentFormat::Xlsx, path).unwrap();
}

fn slides_pptx(path: &Path) {
    let mut s1 = sec(
        Some("Slide one"),
        vec![
            h(1, "Slide one"),
            p("Body text on slide one"),
            nested_list(),
        ],
    );
    s1.speaker_notes = Some(vec![p("Speaker note one")]);
    let s2 = sec(Some("Slide two"), vec![h(1, "Slide two"), simple_table()]);
    create_from_ir(
        &ir(DocumentFormat::Pptx, vec![s1, s2]),
        DocumentFormat::Pptx,
        path,
    )
    .unwrap();
}

/// The pptx writer emits a plain grid; merge attributes are patched into slide XML.
fn merged_pptx(path: &Path) {
    let r = |v: &[&str]| row(v.iter().map(|t| cell(t, 1, 1)).collect(), false);
    let t = table(vec![
        r(&["H1", "H2", "H3"]),
        r(&["X", "Y", ""]),
        r(&["", "b2", "c2"]),
    ]);
    let doc = ir(DocumentFormat::Pptx, vec![sec(Some("Merged"), vec![t])]);
    let mut buf = Cursor::new(Vec::new());
    create_from_ir_to_writer(&doc, DocumentFormat::Pptx, &mut buf).unwrap();

    let mut za = zip::ZipArchive::new(Cursor::new(buf.into_inner())).unwrap();
    let mut out = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let mut patched = false;
    for i in 0..za.len() {
        let mut f = za.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        let mut data = Vec::new();
        f.read_to_end(&mut data).unwrap();
        if name == "ppt/slides/slide1.xml" {
            let mut s = String::from_utf8(data).unwrap();
            let tc = "<a:tc><a:txBody><a:bodyPr/><a:p>";
            let empty = "<a:tc><a:txBody><a:bodyPr/><a:p></a:p>";
            let mut n = 0;
            for (from, to) in [
                (
                    format!("{tc}<a:r><a:t>X<"),
                    "<a:tc rowSpan=\"2\"><a:txBody><a:bodyPr/><a:p><a:r><a:t>X<".to_string(),
                ),
                (
                    format!("{tc}<a:r><a:t>Y<"),
                    "<a:tc gridSpan=\"2\"><a:txBody><a:bodyPr/><a:p><a:r><a:t>Y<".to_string(),
                ),
                (
                    empty.to_string(),
                    "<a:tc hMerge=\"1\"><a:txBody><a:bodyPr/><a:p></a:p>".to_string(),
                ),
                (
                    empty.to_string(),
                    "<a:tc vMerge=\"1\"><a:txBody><a:bodyPr/><a:p></a:p>".to_string(),
                ),
            ] {
                if s.contains(from.as_str()) {
                    s = s.replacen(from.as_str(), &to, 1);
                    n += 1;
                }
            }
            assert!(n == 4, "merged.pptx: {n} of 4 patch anchors found");
            data = s.into_bytes();
            patched = true;
        }
        out.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        out.write_all(&data).unwrap();
    }
    out.finish().unwrap();
    assert!(patched, "merged.pptx: ppt/slides/slide1.xml not found");
}

/// OOXML encryption imitation: CFB with `EncryptionInfo` and `EncryptedPackage`.
fn encrypted_docx(path: &Path) {
    let mut comp = cfb::create(path).unwrap();
    comp.create_stream("/EncryptionInfo")
        .unwrap()
        .write_all(&[4, 0, 4, 0, 0x40, 0, 0, 0])
        .unwrap();
    comp.create_stream("/EncryptedPackage")
        .unwrap()
        .write_all(&[0u8; 64])
        .unwrap();
    comp.flush().unwrap();
}

/// 60 nested 1x1 tables, the innermost holds paragraph `"deep"`.
fn deep_docx(path: &Path) {
    let mut e = p("deep");
    for _ in 0..60 {
        e = table(vec![row(
            vec![TableCell {
                content: vec![e],
                col_span: 1,
                row_span: 1,
                ..Default::default()
            }],
            false,
        )]);
    }
    create_from_ir(
        &ir(DocumentFormat::Docx, vec![sec(None, vec![e])]),
        DocumentFormat::Docx,
        path,
    )
    .unwrap();
}

#[test]
#[ignore = "regenerates committed fixtures"]
fn generate_office_fixtures() {
    let dir = common::fixture("generated");
    std::fs::create_dir_all(&dir).unwrap();
    tables_docx(&dir.join("tables.docx"));
    values_xlsx(&dir.join("values.xlsx"));
    slides_pptx(&dir.join("slides.pptx"));
    merged_pptx(&dir.join("merged.pptx"));
    encrypted_docx(&dir.join("encrypted.docx"));
    deep_docx(&dir.join("deep.docx"));
}
