//! Builds small ODF packages in memory (no binary fixtures in the repository).
#![allow(dead_code)]
use std::io::Write;

use plaindoc_core::*;
use plaindoc_formats::odf::OdfParser;

use zip::write::{SimpleFileOptions, ZipWriter};

pub const NS: &str = concat!(
    r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" "#,
    r#"xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" "#,
    r#"xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" "#,
    r#"xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" "#,
    r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" "#,
    r#"xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" "#,
    r#"xmlns:xlink="http://www.w3.org/1999/xlink" "#,
    r#"xmlns:dc="http://purl.org/dc/elements/1.1/" "#,
    r#"xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0""#,
);
pub const ODT: &str = "application/vnd.oasis.opendocument.text";
pub const ODS: &str = "application/vnd.oasis.opendocument.spreadsheet";

/// `mimetype` first and stored (so `detect` sees `mimetypeapplication/...`),
/// then the other parts deflated.
pub fn package(mimetype: &str, parts: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(mimetype.as_bytes()).unwrap();
    for (name, body) in parts {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

pub fn content(auto_styles: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {NS} office:version="1.3"><office:automatic-styles>{auto_styles}</office:automatic-styles><office:body>{body}</office:body></office:document-content>"#
    )
}

pub fn odt(auto_styles: &str, text: &str) -> Vec<u8> {
    package(
        ODT,
        &[(
            "content.xml",
            &content(auto_styles, &format!("<office:text>{text}</office:text>")),
        )],
    )
}

pub fn ods(auto_styles: &str, sheets: &str) -> Vec<u8> {
    package(
        ODS,
        &[(
            "content.xml",
            &content(
                auto_styles,
                &format!("<office:spreadsheet>{sheets}</office:spreadsheet>"),
            ),
        )],
    )
}

pub fn styles_xml(master: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-styles {NS}><office:master-styles>{master}</office:master-styles></office:document-styles>"#
    )
}

pub fn meta_xml(title: &str, author: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-meta {NS}><office:meta><dc:title>{title}</dc:title><meta:initial-creator>{author}</meta:initial-creator></office:meta></office:document-meta>"#
    )
}

// ---- assertions shared by odt.rs and ods.rs ----

pub fn run(bytes: Vec<u8>, format: Format, o: &ParseOptions) -> Result<Document> {
    OdfParser.parse(Input::Bytes(bytes), format, o)
}
pub fn doc(bytes: Vec<u8>) -> Document {
    run(bytes, Format::Odt, &ParseOptions::default()).unwrap()
}
pub fn p(t: &str) -> Block {
    Block::paragraph(t, None)
}
pub fn kinds(d: &Document) -> Vec<(WarningKind, Level)> {
    d.warnings
        .iter()
        .chain(d.sections.iter().flat_map(|s| &s.warnings))
        .map(|w| (w.kind, w.level))
        .collect()
}
/// Text, colspan and rowspan of every cell.
pub type Grid = Vec<Vec<(String, u32, u32)>>;
pub fn sig(b: &Block) -> (Grid, usize) {
    let Block::Table {
        rows, header_rows, ..
    } = b
    else {
        panic!("{b:?}")
    };
    (
        rows.iter()
            .map(|r| r.iter().map(|c| (c.text(), c.colspan, c.rowspan)).collect())
            .collect(),
        *header_rows,
    )
}
pub fn s(t: &str, c: u32, r: u32) -> (String, u32, u32) {
    (t.into(), c, r)
}
pub fn val(c: &Cell) -> Option<ScalarValue> {
    c.value.as_ref().and_then(|v| v.value.clone())
}
