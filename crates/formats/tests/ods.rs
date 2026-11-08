#![cfg(feature = "odf")]
mod odf_util;
use odf_util::*;
use plaindoc_core::*;
use plaindoc_formats::Registry;

const LO_SHEET: &str = r#"<table:table table:name="Лист1" table:style-name="ta1"><table:table-column table:number-columns-repeated="1024"/><table:table-row><table:table-cell office:value-type="string"><text:p>Имя</text:p></table:table-cell><table:table-cell office:value-type="string"><text:p>Сумма</text:p></table:table-cell><table:table-cell table:number-columns-repeated="1022"/></table:table-row><table:table-row><table:table-cell office:value-type="string"><text:p>Иван</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="1.5"><text:p>1,5</text:p></table:table-cell><table:table-cell table:number-columns-repeated="1022"/></table:table-row><table:table-row table:number-rows-repeated="1048574"><table:table-cell table:number-columns-repeated="1024"/></table:table-row></table:table>"#;

#[test]
fn ods_trailing_repeats_cost_nothing() {
    let d = run(ods("", LO_SHEET), Format::Ods, &ParseOptions::default()).unwrap();
    let sh = &d.sections[0];
    assert_eq!(
        (sh.kind, sh.name.as_deref()),
        (SectionKind::Sheet, Some("Лист1"))
    );
    let (rows, header) = sig(&sh.blocks[0]);
    assert_eq!(
        rows,
        vec![
            vec![s("Имя", 1, 1), s("Сумма", 1, 1)],
            vec![s("Иван", 1, 1), s("1,5", 1, 1)]
        ]
    );
    assert_eq!(header, 1);
    let Block::Table { rows, .. } = &sh.blocks[0] else {
        panic!()
    };
    assert_eq!(val(&rows[1][1]), Some(ScalarValue::Number(1.5)));
    assert_eq!(val(&rows[0][0]), Some(ScalarValue::Text("Имя".into())));
    assert_eq!(d.metadata.page_count, Some(1));
    assert_eq!(
        d.to_markdown(),
        "<!-- sheet 1: Лист1 -->\n\n| Имя | Сумма |\n| --- | --- |\n| Иван | 1,5 |\n"
    );
}

#[test]
fn ods_inner_gaps_and_repeats_are_materialized() {
    let sheet = r#"<table:table table:name="S"><table:table-row><table:table-cell office:value-type="string"><text:p>a</text:p></table:table-cell><table:table-cell table:number-columns-repeated="2"/><table:table-cell office:value-type="float" office:value="2" table:number-columns-repeated="2"><text:p>2</text:p></table:table-cell></table:table-row><table:table-row table:number-rows-repeated="2"><table:table-cell/></table:table-row><table:table-row table:number-rows-repeated="2"><table:table-cell office:value-type="string"><text:p>z</text:p></table:table-cell></table:table-row></table:table>"#;
    let d = run(ods("", sheet), Format::Ods, &ParseOptions::default()).unwrap();
    let (rows, _) = sig(&d.sections[0].blocks[0]);
    assert_eq!(
        rows[0].iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
        ["a", "", "", "2", "2"]
    );
    assert!(rows[1].is_empty() && rows[2].is_empty());
    assert_eq!((rows.len(), rows[4][0].0.as_str()), (5, "z"));
}

#[test]
fn repeated_non_empty_cells_hit_the_cell_limit() {
    let sheet = r#"<table:table table:name="S"><table:table-row table:number-rows-repeated="1000"><table:table-cell office:value-type="float" office:value="1" table:number-columns-repeated="16384"><text:p>1</text:p></table:table-cell></table:table-row></table:table>"#;
    let r = run(ods("", sheet), Format::Ods, &ParseOptions::default());
    assert!(
        matches!(&r, Err(ParseError::Limit { kind, limit: 2_000_000 }) if kind == "table cells"),
        "{r:?}"
    );
}

#[test]
fn ods_formulas_dates_merges_comments_hidden_sheets_and_max_pages() {
    let auto = r#"<style:style style:name="ta2" style:family="table"><style:table-properties table:display="false"/></style:style>"#;
    let sheets = r#"<table:table table:name="A"><table:table-row><table:table-cell table:number-columns-spanned="2" office:value-type="string"><text:p>Итог</text:p></table:table-cell><table:covered-table-cell/></table:table-row><table:table-row><table:table-cell office:value-type="date" office:date-value="2024-01-01"><text:p>01.01.24</text:p></table:table-cell><table:table-cell table:formula="of:=[.A2]+1" office:value-type="float" office:value="45293"><text:p>45293</text:p><office:annotation><dc:creator>I</dc:creator><text:p>проверить</text:p></office:annotation></table:table-cell></table:table-row></table:table><table:table table:name="B" table:style-name="ta2"><table:table-row><table:table-cell office:value-type="boolean" office:boolean-value="true"><text:p>TRUE</text:p></table:table-cell></table:table-row></table:table>"#;
    let d = run(ods(auto, sheets), Format::Ods, &ParseOptions::default()).unwrap();
    let a = &d.sections[0];
    let Block::Table { rows, .. } = &a.blocks[0] else {
        panic!()
    };
    assert_eq!(rows[0][0].colspan, 2);
    assert_eq!(val(&rows[1][0]), Some(ScalarValue::Date(45292.0)));
    assert_eq!(
        rows[1][1].value.as_ref().unwrap().formula.as_deref(),
        Some("[.A2]+1")
    );
    assert_eq!(rows[1][1].text(), "45293");
    assert_eq!(
        a.blocks[1],
        Block::Note {
            kind: NoteKind::Comment,
            text: "B2: проверить".into(),
            bbox: None
        }
    );
    assert_eq!(d.sections[1].warnings[0].kind, WarningKind::HiddenContent);
    let mut o = ParseOptions::default();
    o.max_pages = Some(1);
    let d = run(ods(auto, sheets), Format::Ods, &o).unwrap();
    assert_eq!((d.sections.len(), d.metadata.page_count), (1, Some(2)));
    assert_eq!(d.warnings[0].message, "processed 1 of 2 sheets (max_pages)");
}

#[test]
fn empty_spreadsheet_is_empty_output() {
    let d = Registry::with_defaults()
        .parse(Input::Bytes(ods("", r#"<table:table table:name="S"><table:table-row><table:table-cell/></table:table-row></table:table>"#)), &ParseOptions::default())
        .unwrap();
    assert_eq!(kinds(&d), vec![(WarningKind::EmptyOutput, Level::Warn)]);
    assert_eq!(d.quality, 0.9);
}

#[test]
fn parallel_parse_matches_sequential() {
    let docs: Vec<Vec<u8>> = (0..12)
        .map(|i| {
            if i % 2 == 0 {
                ods("", LO_SHEET)
            } else {
                odt("", &format!("<text:h>{i}</text:h><text:p>x</text:p>"))
            }
        })
        .collect();
    let r = Registry::with_defaults();
    let o = ParseOptions::default();
    let seq: Vec<String> = docs
        .iter()
        .map(|b| r.parse(Input::Bytes(b.clone()), &o).unwrap().to_json())
        .collect();
    let par: Vec<String> = r
        .parse_many(docs.into_iter().map(Input::Bytes).collect(), &o)
        .into_iter()
        .map(|d| d.unwrap().to_json())
        .collect();
    assert_eq!(seq, par);
}

// Probes P4/P4b of the review (C3) and I3.
#[test]
fn huge_repeat_attributes_are_clamped_not_a_panic() {
    let sheet = r#"<table:table table:name="S"><table:table-row><table:table-cell table:number-columns-repeated="18446744073709551615"/><table:table-cell office:value-type="string"><text:p>x</text:p></table:table-cell></table:table-row></table:table>"#;
    let d = run(ods("", sheet), Format::Ods, &ParseOptions::default()).unwrap();
    let (rows, _) = sig(&d.sections[0].blocks[0]);
    assert_eq!(rows[0].len(), 16_385);
    assert_eq!(rows[0][16_384].0, "x");
    let sheet = r#"<table:table table:name="S"><table:table-row table:number-rows-repeated="9223372036854775807"><table:table-cell office:value-type="string"><text:p>x</text:p></table:table-cell><table:table-cell office:value-type="string"><text:p>y</text:p></table:table-cell></table:table-row></table:table>"#;
    let r = run(ods("", sheet), Format::Ods, &ParseOptions::default());
    assert!(
        matches!(&r, Err(ParseError::Limit { kind, limit: 2_000_000 }) if kind == "table cells"),
        "{r:?}"
    );
}

#[test]
fn max_pages_stops_before_later_sheets_are_parsed() {
    let bomb = r#"<table:table table:name="B"><table:table-row table:number-rows-repeated="1000"><table:table-cell office:value-type="float" office:value="1" table:number-columns-repeated="16384"><text:p>1</text:p></table:table-cell></table:table-row></table:table>"#;
    let sheets = format!(
        r#"<table:table table:name="A"><table:table-row><table:table-cell office:value-type="string"><text:p>a</text:p></table:table-cell></table:table-row></table:table>{bomb}{bomb}"#
    );
    let mut o = ParseOptions::default();
    o.max_pages = Some(1);
    let d = run(ods("", &sheets), Format::Ods, &o).unwrap();
    assert_eq!((d.sections.len(), d.metadata.page_count), (1, Some(3)));
    assert_eq!(d.warnings[0].message, "processed 1 of 3 sheets (max_pages)");
}
