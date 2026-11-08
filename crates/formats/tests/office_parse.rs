#![cfg(feature = "office")]
mod common;
use common::*;
use plaindoc_core::*;
use plaindoc_formats::office::OfficeParser;
use std::time::Duration;

#[test]
fn registry_routes_office_formats() {
    for rel in [
        "generated/tables.docx",
        "generated/values.xlsx",
        "generated/slides.pptx",
        "poi/simple-table.doc",
        "poi/SimpleWithFormula.xls",
        "poi/bullets.ppt",
    ] {
        let d = parse_fixture(rel).unwrap_or_else(|e| panic!("{rel}: {e}"));
        assert!(d.metadata.format.is_some(), "{rel}");
    }
}

#[test]
fn registry_probe_classifies_misnamed_legacy() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("sheet.doc");
    std::fs::copy(fixture("poi/SimpleWithFormula.xls"), &p).unwrap();
    assert_eq!(
        registry().detect(&Input::Path(p.clone())).unwrap(),
        Format::Xls
    );
    let d = registry()
        .parse(Input::Path(p), &ParseOptions::default())
        .unwrap();
    assert_eq!(d.metadata.format, Some(Format::Xls));
    assert!(matches!(
        parse_fixture("generated/encrypted.docx"),
        Err(ParseError::Encrypted)
    ));
}

#[test]
fn html_disguised_as_xls_never_reaches_office() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("report.xls");
    std::fs::write(
        &p,
        "<html><body><table><tr><td>1</td></tr></table></body></html>",
    )
    .unwrap();
    assert_eq!(
        registry().detect(&Input::Path(p.clone())).unwrap(),
        Format::Html
    );
    let r = registry().parse(Input::Path(p), &ParseOptions::default());
    assert!(
        matches!(r, Err(ParseError::Unsupported(Format::Html))) || r.is_ok(),
        "D6: html parser or Unsupported(Html), never office Corrupt: {r:?}"
    );
}

#[test]
fn docx_structure() {
    let d = parse_fixture("generated/tables.docx").unwrap();
    let t = all_text(&d);
    for s in [
        "Report",
        "Intro text.",
        "Привет, мир",
        "in1",
        "after",
        "one.a",
        "Body with note",
    ] {
        assert!(t.contains(s), "{s}");
    }
    assert_eq!(notes(&d, NoteKind::Header), ["Header text"]);
    assert_eq!(notes(&d, NoteKind::Footer), ["Footer text"]);
    let md = d.to_markdown();
    assert!(!md.contains("Header text") && !md.contains("Footer text"));
    assert!(md.contains("Footnote text"));
    let mut o = MarkdownOptions::default();
    o.headers_footers = true;
    assert!(
        d.to_markdown_with(&o)
            .contains("<!-- header: Header text -->")
    );
    assert_eq!(notes(&d, NoteKind::Footnote), ["Footnote text"]);
    assert_eq!(tables(&d).len(), 3);
    let Block::List {
        ordered: true,
        items,
        ..
    } = d.sections[0]
        .blocks
        .iter()
        .find(|b| matches!(b, Block::List { .. }))
        .unwrap()
    else {
        panic!()
    };
    assert!(matches!(
        items[0].blocks.last(),
        Some(Block::List { ordered: false, .. })
    ));
    assert_eq!(d.quality, 1.0);
}

#[test]
fn xlsx_sheets_and_values() {
    let d = parse_fixture("generated/values.xlsx").unwrap();
    let names: Vec<_> = d
        .sections
        .iter()
        .map(|s| (s.kind, s.name.clone()))
        .collect();
    assert_eq!(
        names,
        [
            (SectionKind::Sheet, Some("Data".into())),
            (SectionKind::Sheet, Some("Merged".into()))
        ]
    );
    let Block::Table {
        rows, header_rows, ..
    } = tables(&d)[0]
    else {
        panic!()
    };
    assert_eq!(*header_rows, 1);
    let v42 = rows[1][1].value.as_ref().unwrap();
    assert_eq!(
        (&v42.value, &v42.formula),
        (&Some(ScalarValue::Number(42.0)), &None)
    );
    let v84 = rows[2][1].value.as_ref().unwrap();
    assert_eq!(
        (&v84.value, v84.formula.as_deref()),
        (&Some(ScalarValue::Number(84.0)), Some("B2*2"))
    );
    assert_eq!(rows[2][1].text(), "84");
    assert_eq!(
        rows[1][0].value.as_ref().and_then(|v| v.value.clone()),
        Some(ScalarValue::Text("a".into()))
    );
}

#[test]
fn pptx_slides_notes_and_table_from_textbox() {
    let d = parse_fixture("generated/slides.pptx").unwrap();
    assert_eq!(
        d.sections
            .iter()
            .map(|s| s.name.clone().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["Slide one", "Slide two"]
    );
    assert!(d.sections.iter().all(|s| s.kind == SectionKind::Slide));
    assert_eq!(notes(&d, NoteKind::SpeakerNotes), ["Speaker note one"]);
    assert_eq!(tables(&d).len(), 1);
}

#[test]
fn legacy_formats_parse() {
    let d = parse_fixture("poi/basic_test_ppt_file.ppt").unwrap();
    assert_eq!(d.sections.len(), 2);
    assert_eq!(
        notes(&d, NoteKind::SpeakerNotes),
        [
            "These are the notes for page 1",
            "These are the notes on page two, again lacking formatting"
        ]
    );
    let f = parse_fixture("poi/footnote.doc").unwrap();
    assert_eq!(notes(&f, NoteKind::Footnote), ["TestFootnote"]);
    assert_eq!(
        notes(&f, NoteKind::Comment),
        ["Maxim Valyanskiy: TestComment"]
    );
    assert_eq!(notes(&f, NoteKind::Endnote), ["TestEndnote"]);
}

#[test]
fn xls_cells_have_text_but_no_value() {
    let d = parse_fixture("poi/SimpleWithFormula.xls").unwrap();
    let Block::Table { rows, .. } = tables(&d)[0] else {
        panic!()
    };
    assert!(rows.iter().flatten().any(|c| !c.text().is_empty()));
    assert!(rows.iter().flatten().all(|c| c.value.is_none()));
}

#[test]
fn corrupt_and_truncated_inputs() {
    let o = ParseOptions::default();
    // Office-authored pptx: [Content_Types].xml is the first ZIP entry, so the cut head is still detected as Pptx.
    let docx = std::fs::read(fixture("poi/SmartArt.pptx")).unwrap();
    assert!(matches!(
        registry().parse(Input::Bytes(docx[..docx.len() / 2].to_vec()), &o),
        Err(ParseError::Corrupt(_))
    ));
    let doc = std::fs::read(fixture("poi/table-merges.doc")).unwrap();
    assert!(matches!(
        registry().parse(Input::Bytes(doc[..doc.len() / 2].to_vec()), &o),
        Err(ParseError::Corrupt(_))
    ));
}

#[test]
fn zero_byte_and_stub_files_are_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty.docx");
    std::fs::write(&empty, b"").unwrap();
    let stub = dir.path().join("stub.doc");
    std::fs::write(
        &stub,
        [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0, 0, 0],
    )
    .unwrap();
    for p in [empty, stub] {
        let r = registry().parse(Input::Path(p.clone()), &ParseOptions::default());
        assert!(
            matches!(r, Err(ParseError::Corrupt(_))),
            "{}: {r:?}",
            p.display()
        );
    }
}

#[test]
fn xlsx_named_docx_parses_as_sheets() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("report.docx");
    std::fs::copy(fixture("generated/values.xlsx"), &p).unwrap();
    let d = registry()
        .parse(Input::Path(p), &ParseOptions::default())
        .unwrap();
    assert_eq!(d.sections[0].kind, SectionKind::Sheet);
}

#[test]
fn path_and_bytes_give_identical_documents() {
    for f in all_fixtures()
        .into_iter()
        .filter(|f| !f.ends_with("encrypted.docx"))
    {
        let a = registry()
            .parse(Input::Path(f.clone()), &ParseOptions::default())
            .unwrap();
        let b = registry()
            .parse(
                Input::Bytes(std::fs::read(&f).unwrap()),
                &ParseOptions::default(),
            )
            .unwrap();
        assert_eq!(a.to_json(), b.to_json(), "{}", f.display());
    }
}

#[test]
fn timeout_and_parse_after_timeout_still_works() {
    let mut o = ParseOptions::default();
    o.timeout = Some(Duration::ZERO);
    assert!(matches!(
        registry().parse(Input::Path(fixture("poi/table-merges.doc")), &o),
        Err(ParseError::Timeout(_))
    ));
    assert!(parse_fixture("poi/table-merges.doc").is_ok());
}

#[test]
fn max_size_is_too_large() {
    let mut o = ParseOptions::default();
    o.max_size = Some(1024);
    assert!(matches!(
        registry().parse(Input::Path(fixture("generated/tables.docx")), &o),
        Err(ParseError::TooLarge { limit: 1024, .. })
    ));
}

#[test]
fn empty_and_image_only_outputs_are_flagged() {
    let d = parse_fixture("poi/SmartArt.pptx").unwrap();
    assert!(
        d.warnings
            .iter()
            .any(|w| (w.kind, w.level) == (WarningKind::EmptyOutput, Level::Warn))
    );
    assert!(d.quality < 1.0);
}

#[test]
fn every_fixture_parses_without_internal_error() {
    for f in all_fixtures() {
        let format = registry().detect(&Input::Path(f.clone()));
        let Ok(format) = format else {
            assert!(
                matches!(format, Err(ParseError::Encrypted)),
                "{}: {format:?}",
                f.display()
            );
            continue;
        };
        if let Err(e) =
            OfficeParser::new().parse(Input::Path(f.clone()), format, &ParseOptions::default())
        {
            panic!("{}: {e:?}", f.display())
        }
    }
}

#[test]
fn xls_formulas_give_one_formula_skipped() {
    let d = parse_fixture("poi/SimpleWithFormula.xls").unwrap();
    assert_eq!(
        d.warnings
            .iter()
            .filter(|w| w.kind == WarningKind::FormulaSkipped)
            .count(),
        1
    );
}

#[test]
fn max_pages_limits_slides() {
    let mut o = ParseOptions::default();
    o.max_pages = Some(1);
    let d = registry()
        .parse(Input::Path(fixture("generated/slides.pptx")), &o)
        .unwrap();
    assert_eq!(d.sections.len(), 1);
    assert_eq!(d.metadata.page_count, Some(2));
    assert!(
        d.warnings
            .iter()
            .any(|w| (w.kind, w.level) == (WarningKind::PartialContent, Level::Warn))
    );
}

/// One slide: body text, a footer placeholder and bulleted footer / slide-number placeholders.
fn bulleted_placeholders_pptx() -> Vec<u8> {
    use std::io::Write;
    let ns = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
    let sp = |i: u32, ph: &str, text: &str, bullet: bool| {
        let bu = if bullet {
            r#"<a:pPr><a:buChar char="&#8226;"/></a:pPr>"#
        } else {
            ""
        };
        format!(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="{i}" name="s{i}"/><p:cNvSpPr/><p:nvPr><p:ph type="{ph}" idx="{i}"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p>{bu}<a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>"#
        )
    };
    let rel = |id: &str, ty: &str, target: &str| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{ty}" Target="{target}"/></Relationships>"#
        )
    };
    let parts = [
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/></Types>"#.to_string(),
        ),
        ("_rels/.rels", rel("rId1", "officeDocument", "ppt/presentation.xml")),
        (
            "ppt/presentation.xml",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><p:presentation {ns}><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
            ),
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            rel("rId2", "slide", "slides/slide1.xml"),
        ),
        (
            "ppt/slides/slide1.xml",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><p:sld {ns}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="b"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>Body</a:t></a:r></a:p></p:txBody></p:sp>{}{}{}</p:spTree></p:cSld></p:sld>"#,
                sp(3, "ftr", "PlainFooter", false),
                sp(4, "ftr", "BulletFooter", true),
                sp(5, "sldNum", "7", true)
            ),
        ),
    ];
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in parts {
        z.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(body.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

#[test]
fn pptx_bulleted_footer_placeholders_are_hidden_by_default() {
    let d = OfficeParser::new()
        .parse(
            Input::Bytes(bulleted_placeholders_pptx()),
            Format::Pptx,
            &ParseOptions::default(),
        )
        .unwrap();
    // the bulleted placeholders are notes inside list items; the model keeps them all
    assert_eq!(notes(&d, NoteKind::Footer), ["PlainFooter"]);
    let json = d.to_json();
    assert!(json.contains("\"BulletFooter\"") && json.contains("\"page_number\""));
    assert_eq!(d.to_markdown(), "<!-- slide 1 -->\n\nBody\n");
    let mut o = MarkdownOptions::default();
    o.headers_footers = true;
    let md = d.to_markdown_with(&o);
    assert!(
        md.contains("<!-- footer: BulletFooter -->") && md.contains("<!-- page-number: 7 -->"),
        "{md}"
    );
}

/// A small form: a title without heading styles, a field line above a hint, a right-tab fill
/// line and an auto-numbered footnote.
fn form_docx() -> Vec<u8> {
    use std::io::Write;
    const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
    let run = |t: &str, sz: u32, bold: bool| {
        let b = if bold { "<w:b/>" } else { "" };
        format!(
            r#"<w:r><w:rPr>{b}<w:sz w:val="{sz}"/></w:rPr><w:t xml:space="preserve">{t}</w:t></w:r>"#
        )
    };
    let center = r#"<w:pPr><w:jc w:val="center"/></w:pPr>"#;
    let line = r#"<w:pPr><w:pBdr><w:top w:val="single" w:sz="4" w:space="1" w:color="auto"/></w:pBdr><w:jc w:val="center"/></w:pPr>"#;
    let tab = r#"<w:pPr><w:tabs><w:tab w:val="right" w:pos="9925"/></w:tabs></w:pPr><w:r><w:tab/><w:t>.</w:t></w:r>"#;
    let body = [
        format!("<w:p>{center}{}</w:p>", run("ОТЗЫВ", 26, true)),
        format!("<w:p>{}</w:p>", run("1.  ", 22, false)),
        format!("<w:p>{line}{}</w:p>", run("(фамилия, имя, отчество", 18, false)),
        format!("<w:p>{tab}</w:p>"),
        format!("<w:p>{line}{}</w:p>", run("и должность)", 18, false)),
        format!(
            r#"<w:p>{}<w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:footnoteReference w:id="1"/></w:r>{}</w:p>"#,
            run("Приложение", 22, false),
            run(".", 22, false)
        ),
        format!("<w:p>{}</w:p>", run("Основной текст документа, набранный обычным шрифтом.", 22, false)),
    ]
    .concat();
    let rel = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#;
    let parts = [
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/footnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"/></Types>"#.to_string(),
        ),
        (
            "_rels/.rels",
            format!(r#"{rel}<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#),
        ),
        (
            "word/_rels/document.xml.rels",
            format!(r#"{rel}<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/></Relationships>"#),
        ),
        (
            "word/document.xml",
            format!(r#"<?xml version="1.0" encoding="UTF-8"?><w:document {W}><w:body>{body}</w:body></w:document>"#),
        ),
        (
            "word/footnotes.xml",
            format!(r#"<?xml version="1.0" encoding="UTF-8"?><w:footnotes {W}><w:footnote w:id="1"><w:p><w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:footnoteRef/></w:r><w:r><w:t xml:space="preserve"> Не прилагается.</w:t></w:r></w:p></w:footnote></w:footnotes>"#),
        ),
    ];
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in parts {
        z.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(body.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

#[test]
fn docx_form_fields_title_and_footnote_label() {
    let d = OfficeParser::new()
        .parse(
            Input::Bytes(form_docx()),
            Format::Docx,
            &ParseOptions::default(),
        )
        .unwrap();
    let md = d.to_markdown();
    assert!(!md.contains('\t'), "{md}");
    assert_eq!(
        md,
        "# ОТЗЫВ\n\n1\\. _____\n\n(фамилия, имя, отчество\n\n_____.\n\nи должность)\n\nПриложение[1].\n\nОсновной текст документа, набранный обычным шрифтом.\n\n> Footnote: [1] Не прилагается.\n"
    );
}
