#![cfg(feature = "text")]
use plaindoc_core::*;
use plaindoc_formats::text::CsvParser;

fn table(input: Input) -> (Vec<Vec<String>>, usize, Vec<WarningKind>) {
    table_as(input, Format::Csv)
}

fn table_as(input: Input, format: Format) -> (Vec<Vec<String>>, usize, Vec<WarningKind>) {
    let d = CsvParser
        .parse(input, format, &ParseOptions::default())
        .unwrap();
    let s = &d.sections[0];
    assert_eq!(s.kind, SectionKind::Sheet);
    let kinds = s.warnings.iter().map(|w| w.kind).collect();
    match &s.blocks[..] {
        [
            Block::Table {
                rows, header_rows, ..
            },
        ] => (
            rows.iter()
                .map(|r| r.iter().map(Cell::text).collect())
                .collect(),
            *header_rows,
            kinds,
        ),
        other => panic!("expected one table, got {other:?}"),
    }
}

fn bytes(s: &str) -> Input {
    Input::Bytes(s.as_bytes().to_vec())
}

#[test]
fn comma_csv_with_quotes() {
    let (rows, header, warns) = table(bytes("name,note\nA,\"x, y\"\nB,\"multi\nline\"\n"));
    assert_eq!(
        rows,
        vec![
            vec!["name", "note"],
            vec!["A", "x, y"],
            vec!["B", "multi\nline"]
        ]
    );
    assert_eq!(header, 1);
    assert!(warns.is_empty());
}

#[test]
fn semicolon_csv_is_sniffed() {
    let (rows, _, _) = table(bytes("Имя;Сумма\nИван;1,5\n"));
    assert_eq!(rows, vec![vec!["Имя", "Сумма"], vec!["Иван", "1,5"]]);
}

#[test]
fn tsv_by_format() {
    let (rows, _, _) = table_as(bytes("a,1\tb\nc\td\n"), Format::Tsv);
    assert_eq!(rows, vec![vec!["a,1", "b"], vec!["c", "d"]]);
}

#[test]
fn oversized_text_is_rejected() {
    let mut o = ParseOptions::default();
    o.max_text_bytes = 4;
    let r = CsvParser.parse(bytes("a,b,c"), Format::Csv, &o);
    assert!(matches!(r, Err(ParseError::TooLarge { size: 5, limit: 4 })));
}

#[test]
fn ragged_rows_are_padded_and_flagged() {
    let (rows, _, warns) = table(bytes("a,b,c\n1\n2,3\n"));
    assert_eq!(
        rows,
        vec![vec!["a", "b", "c"], vec!["1", "", ""], vec!["2", "3", ""]]
    );
    assert_eq!(warns, vec![WarningKind::TableUncertain]);
}

#[test]
fn empty_csv_has_no_table() {
    let d = CsvParser
        .parse(bytes(""), Format::Csv, &ParseOptions::default())
        .unwrap();
    assert!(d.sections[0].blocks.is_empty());
}

#[test]
fn units_in_header_do_not_hide_semicolons() {
    let (rows, _, _) = table(bytes("Наименование;Цена, руб.;Кол-во, шт.\nБолт;1,5;2\n"));
    assert_eq!(rows[0], vec!["Наименование", "Цена, руб.", "Кол-во, шт."]);
    assert_eq!(rows[1], vec!["Болт", "1,5", "2"]);
}

#[test]
fn decimal_comma_without_header() {
    let (rows, _, _) = table(bytes("Иван;1,5;2,75\nОльга;3,5;4,25\n"));
    assert_eq!(
        rows,
        vec![vec!["Иван", "1,5", "2,75"], vec!["Ольга", "3,5", "4,25"]]
    );
}

#[test]
fn delimiters_inside_quotes_are_ignored() {
    let (rows, _, _) = table(bytes("\"a,b\";\"c,d\";e\n1;2;3\n"));
    assert_eq!(rows, vec![vec!["a,b", "c,d", "e"], vec!["1", "2", "3"]]);
}

#[test]
fn cp1251_semicolon_file() {
    let (enc, _, _) = encoding_rs::WINDOWS_1251.encode("Имя;Город\nИван;Москва\nОльга;Тверь\n");
    let (rows, _, _) = table(Input::Bytes(enc.into_owned()));
    assert_eq!(rows[0], vec!["Имя", "Город"]);
    assert_eq!(rows[2], vec!["Ольга", "Тверь"]);
}

#[test]
fn header_only_file_is_one_row_table() {
    let (rows, header, _) = table(bytes("a;b;c\n"));
    assert_eq!(rows, vec![vec!["a", "b", "c"]]);
    assert_eq!(header, 1);
}

#[test]
fn excel_sep_line_is_honoured_and_skipped() {
    let (rows, _, _) = table(bytes("sep=;\na,b;c\n1;2\n"));
    assert_eq!(rows, vec![vec!["a,b", "c"], vec!["1", "2"]]);
}

#[test]
fn excel_sep_semicolon_is_honoured_not_sniffed() {
    let (rows, _, _) = table(bytes("sep=;\na,b,c\n1,2,3\n"));
    assert_eq!(rows, vec![vec!["a,b,c"], vec!["1,2,3"]]);
}

#[test]
fn excel_sep_tab_is_recognised() {
    let (rows, _, _) = table(bytes("sep=\t\r\na;b\tc\r\n1\t2\r\n"));
    assert_eq!(rows, vec![vec!["a;b", "c"], vec!["1", "2"]]);
}

#[test]
fn path_over_text_limit_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("big.csv");
    std::fs::write(&p, "a,b\n".repeat(20)).unwrap();
    let mut o = ParseOptions::default();
    o.max_text_bytes = 10;
    let r = CsvParser.parse(Input::Path(p), Format::Csv, &o);
    assert!(matches!(r, Err(ParseError::TooLarge { limit: 10, .. })));
}

#[test]
fn nul_after_the_detect_head_is_flagged_in_csv() {
    let mut data = "a,b\n".repeat(20_000).into_bytes(); // 80 KB of clean rows
    data.extend_from_slice(b"x,\0y\n");
    let d = CsvParser
        .parse(Input::Bytes(data), Format::Csv, &ParseOptions::default())
        .unwrap();
    let w = &d.sections[0].warnings;
    assert!(
        w.iter()
            .any(|w| w.kind == WarningKind::SuspectedGarbage && w.level == Level::Loss)
    );
    let mut d = d;
    apply_quality(&mut d, &ParseOptions::default().quality);
    assert!(d.quality < 1.0);
}
