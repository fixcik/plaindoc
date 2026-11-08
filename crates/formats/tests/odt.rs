#![cfg(feature = "odf")]
mod odf_util;
use odf_util::*;
use plaindoc_core::*;
use plaindoc_formats::Registry;

#[test]
fn odt_headings_paragraphs_spaces_and_breaks() {
    let d = doc(odt(
        "",
        r#"<text:h text:outline-level="2">Глава</text:h><text:p>a<text:s text:c="3"/>b<text:tab/>c<text:line-break/>d   <text:span>e</text:span></text:p><text:p/><text:h text:outline-level="9">deep</text:h>"#,
    ));
    assert_eq!(d.sections[0].kind, SectionKind::Body);
    assert_eq!(
        d.sections[0].blocks,
        vec![
            Block::heading(2, "Глава", None),
            p("a   b\tc\nd e"),
            Block::heading(6, "deep", None),
        ]
    );
    assert!(kinds(&d).is_empty());
}

#[test]
fn odt_lists_take_ordered_from_the_list_style_per_level() {
    let styles = r#"<text:list-style style:name="L1"><text:list-level-style-number text:level="1" style:num-format="1"/><text:list-level-style-bullet text:level="2" text:bullet-char="•"/></text:list-style>"#;
    let d = doc(odt(
        styles,
        r#"<text:list text:style-name="L1"><text:list-item><text:p>one</text:p><text:list><text:list-item><text:p>sub</text:p></text:list-item></text:list></text:list-item><text:list-item><text:p>two</text:p></text:list-item></text:list><text:list><text:list-item><text:p>no style</text:p></text:list-item></text:list>"#,
    ));
    assert_eq!(d.to_markdown(), "1. one\n   - sub\n2. two\n\n- no style\n");
}

#[test]
fn odt_table_with_spans_covered_cells_and_header_rows() {
    let d = doc(odt(
        "",
        r#"<table:table><table:table-column table:number-columns-repeated="3"/><table:table-header-rows><table:table-row><table:table-cell table:number-columns-spanned="2"><text:p>H</text:p></table:table-cell><table:covered-table-cell/><table:table-cell><text:p>C</text:p></table:table-cell></table:table-row></table:table-header-rows><table:table-row><table:table-cell table:number-rows-spanned="2"><text:p>a</text:p></table:table-cell><table:table-cell><text:p>b</text:p></table:table-cell><table:table-cell/></table:table-row><table:table-row><table:covered-table-cell/><table:table-cell><text:p>c</text:p></table:table-cell><table:table-cell/></table:table-row></table:table>"#,
    ));
    let (rows, header) = sig(&d.sections[0].blocks[0]);
    assert_eq!(
        rows,
        vec![
            vec![s("H", 2, 1), s("C", 1, 1)],
            vec![s("a", 1, 2), s("b", 1, 1), s("", 1, 1)],
            vec![s("c", 1, 1), s("", 1, 1)]
        ]
    );
    assert_eq!(header, 1);
    assert!(kinds(&d).is_empty(), "{:?}", kinds(&d));
}

#[test]
fn odt_cell_keeps_paragraph_table_paragraph_order() {
    let inner = r#"<table:table><table:table-row><table:table-cell><text:p>1</text:p></table:table-cell><table:table-cell><text:p>2</text:p></table:table-cell></table:table-row></table:table>"#;
    let d = doc(odt(
        "",
        &format!(
            r#"<table:table><table:table-row><table:table-cell><text:p>E</text:p>{inner}<text:p>F</text:p></table:table-cell><table:table-cell><text:p>x</text:p></table:table-cell></table:table-row></table:table>"#
        ),
    ));
    let Block::Table { rows, .. } = &d.sections[0].blocks[0] else {
        panic!()
    };
    assert!(matches!(
        &rows[0][0].blocks[..],
        [
            Block::Paragraph { .. },
            Block::Table { .. },
            Block::Paragraph { .. }
        ]
    ));
    assert!(d.to_markdown().starts_with("<table>"));
}

#[test]
fn odt_notes_and_comments_follow_their_block() {
    let d = doc(odt(
        "",
        r#"<text:p>Text<text:note text:note-class="footnote"><text:note-citation>1</text:note-citation><text:note-body><text:p>Сноска</text:p></text:note-body></text:note> more<office:annotation><dc:creator>I</dc:creator><text:p>Коммент</text:p></office:annotation><text:note text:note-class="endnote"><text:note-body><text:p>Конец</text:p></text:note-body></text:note></text:p><text:p>next</text:p>"#,
    ));
    assert_eq!(
        d.sections[0].blocks,
        vec![
            p("Text more"),
            Block::Note {
                kind: NoteKind::Footnote,
                text: "Сноска".into(),
                bbox: None
            },
            Block::Note {
                kind: NoteKind::Comment,
                text: "Коммент".into(),
                bbox: None
            },
            Block::Note {
                kind: NoteKind::Endnote,
                text: "Конец".into(),
                bbox: None
            },
            p("next"),
        ]
    );
}

#[test]
fn odt_headers_footers_and_metadata() {
    let master = r#"<style:master-page style:name="Standard"><style:header><text:p>Шапка</text:p></style:header><style:header-left><text:p>Шапка</text:p></style:header-left><style:footer><text:p>Подвал</text:p></style:footer></style:master-page>"#;
    let bytes = package(
        ODT,
        &[
            (
                "content.xml",
                &content("", "<office:text><text:p>body</text:p></office:text>"),
            ),
            ("styles.xml", &styles_xml(master)),
            ("meta.xml", &meta_xml("Отчёт", "Иван")),
        ],
    );
    let d = doc(bytes);
    assert_eq!(
        d.sections[0].blocks,
        vec![
            Block::Note {
                kind: NoteKind::Header,
                text: "Шапка".into(),
                bbox: None
            },
            p("body"),
            Block::Note {
                kind: NoteKind::Footer,
                text: "Подвал".into(),
                bbox: None
            },
        ]
    );
    assert_eq!(d.metadata.title.as_deref(), Some("Отчёт"));
    assert_eq!(d.metadata.author.as_deref(), Some("Иван"));
    // header and footer stay in the model, Markdown shows them only on request
    assert_eq!(d.to_markdown(), "body\n");
}

#[test]
fn odt_images_text_boxes_and_tracked_deletions() {
    let d = doc(odt(
        "",
        r#"<text:tracked-changes><text:changed-region><text:deletion><text:p>удалено</text:p></text:deletion></text:changed-region></text:tracked-changes><text:p><draw:frame><draw:image xlink:href="Pictures/a.png"/><svg:title>Схема</svg:title></draw:frame>Подпись</text:p><text:p><draw:frame><draw:text-box><text:p>в рамке</text:p></draw:text-box></draw:frame></text:p>"#,
    ));
    // the package has no Pictures/a.png: the mark stays, the picture is skipped
    let mut missing = ImageInfo::skipped("no_data", None);
    missing.source = Some("Pictures/a.png".into());
    assert_eq!(
        d.sections[0].blocks,
        vec![
            p("Подпись"),
            Block::Image {
                alt: Some("Схема".into()),
                ocr_text: None,
                bbox: None,
                image: Some(Box::new(missing)),
            },
            p("в рамке")
        ]
    );
}

#[test]
fn encrypted_package_is_encrypted() {
    let manifest = r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="content.xml"><manifest:encryption-data manifest:checksum-type="SHA1"/></manifest:file-entry></manifest:manifest>"#;
    let bytes = package(
        ODT,
        &[
            ("META-INF/manifest.xml", manifest),
            ("content.xml", "\u{1}\u{2}binary"),
        ],
    );
    assert!(matches!(
        run(bytes, Format::Odt, &ParseOptions::default()),
        Err(ParseError::Encrypted)
    ));
}

#[test]
fn generated_packages_are_detected_from_bytes() {
    let r = Registry::with_defaults();
    assert_eq!(
        r.detect(&Input::Bytes(odt("", "<text:p>x</text:p>")))
            .unwrap(),
        Format::Odt
    );
    assert_eq!(r.detect(&Input::Bytes(ods("", ""))).unwrap(), Format::Ods);
    let d = r
        .parse(
            Input::Bytes(odt("", "<text:p>x</text:p>")),
            &ParseOptions::default(),
        )
        .unwrap();
    assert_eq!(d.metadata.format, Some(Format::Odt));
    assert_eq!(d.to_markdown(), "x\n");
}

#[test]
fn oversized_part_is_a_limit_error() {
    let big = format!("<text:p>{}</text:p>", " ".repeat(2 << 20));
    let mut o = ParseOptions::default();
    o.max_text_bytes = 1 << 20;
    let bytes = odt("", &big);
    assert!(bytes.len() < 64 * 1024, "{}", bytes.len());
    let r = run(bytes, Format::Odt, &o);
    assert!(
        matches!(&r, Err(ParseError::Limit { kind, limit }) if kind == "uncompressed content.xml" && *limit == 1 << 20),
        "{r:?}"
    );
}

#[test]
fn broken_packages_are_corrupt() {
    let o = ParseOptions::default();
    assert!(matches!(
        run(b"PK\x03\x04garbage".to_vec(), Format::Odt, &o),
        Err(ParseError::Corrupt(_))
    ));
    let no_content = package(ODT, &[("styles.xml", "<x/>")]);
    assert!(
        matches!(run(no_content, Format::Odt, &o), Err(ParseError::Corrupt(m)) if m == "odf: content.xml is missing")
    );
    let dtd = package(
        ODT,
        &[("content.xml", "<!DOCTYPE x [<!ENTITY a \"a\">]><x>&a;</x>")],
    );
    assert!(matches!(
        run(dtd, Format::Odt, &o),
        Err(ParseError::Corrupt(_))
    ));
}

#[test]
fn deep_sections_are_flattened_on_a_small_stack() {
    let body = format!(
        "{}<text:p>deep</text:p>{}",
        "<text:section>".repeat(300),
        "</text:section>".repeat(300)
    );
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .stack_size(2 << 20)
        .build()
        .unwrap();
    let d = pool.install(|| doc(odt("", &body)));
    assert_eq!(d.sections[0].blocks, vec![p("deep")]);
    assert_eq!(kinds(&d), vec![(WarningKind::ContentDropped, Level::Info)]);
}

// Probe C2 of the review (M11) and rulings 4 and 5.
#[test]
fn nested_repeats_hit_the_cell_limit() {
    // 10 x 1000 cells inside each of 10 x 1000 cells: about 10^8 if clones were free.
    let inner = r#"<table:table><table:table-row table:number-rows-repeated="1000"><table:table-cell table:number-columns-repeated="10"><text:p>x</text:p></table:table-cell></table:table-row></table:table>"#;
    let body = format!(
        r#"<table:table><table:table-row table:number-rows-repeated="1000"><table:table-cell table:number-columns-repeated="10">{inner}</table:table-cell></table:table-row></table:table>"#
    );
    let t = std::time::Instant::now();
    let r = run(odt("", &body), Format::Odt, &ParseOptions::default());
    assert!(
        t.elapsed() < std::time::Duration::from_secs(2),
        "{:?}",
        t.elapsed()
    );
    assert!(
        matches!(&r, Err(ParseError::Limit { kind, limit: 2_000_000 }) if kind == "table cells"),
        "{r:?}"
    );
}

#[test]
fn hidden_text_is_skipped_and_cell_images_are_counted() {
    let d = doc(odt(
        "",
        r#"<text:p>a<text:hidden-text text:string-value="h" text:is-hidden="true">h</text:hidden-text><text:hidden-text text:string-value="v" text:is-hidden="false">v</text:hidden-text>b</text:p><text:p><text:hidden-paragraph text:condition="ooow:1" text:is-hidden="true"/>secret2</text:p><text:p><text:hidden-paragraph text:is-hidden="false"/>shown</text:p><text:section text:name="S" text:display="none"><text:p>secret</text:p></text:section><table:table><table:table-row><table:table-cell><text:p><draw:frame><draw:image xlink:href="Pictures/i.png"/></draw:frame>x</text:p></table:table-cell><table:table-cell><text:p>y</text:p></table:table-cell></table:table-row></table:table>"#,
    ));
    assert_eq!(d.sections[0].blocks[..2], [p("avb"), p("shown")]);
    assert!(!d.to_markdown().contains("secret"));
    assert_eq!(
        kinds(&d),
        vec![
            (WarningKind::HiddenContent, Level::Info),
            (WarningKind::ContentDropped, Level::Info)
        ]
    );
    assert_eq!(d.warnings[0].message, "odf: 3 hidden elements skipped");
    assert_eq!(
        d.warnings[1].message,
        "odf: 1 images in table cells dropped"
    );
}

#[test]
fn repeated_big_cells_hit_the_expanded_text_limit() {
    let big = "a".repeat(1 << 20);
    let cell = format!(
        r#"<table:table-cell table:number-columns-repeated="16384"><text:p>{big}</text:p></table:table-cell>"#
    );
    let table = format!(
        r#"<table:table table:name="S"><table:table-row>{cell}</table:table-row></table:table>"#
    );
    let o = ParseOptions::default();
    let is_limit = |r: Result<Document>| matches!(&r, Err(ParseError::Limit { kind, .. }) if kind == "odf expanded text");
    let t = std::time::Instant::now();
    assert!(is_limit(run(odt("", &table), Format::Odt, &o)));
    assert!(is_limit(run(ods("", &table), Format::Ods, &o)));
    // The same text in repeated rows.
    let rows = format!(
        r#"<table:table><table:table-row table:number-rows-repeated="100000"><table:table-cell><text:p>{big}</text:p></table:table-cell></table:table-row></table:table>"#
    );
    assert!(is_limit(run(odt("", &rows), Format::Odt, &o)));
    assert!(
        t.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        t.elapsed()
    );
}

#[test]
fn text_s_expansion_is_budgeted() {
    let spaces = r#"<text:s text:c="1024"/>"#.repeat(2000);
    let mut o = ParseOptions::default();
    o.max_text_bytes = 1 << 20;
    let r = run(
        odt("", &format!("<text:p>a{spaces}b</text:p>")),
        Format::Odt,
        &o,
    );
    assert!(matches!(&r, Err(ParseError::Limit { kind, .. }) if kind == "odf expanded text"));
}

#[test]
fn flattening_at_the_depth_limit_keeps_the_hidden_rules() {
    let body = format!(
        r#"{}<text:p>vis<text:hidden-text text:is-hidden="true">secret</text:hidden-text></text:p>{}"#,
        "<text:section>".repeat(130),
        "</text:section>".repeat(130)
    );
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .stack_size(2 << 20)
        .build()
        .unwrap();
    let d = pool.install(|| doc(odt("", &body)));
    assert_eq!(d.sections[0].blocks, vec![p("vis")]);
    assert!(
        d.warnings
            .iter()
            .any(|w| w.message == "odf: 1 hidden elements skipped")
    );
}

#[test]
fn repeated_value_only_cells_are_charged() {
    let big = "v".repeat(1 << 20);
    let table = format!(
        r#"<table:table table:name="S"><table:table-row><table:table-cell office:value-type="string" office:string-value="{big}" table:number-columns-repeated="16384"/></table:table-row></table:table>"#
    );
    let r = run(ods("", &table), Format::Ods, &ParseOptions::default());
    assert!(
        matches!(&r, Err(ParseError::Limit { kind, .. }) if kind == "odf expanded text"),
        "{r:?}"
    );
}

#[test]
fn repeated_cells_with_flattened_content_are_charged() {
    let big = "a".repeat(100_000);
    let inner = format!(
        "{}<text:p>{big}</text:p>{}",
        "<text:section>".repeat(130),
        "</text:section>".repeat(130)
    );
    let table = format!(
        r#"<table:table><table:table-row><table:table-cell table:number-columns-repeated="16384">{inner}</table:table-cell></table:table-row></table:table>"#
    );
    let mut o = ParseOptions::default();
    o.max_text_bytes = 1 << 20;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .stack_size(2 << 20)
        .build()
        .unwrap();
    let r = pool.install(|| run(odt("", &table), Format::Odt, &o));
    assert!(
        matches!(&r, Err(ParseError::Limit { kind, .. }) if kind == "odf expanded text"),
        "{r:?}"
    );
}
