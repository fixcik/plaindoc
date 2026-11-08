#![cfg(feature = "markup")]
use plaindoc_core::*;
use plaindoc_formats::Registry;

fn parse_file(name: &str, bytes: &[u8], o: &ParseOptions) -> Result<Document> {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(name);
    std::fs::write(&p, bytes).unwrap();
    Registry::with_defaults().parse(Input::Path(p), o)
}

const BOOK: &str = r#"<?xml version="1.0"?>
<?mso-application progid="Excel.Sheet"?>
<Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet" xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet" xmlns:x="urn:schemas-microsoft-com:office:excel">
 <Worksheet ss:Name="Отчёт">
  <Table>
   <Row><Cell ss:MergeAcross="1"><Data ss:Type="String">Итог</Data></Cell><Cell><Data ss:Type="String">C</Data></Cell></Row>
   <Row><Cell ss:MergeDown="1"><Data ss:Type="Number">1.5</Data></Cell><Cell ss:Formula="=A2*2"><Data ss:Type="Number">3</Data></Cell><Cell><Data ss:Type="DateTime">2024-01-01T12:00:00.000</Data></Cell></Row>
   <Row><Cell ss:Index="2"><Data ss:Type="Boolean">1</Data><Comment><Data>проверить</Data></Comment></Cell><Cell><Data ss:Type="Error">#DIV/0!</Data></Cell></Row>
   <Row ss:Index="5"><Cell ss:Index="3"><Data ss:Type="String">far</Data></Cell></Row>
  </Table>
 </Worksheet>
 <Worksheet ss:Name="Hidden"><Table><Row><Cell><Data ss:Type="String">h</Data></Cell></Row></Table>
  <WorksheetOptions xmlns="urn:schemas-microsoft-com:office:excel"><Visible>SheetHidden</Visible></WorksheetOptions></Worksheet>
</Workbook>"#;

fn rows(s: &Section) -> &Vec<Vec<Cell>> {
    match &s.blocks[0] {
        Block::Table { rows, .. } => rows,
        b => panic!("{b:?}"),
    }
}
fn val(c: &Cell) -> Option<ScalarValue> {
    c.value.as_ref().and_then(|v| v.value.clone())
}

#[test]
fn generic_xml_stays_a_code_block() {
    let d = parse_file(
        "a.xml",
        b"<?xml version=\"1.0\"?>\n<a><b>1</b></a>\n",
        &ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(d.metadata.format, Some(Format::Xml));
    assert_eq!(
        d.to_markdown(),
        "```xml\n<?xml version=\"1.0\"?>\n<a><b>1</b></a>\n```\n"
    );
    // an XSLT that generates SpreadsheetML: the root is not ss:Workbook
    let xslt = r#"<?xml version="1.0"?>
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet">
 <xsl:template match="/"><ss:Workbook><ss:Worksheet ss:Name="S"/></ss:Workbook></xsl:template>
</xsl:stylesheet>
"#;
    let d = parse_file("t.xml", xslt.as_bytes(), &ParseOptions::default()).unwrap();
    assert!(
        matches!(&d.sections[0].blocks[..], [Block::Code { .. }]),
        "{d:?}"
    );
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
}

#[test]
fn xml_spreadsheet_2003_saved_as_xls_becomes_sheets() {
    let d = parse_file("report.xls", BOOK.as_bytes(), &ParseOptions::default()).unwrap();
    assert_eq!(d.metadata.format, Some(Format::Xml));
    assert_eq!(d.metadata.page_count, Some(2));
    let s = &d.sections[0];
    assert_eq!(
        (s.kind, s.name.as_deref()),
        (SectionKind::Sheet, Some("Отчёт"))
    );
    let r = rows(s);
    assert_eq!(r.len(), 5);
    assert_eq!((r[0][0].text(), r[0][0].colspan), ("Итог".to_string(), 2));
    assert_eq!(
        (r[1][0].rowspan, val(&r[1][0])),
        (2, Some(ScalarValue::Number(1.5)))
    );
    assert_eq!(
        r[1][1].value.as_ref().unwrap().formula.as_deref(),
        Some("A2*2")
    );
    assert_eq!(val(&r[1][2]), Some(ScalarValue::Date(45292.5)));
    // row 3: column A is covered by MergeDown, ss:Index=2 is the first cell
    assert_eq!(r[2].len(), 2);
    assert_eq!(val(&r[2][0]), Some(ScalarValue::Bool(true)));
    assert_eq!(val(&r[2][1]), Some(ScalarValue::Error("#DIV/0!".into())));
    assert!(r[3].is_empty());
    assert_eq!(
        r[4].iter().map(Cell::text).collect::<Vec<_>>(),
        ["", "", "far"]
    );
    assert_eq!(
        s.blocks[1],
        Block::Note {
            kind: NoteKind::Comment,
            text: "B3: проверить".into(),
            bbox: None
        }
    );
    assert_eq!(d.sections[1].warnings[0].kind, WarningKind::HiddenContent);
    // a sparse sheet (row 4 is empty) is not an uncertain table
    assert!(
        !s.warnings
            .iter()
            .any(|w| w.kind == WarningKind::TableUncertain),
        "{:?}",
        s.warnings
    );
}

#[test]
fn spreadsheet_xml_in_cp1251_and_max_pages() {
    let src = BOOK.replace(
        "<?xml version=\"1.0\"?>",
        "<?xml version=\"1.0\" encoding=\"windows-1251\"?>",
    );
    let bytes = encoding_rs::WINDOWS_1251.encode(&src).0.into_owned();
    let mut o = ParseOptions::default();
    o.max_pages = Some(1);
    let d = parse_file("r.xml", &bytes, &o).unwrap();
    assert_eq!(d.sections.len(), 1);
    assert_eq!(d.sections[0].name.as_deref(), Some("Отчёт"));
    assert_eq!(d.metadata.page_count, Some(2));
    assert_eq!(d.warnings[0].kind, WarningKind::PartialContent);
    assert_eq!(d.warnings[0].message, "processed 1 of 2 sheets (max_pages)");
}

#[test]
fn spreadsheet_xml_with_dtd_is_corrupt() {
    let src = BOOK.replace(
        "<?mso-application progid=\"Excel.Sheet\"?>",
        "<!DOCTYPE x [<!ENTITY a \"aaaa\">]>",
    );
    let r = parse_file("r.xls", src.as_bytes(), &ParseOptions::default());
    assert!(
        matches!(&r, Err(ParseError::Corrupt(m)) if m.contains("DTD")),
        "{r:?}"
    );
}

#[test]
fn huge_row_index_hits_the_cell_limit() {
    let src = BOOK.replace("<Row ss:Index=\"5\">", "<Row ss:Index=\"3000000\">");
    let r = parse_file("r.xls", src.as_bytes(), &ParseOptions::default());
    assert!(
        matches!(&r, Err(ParseError::Limit { kind, limit: 2_000_000 }) if kind == "table cells"),
        "{r:?}"
    );
}

// Probes P5/P5b of the review (C3, C4) and I3.

#[test]
fn huge_merges_are_clamped_and_cheap() {
    for v in ["10000000", "18446744073709551615"] {
        let book = format!(
            r#"<?xml version="1.0"?><Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet" xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet"><Worksheet ss:Name="S"><Table><Row><Cell ss:MergeAcross="{v}" ss:MergeDown="{v}"><Data ss:Type="String">a</Data></Cell></Row><Row ss:Index="3000"><Cell><Data ss:Type="String">b</Data></Cell></Row></Table></Worksheet></Workbook>"#
        );
        let t = std::time::Instant::now();
        let d = parse_file("m.xls", book.as_bytes(), &ParseOptions::default()).unwrap();
        assert!(
            t.elapsed() < std::time::Duration::from_secs(2),
            "{v}: {:?}",
            t.elapsed()
        );
        let r = rows(&d.sections[0]);
        assert_eq!(
            (r.len(), r[0][0].colspan, r[0][0].rowspan),
            (3000, 1000, 3000),
            "{v}"
        );
        assert_eq!(r[2999][0].text(), "b");
    }
}

#[test]
fn nested_visible_elements_are_cheap() {
    let n = 40_000;
    let chain = format!("{}{}", "<Visible>".repeat(n), "</Visible>".repeat(n));
    for place in [
        format!("<WorksheetOptions>{chain}</WorksheetOptions>"),
        chain.clone(),
    ] {
        let book = format!(
            r#"<?xml version="1.0"?><Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet" xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet"><Worksheet ss:Name="S"><Table><Row><Cell><Data ss:Type="String">a</Data></Cell></Row></Table>{place}</Worksheet></Workbook>"#
        );
        let t = std::time::Instant::now();
        let d = parse_file("v.xml", book.as_bytes(), &ParseOptions::default()).unwrap();
        assert!(
            t.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            t.elapsed()
        );
        assert_eq!(rows(&d.sections[0])[0][0].text(), "a");
        assert!(d.sections[0].warnings.is_empty());
    }
}

#[test]
fn max_pages_stops_before_later_sheets_are_parsed() {
    let src = BOOK.replace(
        "<Worksheet ss:Name=\"Hidden\"><Table>",
        "<Worksheet ss:Name=\"Hidden\"><Table><Row ss:Index=\"3000000\"/>",
    );
    let r = parse_file("r.xls", src.as_bytes(), &ParseOptions::default());
    assert!(
        matches!(&r, Err(ParseError::Limit { kind, limit: 2_000_000 }) if kind == "table cells"),
        "{r:?}"
    );
    let mut o = ParseOptions::default();
    o.max_pages = Some(1);
    let d = parse_file("r.xls", src.as_bytes(), &o).unwrap();
    assert_eq!((d.sections.len(), d.metadata.page_count), (1, Some(2)));
}

#[test]
fn covered_row_ends_are_charged() {
    // MergeDown covers columns 8000..16384; every later row walks to the column cap
    let book = format!(
        r#"<?xml version="1.0"?><Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet" xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet"><Worksheet ss:Name="S"><Table><Row>{}</Row>{}</Table></Worksheet></Workbook>"#,
        r#"<Cell ss:Index="8001" ss:MergeAcross="999" ss:MergeDown="65533"/>"#.repeat(10),
        r#"<Row><Cell ss:Index="8001"/></Row>"#.repeat(400)
    );
    assert!(book.len() < 20_000);
    let r = parse_file("c.xls", book.as_bytes(), &ParseOptions::default());
    assert!(
        matches!(&r, Err(ParseError::Limit { kind, limit: 2_000_000 }) if kind == "table cells"),
        "{:?}",
        r.map(|d| d.sections.len())
    );
}
