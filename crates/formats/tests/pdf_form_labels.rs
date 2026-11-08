//! SP8b S2 (review K2): field labels of questionnaires ("ФИО", "ПАСПОРТ", "1. Фамилия" over the
//! value or over a blank line) are not headings; the form and section titles are
//! (`tests/fixtures/pdf/forms`, `scripts/forms/gen_forms.py`).

use plaindoc_core::*;
use plaindoc_formats::pdf::PdfParser;
use serde_json::Value;
use std::path::PathBuf;

fn norm(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[test]
fn form_field_labels_are_not_headings() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdf/forms");
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "pdf"))
        .collect();
    paths.sort();
    assert!(paths.len() >= 5, "form fixtures missing");
    let mut bad = Vec::new();
    for p in paths {
        let gt: Value =
            serde_json::from_slice(&std::fs::read(p.with_extension("json")).unwrap()).unwrap();
        let doc = PdfParser::new()
            .parse(
                Input::Bytes(std::fs::read(&p).unwrap()),
                Format::Pdf,
                &ParseOptions::default(),
            )
            .unwrap();
        let heads: Vec<String> = doc
            .sections
            .iter()
            .flat_map(|s| &s.blocks)
            .filter_map(|b| match b {
                Block::Heading { text, .. } => Some(norm(text)),
                _ => None,
            })
            .collect();
        let name = p.file_stem().unwrap().to_string_lossy().into_owned();
        for l in gt["labels"].as_array().unwrap() {
            let l = norm(l.as_str().unwrap());
            if heads.contains(&l) {
                bad.push(format!("{name}: label {l:?} is a heading"));
            }
        }
        for h in gt["headings"].as_array().unwrap() {
            let t = norm(h["text"].as_str().unwrap());
            if !heads.contains(&t) {
                bad.push(format!("{name}: heading {t:?} missing"));
            }
        }
        if heads.len() != gt["headings"].as_array().unwrap().len() {
            bad.push(format!("{name}: headings {heads:?}"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
