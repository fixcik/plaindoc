#![cfg(feature = "text")]
use plaindoc_core::*;
use plaindoc_formats::text::{PlainTextParser, code_language};

fn parse(bytes: &str, format: Format) -> Document {
    PlainTextParser
        .parse(
            Input::Bytes(bytes.as_bytes().to_vec()),
            format,
            &ParseOptions::default(),
        )
        .unwrap()
}

fn parse_file(name: &str, body: &str, format: Format) -> Document {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(name);
    std::fs::write(&p, body).unwrap();
    PlainTextParser
        .parse(Input::Path(p), format, &ParseOptions::default())
        .unwrap()
}

#[test]
fn plain_text_is_one_code_block_as_is() {
    let d = parse("line one\n\n  *not markdown*\n\n", Format::Txt);
    let s = &d.sections[0];
    assert_eq!(s.kind, SectionKind::Body);
    assert_eq!(
        s.blocks,
        vec![Block::Code {
            lang: None,
            text: "line one\n\n  *not markdown*\n".into(),
            bbox: None
        }]
    );
    assert_eq!(d.to_markdown(), "```\nline one\n\n  *not markdown*\n```\n");
}

#[test]
fn markdown_is_passed_through_unparsed() {
    let src = "# T\n\n| a |\n|---|\n| 1 |\n";
    let d = parse(src, Format::Md);
    assert_eq!(
        d.sections[0].blocks,
        vec![Block::paragraph(src.trim_end(), None)]
    );
    assert_eq!(d.to_markdown(), src);
}

#[test]
fn language_from_extension_and_content() {
    let lang = |d: &Document| match &d.sections[0].blocks[0] {
        Block::Code { lang, .. } => lang.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        lang(&parse_file("main.rs", "fn main() {}", Format::Txt)).as_deref(),
        Some("rust")
    );
    assert_eq!(lang(&parse_file("app.log", "INFO ok", Format::Txt)), None);
    assert_eq!(
        lang(&parse("<?xml version=\"1.0\"?><a/>", Format::Xml)).as_deref(),
        Some("xml")
    );
    assert_eq!(
        code_language(None, "#!/usr/bin/env python3\nprint(1)"),
        Some("python")
    );
    assert_eq!(code_language(None, "#!/bin/sh\necho"), Some("bash"));
    assert_eq!(code_language(None, " {\"a\": [1]}\n"), Some("json"));
    assert_eq!(code_language(None, "[x] todo"), None);
    assert_eq!(code_language(Some("yml"), "a: 1"), Some("yaml"));
}

#[test]
fn backticks_in_text_lengthen_the_fence() {
    let d = parse("a ``` b", Format::Txt);
    assert_eq!(d.to_markdown(), "````\na ``` b\n````\n");
}

#[test]
fn empty_and_whitespace_files_give_an_empty_section() {
    for src in ["", " \n\n"] {
        let d = parse(src, Format::Txt);
        assert_eq!(d.sections.len(), 1);
        assert!(d.sections[0].blocks.is_empty());
    }
}

#[test]
fn garbage_warning_reaches_the_section() {
    let junk: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    let d = PlainTextParser
        .parse(Input::Bytes(junk), Format::Txt, &ParseOptions::default())
        .unwrap();
    assert!(
        d.sections[0]
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::SuspectedGarbage)
    );
}

#[test]
fn oversized_text_is_rejected() {
    let mut o = ParseOptions::default();
    o.max_text_bytes = 2;
    let r = PlainTextParser.parse(Input::Bytes(b"abc".to_vec()), Format::Txt, &o);
    assert!(matches!(r, Err(ParseError::TooLarge { size: 3, limit: 2 })));
}

#[test]
fn nul_byte_is_flagged_even_below_control_threshold() {
    let mut s = "a".repeat(200);
    s.push('\0');
    let d = parse(&s, Format::Txt);
    assert!(
        d.sections[0]
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::SuspectedGarbage && w.level == Level::Loss)
    );
}

#[test]
fn utf16_text_is_not_flagged_as_nul() {
    let mut b = vec![0xFF, 0xFE];
    for u in "hello".encode_utf16() {
        b.extend_from_slice(&u.to_le_bytes());
    }
    let d = PlainTextParser
        .parse(Input::Bytes(b), Format::Txt, &ParseOptions::default())
        .unwrap();
    assert!(d.sections[0].warnings.is_empty());
}

#[test]
fn path_over_text_limit_is_rejected_without_reading_it_all() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("big.txt");
    std::fs::write(&p, vec![b'a'; 100]).unwrap();
    let mut o = ParseOptions::default();
    o.max_text_bytes = 10;
    let r = PlainTextParser.parse(Input::Path(p), Format::Txt, &o);
    assert!(matches!(r, Err(ParseError::TooLarge { limit: 10, .. })));
}

#[cfg(unix)]
#[test]
fn endless_path_is_rejected_not_read_forever() {
    let mut o = ParseOptions::default();
    o.max_text_bytes = 1024;
    let r = PlainTextParser.parse(Input::Path("/dev/zero".into()), Format::Txt, &o);
    assert!(matches!(r, Err(ParseError::TooLarge { limit: 1024, .. })));
}
