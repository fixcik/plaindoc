#![cfg(feature = "text")]
use plaindoc_core::*;
use plaindoc_formats::Registry;

#[test]
fn defaults_support_formats_of_enabled_features() {
    let r = Registry::with_defaults();
    for f in [
        Format::Txt,
        Format::Csv,
        Format::Tsv,
        Format::Md,
        Format::Xml,
    ] {
        assert!(r.supports(f), "{f}");
    }
    assert_eq!(r.supports(Format::Pdf), cfg!(feature = "pdf"));
    assert_eq!(r.supports(Format::Html), cfg!(feature = "markup"));
    assert_eq!(r.supports(Format::Odt), cfg!(feature = "odf"));
    assert_eq!(r.supports(Format::Ods), cfg!(feature = "odf"));
    assert_eq!(r.supports(Format::Rtf), cfg!(feature = "rtf"));
}

#[test]
fn parse_csv_file_sets_format_and_quality() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.csv");
    std::fs::write(&p, "a,b\n1\n").unwrap();
    let doc = Registry::with_defaults()
        .parse(Input::Path(p), &ParseOptions::default())
        .unwrap();
    assert_eq!(doc.metadata.format, Some(Format::Csv));
    assert_eq!(doc.sections[0].quality, 0.9);
    assert_eq!(doc.quality, 0.9);
}

#[test]
fn semicolon_csv_file_through_registry() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("s.csv");
    std::fs::write(&p, "a;b\n1;2\n").unwrap();
    let doc = Registry::with_defaults()
        .parse(Input::Path(p), &ParseOptions::default())
        .unwrap();
    assert_eq!(doc.metadata.format, Some(Format::Csv));
    let md = doc.to_markdown();
    assert!(md.contains("| a | b |") && md.contains("| 1 | 2 |"), "{md}");
}

#[test]
fn md_file_through_registry() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("r.MD");
    std::fs::write(&p, "# Hi\n").unwrap();
    let doc = Registry::with_defaults()
        .parse(Input::Path(p), &ParseOptions::default())
        .unwrap();
    assert_eq!(doc.to_markdown(), "# Hi\n");
}

#[test]
fn bytes_without_extension_fall_back_to_txt() {
    let d = Registry::with_defaults()
        .parse(Input::Bytes(b"a,b".to_vec()), &ParseOptions::default())
        .unwrap();
    assert_eq!(d.metadata.format, Some(Format::Txt));
}

#[test]
fn source_file_with_unknown_extension_becomes_code_block() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("lib.rs");
    std::fs::write(&p, "fn f() {}\n").unwrap();
    let doc = Registry::with_defaults()
        .parse(Input::Path(p), &ParseOptions::default())
        .unwrap();
    assert_eq!(doc.metadata.format, Some(Format::Txt));
    assert_eq!(doc.to_markdown(), "```rust\nfn f() {}\n```\n");
}
