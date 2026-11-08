//! SP8 checks on multipage corpora kept outside the repo (`SP8_CORPUS_DIRS`, colon-separated
//! directories of PDFs, e.g. the synthetic corpus and PubTables-1M). Quality gates (cer, headings,
//! running heads) are computed by `scripts/text/*.py` over CLI outputs.
use plaindoc_core::*;
use plaindoc_formats::pdf::{PdfConfig, PdfParser};
use std::path::PathBuf;

fn files() -> Vec<PathBuf> {
    let dirs = std::env::var("SP8_CORPUS_DIRS").expect("SP8_CORPUS_DIRS is not set");
    let mut v: Vec<PathBuf> = dirs
        .split(':')
        .flat_map(|d| std::fs::read_dir(d).unwrap_or_else(|e| panic!("{d}: {e}")))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "pdf"))
        .collect();
    v.sort();
    assert!(!v.is_empty(), "no PDFs in SP8_CORPUS_DIRS");
    v
}

#[test]
#[ignore = "needs SP8_CORPUS_DIRS"]
fn corpus_parses_and_parallel_equals_sequential() {
    let mut c = PdfConfig::default();
    c.min_parallel_pages = 1;
    let parser = PdfParser::with_config(c);
    let (mut seq, mut par) = (ParseOptions::default(), ParseOptions::default());
    seq.page_parallel = false;
    par.page_parallel = true;
    for f in files() {
        let a = parser.parse(Input::Path(f.clone()), Format::Pdf, &seq);
        let b = parser.parse(Input::Path(f.clone()), Format::Pdf, &par);
        match (a, b) {
            (Ok(a), Ok(b)) => assert_eq!(a.to_json(), b.to_json(), "{}", f.display()),
            (a, b) => assert_eq!(a.is_ok(), b.is_ok(), "{}", f.display()),
        }
    }
}
