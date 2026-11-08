#![cfg(feature = "office")]
mod common;
use common::*;

#[test]
fn fixtures_present_and_within_budget() {
    let files = all_fixtures();
    for name in [
        "generated/tables.docx",
        "generated/values.xlsx",
        "generated/slides.pptx",
        "generated/merged.pptx",
        "generated/encrypted.docx",
        "generated/deep.docx",
        "poi/table-merges.doc",
        "poi/basic_test_ppt_file.ppt",
        "poi/SimpleWithFormula.xls",
        "poi/NOTICE",
    ] {
        assert!(fixture(name).is_file(), "missing {name}");
    }
    let mut total = 0u64;
    for f in &files {
        let len = std::fs::metadata(f).unwrap().len();
        assert!(len <= 256 * 1024, "{} is {len} bytes", f.display());
        total += len;
    }
    assert!(
        total <= 2 * 1024 * 1024,
        "office fixtures total {total} bytes"
    );
    assert_eq!(files.len(), 20, "6 generated + 14 POI");
}
