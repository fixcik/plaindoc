#![cfg(feature = "office")]
mod common;
use common::*;
use plaindoc_core::*;

type Sig = Vec<Vec<(String, u32, u32)>>;
fn kinds(c: &Cell) -> String {
    c.blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph { .. } => 'P',
            Block::Table { .. } => 'T',
            Block::List { .. } => 'L',
            _ => '?',
        })
        .collect()
}

#[test]
fn curated_tables_match_exactly() {
    let raw = std::fs::read_to_string(fixture("expected/tables.json")).unwrap();
    let want: std::collections::BTreeMap<String, Vec<Sig>> = serde_json::from_str(&raw).unwrap();
    let (mut ok, mut total, mut fails) = (0, 0, vec![]);
    for (file, expected) in &want {
        let got: Vec<Sig> = tables(&parse_fixture(file).unwrap())
            .into_iter()
            .map(table_signature)
            .collect();
        for (i, e) in expected.iter().enumerate() {
            total += 1;
            if got.get(i) == Some(e) {
                ok += 1
            } else {
                fails.push(format!("{file}#{i}: got {:?}", got.get(i)))
            }
        }
        if got.len() != expected.len() {
            fails.push(format!(
                "{file}: {} tables, expected {}",
                got.len(),
                expected.len()
            ))
        }
    }
    let score = ok as f64 / total as f64;
    assert!(
        score >= 1.0 && fails.is_empty(),
        "table score {score:.3} ({ok}/{total}):\n{}",
        fails.join("\n")
    );
}

#[test]
fn nested_cell_content_keeps_order() {
    let d = parse_fixture("generated/tables.docx").unwrap();
    let Block::Table { rows, .. } = tables(&d)[2] else {
        panic!()
    };
    assert_eq!(kinds(&rows[0][0]), "PTP");
    assert!(matches!(
        &rows[0][1].blocks[..],
        [Block::List { ordered: true, .. }]
    ));
    let d = parse_fixture("poi/innertable.doc").unwrap();
    let Block::Table { rows, .. } = tables(&d)[0] else {
        panic!()
    };
    assert_eq!(kinds(&rows[1][1]), "PTP");
    assert_eq!(
        table_signature(&rows[1][1].blocks[1]),
        vec![
            vec![("1".into(), 1, 1), ("2".into(), 1, 1)],
            vec![("3".into(), 1, 1), ("4".into(), 1, 1)]
        ]
    );
}

#[test]
fn rectangularity_report_over_corpus() {
    let mut uncertain = vec![];
    for f in all_fixtures()
        .into_iter()
        .filter(|f| !f.ends_with("encrypted.docx"))
    {
        let d = registry()
            .parse(Input::Path(f.clone()), &ParseOptions::default())
            .unwrap();
        let n = d
            .sections
            .iter()
            .flat_map(|s| &s.warnings)
            .chain(&d.warnings)
            .filter(|w| w.kind == WarningKind::TableUncertain)
            .count();
        if n > 0 {
            uncertain.push(format!("{}: {n}", f.display()))
        }
    }
    eprintln!("TableUncertain over corpus: {uncertain:?}");
    let curated = [
        "generated/tables.docx",
        "generated/values.xlsx",
        "generated/merged.pptx",
        "poi/table-merges.doc",
        "poi/innertable.doc",
        "poi/simple-table.doc",
    ];
    assert!(
        curated
            .iter()
            .all(|w| !uncertain.iter().any(|u| u.contains(w))),
        "{uncertain:?}"
    );
}
