use plaindoc_core::*;
use plaindoc_formats::{CfbProbe, Registry};
use std::sync::Arc;
use std::time::Duration;

/// Echoes the input as one paragraph after sleeping `(10 - digit) * 5` ms,
/// so earlier inputs ("0", "1", ...) finish later.
struct SlowEcho;
impl Parser for SlowEcho {
    fn formats(&self) -> &[Format] {
        &[Format::Txt]
    }
    fn parse(&self, input: Input, _: Format, _: &ParseOptions) -> Result<Document> {
        let b = input.into_bytes()?;
        let digit = b.first().map_or(0, |c| c.saturating_sub(b'0')).min(10);
        std::thread::sleep(Duration::from_millis(u64::from(10 - digit) * 5));
        let mut s = Section::new(SectionKind::Body, 0);
        s.blocks.push(Block::paragraph(
            String::from_utf8_lossy(&b).into_owned(),
            None,
        ));
        Ok(Document::new(vec![s]))
    }
}

struct Boom;
impl Parser for Boom {
    fn formats(&self) -> &[Format] {
        &[Format::Txt]
    }
    fn parse(&self, _: Input, _: Format, _: &ParseOptions) -> Result<Document> {
        panic!("library bug")
    }
}

struct Lossy;
impl Parser for Lossy {
    fn formats(&self) -> &[Format] {
        &[Format::Txt]
    }
    fn parse(&self, _: Input, _: Format, _: &ParseOptions) -> Result<Document> {
        let mut s = Section::new(SectionKind::Page, 0);
        s.warnings
            .push(Warning::new(WarningKind::NoTextLayer, Level::Loss, "x"));
        Ok(Document::new(vec![s]))
    }
}

fn echo_registry() -> Registry {
    let mut r = Registry::new();
    r.register(Arc::new(SlowEcho));
    r
}

fn bytes(s: &str) -> Input {
    Input::Bytes(s.as_bytes().to_vec())
}

#[test]
fn register_and_supports() {
    assert!(!Registry::new().supports(Format::Txt));
    let r = echo_registry();
    assert!(r.supports(Format::Txt) && !r.supports(Format::Pdf));
    let d = r.parse(bytes("5"), &ParseOptions::default()).unwrap();
    assert_eq!(d.metadata.format, Some(Format::Txt));
    assert_eq!(d.quality, 1.0);
}

#[test]
fn unsupported_and_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("x.pdf");
    std::fs::write(&pdf, b"%PDF-1.7").unwrap();
    let r = echo_registry();
    let o = ParseOptions::default();
    assert!(matches!(
        r.parse(Input::Path(pdf), &o),
        Err(ParseError::Unsupported(Format::Pdf))
    ));
    assert!(matches!(
        r.parse(Input::Path(dir.path().join("nope.txt")), &o),
        Err(ParseError::Io(_))
    ));
}

#[test]
fn max_size_is_enforced_before_parsing() {
    let mut o = ParseOptions::default();
    o.max_size = Some(3);
    assert!(matches!(
        echo_registry().parse(bytes("hello"), &o),
        Err(ParseError::TooLarge { size: 5, limit: 3 })
    ));
}

#[test]
fn panic_in_parser_is_internal_error() {
    let mut r = Registry::new();
    r.register(Arc::new(Boom));
    let e = r.parse(bytes("x"), &ParseOptions::default()).unwrap_err();
    assert!(matches!(e, ParseError::Internal(m) if m == "library bug"));
}

#[test]
fn later_registration_wins_and_quality_is_aggregated() {
    let mut r = echo_registry();
    r.register(Arc::new(Lossy));
    let d = r.parse(bytes("x"), &ParseOptions::default()).unwrap();
    assert_eq!(d.sections[0].quality, 0.5);
    assert_eq!(d.quality, 0.5);
}
/// Fails unless the Registry passes the detected format through (D4).
struct ExpectsCsv;
impl Parser for ExpectsCsv {
    fn formats(&self) -> &[Format] {
        &[Format::Csv, Format::Txt]
    }
    fn parse(&self, _: Input, format: Format, _: &ParseOptions) -> Result<Document> {
        assert_eq!(format, Format::Csv);
        Ok(Document::new(vec![]))
    }
}

#[test]
fn detected_format_is_passed_to_the_parser() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.csv");
    std::fs::write(&p, "a,b").unwrap();
    let mut r = Registry::new();
    r.register(Arc::new(ExpectsCsv));
    assert!(r.parse(Input::Path(p), &ParseOptions::default()).is_ok());
}

#[test]
fn html_disguised_as_xls_without_html_parser_is_unsupported_html() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("report.xls");
    std::fs::write(&p, "<html><table><tr><td>1</td></tr></table></html>").unwrap();
    let r = echo_registry();
    assert_eq!(r.detect(&Input::Path(p.clone())).unwrap(), Format::Html);
    assert!(matches!(
        r.parse(Input::Path(p), &ParseOptions::default()),
        Err(ParseError::Unsupported(Format::Html))
    ));
}

#[test]
fn parse_many_keeps_input_order_and_matches_sequential() {
    let r = echo_registry();
    let inputs: Vec<Input> = (0..10).map(|i| bytes(&i.to_string())).collect();
    let o = ParseOptions::default();
    let par = r.parse_many(inputs.clone(), &o);
    let seq: Vec<_> = inputs.into_iter().map(|i| r.parse(i, &o)).collect();
    let json = |v: &[Result<Document>]| -> Vec<String> {
        v.iter().map(|d| d.as_ref().unwrap().to_json()).collect()
    };
    assert_eq!(json(&par), json(&seq));
    for (i, d) in par.iter().enumerate() {
        let expected = Block::paragraph(i.to_string(), None);
        assert_eq!(d.as_ref().unwrap().sections[0].blocks[0], expected);
    }
}

#[test]
fn parse_many_isolates_failures() {
    let dir = tempfile::tempdir().unwrap();
    let res = echo_registry().parse_many(
        vec![
            bytes("1"),
            Input::Path(dir.path().join("missing.txt")),
            bytes("2"),
        ],
        &ParseOptions::default(),
    );
    assert_eq!(res.len(), 3);
    assert!(res[0].is_ok() && matches!(res[1], Err(ParseError::Io(_))) && res[2].is_ok());
}

/// Reports the `page_parallel` flag it received as paragraph text.
struct SeesPageParallel;
impl Parser for SeesPageParallel {
    fn formats(&self) -> &[Format] {
        &[Format::Txt]
    }
    fn parse(&self, _: Input, _: Format, opts: &ParseOptions) -> Result<Document> {
        let mut s = Section::new(SectionKind::Body, 0);
        s.blocks
            .push(Block::paragraph(opts.page_parallel.to_string(), None));
        Ok(Document::new(vec![s]))
    }
}

#[test]
fn page_parallel_is_kept_for_parse_and_forced_off_in_parse_many() {
    let mut r = Registry::new();
    r.register(Arc::new(SeesPageParallel));
    let mut o = ParseOptions::default();
    o.page_parallel = true;
    let seen = |d: &Document| d.sections[0].blocks[0].clone();
    let single = r.parse(bytes("x"), &o).unwrap();
    assert_eq!(seen(&single), Block::paragraph("true", None));
    let many = r.parse_many(vec![bytes("x")], &o);
    assert_eq!(
        seen(many[0].as_ref().unwrap()),
        Block::paragraph("false", None)
    );
}

struct FakeProbe(Vec<&'static str>);
impl CfbProbe for FakeProbe {
    fn stream_names(&self, _: &Input) -> Result<Vec<String>> {
        Ok(self.0.iter().map(|s| s.to_string()).collect())
    }
}

#[test]
fn cfb_probe_overrides_extension() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("a.doc");
    std::fs::write(&doc, [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0]).unwrap();
    let input = Input::Path(doc);
    let mut r = Registry::new();
    assert_eq!(r.detect(&input).unwrap(), Format::Doc);
    r.set_cfb_probe(Box::new(FakeProbe(vec![
        "EncryptionInfo",
        "EncryptedPackage",
    ])));
    assert!(matches!(r.detect(&input), Err(ParseError::Encrypted)));
    r.set_cfb_probe(Box::new(FakeProbe(vec!["Workbook"])));
    assert_eq!(r.detect(&input).unwrap(), Format::Xls);
}

struct PanicProbe;
impl CfbProbe for PanicProbe {
    fn stream_names(&self, _: &Input) -> Result<Vec<String>> {
        panic!("probe bug")
    }
}

#[test]
fn panic_in_cfb_probe_is_internal_error() {
    let mut head = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    head.extend_from_slice(&[0; 8]);
    let mut r = Registry::new();
    r.set_cfb_probe(Box::new(PanicProbe));
    let e = r.detect(&Input::Bytes(head.clone())).unwrap_err();
    assert!(matches!(e, ParseError::Internal(m) if m == "probe bug"));
    let e = r
        .parse_many(vec![Input::Bytes(head)], &ParseOptions::default())
        .remove(0)
        .unwrap_err();
    assert!(matches!(e, ParseError::Internal(_)));
}

#[cfg(unix)]
#[test]
fn fifo_input_arrives_intact_beyond_the_detect_head() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("pipe");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    assert!(made.unwrap().success());
    let content: String = "line of text\n".repeat(10_000); // 130 KB > 64 KiB
    let writer = {
        let (fifo, content) = (fifo.clone(), content.clone());
        std::thread::spawn(move || std::fs::write(fifo, content).unwrap())
    };
    let doc = echo_registry()
        .parse(Input::Path(fifo.clone()), &ParseOptions::default())
        .unwrap();
    writer.join().unwrap();
    let Block::Paragraph { text, .. } = &doc.sections[0].blocks[0] else {
        panic!("not a paragraph")
    };
    assert_eq!(text.len(), content.len());
}

#[test]
fn parse_with_stats_matches_parse_and_counts() {
    let r = echo_registry();
    let (d, s) = r.parse_with_stats(bytes("5 one two three"), &ParseOptions::default());
    let d = d.unwrap();
    assert_eq!(
        d.to_json(),
        r.parse(bytes("5 one two three"), &ParseOptions::default())
            .unwrap()
            .to_json()
    );
    assert_eq!(
        (s.format, s.outcome, s.input_bytes),
        (Some(Format::Txt), Outcome::Ok, 15)
    );
    assert_eq!(
        (s.pages.processed, s.sizes.words, s.sizes.blocks),
        (1, 4, 1)
    );
    assert!(
        s.stages_us.is_empty(),
        "no in-flight stages for a plain parser"
    );
    assert_eq!(s.est_peak_bytes, 15 + 15);
}

#[test]
fn stats_on_failures() {
    let r = echo_registry();
    let mut o = ParseOptions::default();
    o.max_size = Some(3);
    let (res, s) = r.parse_with_stats(bytes("12345"), &o);
    assert!(matches!(res, Err(ParseError::TooLarge { .. })));
    assert_eq!(
        (s.outcome, s.limits[&LimitKind::InputSize], s.format),
        (Outcome::TooLarge, 1, None)
    );

    let (res, s) = Registry::new().parse_with_stats(bytes("x"), &ParseOptions::default());
    assert!(matches!(res, Err(ParseError::Unsupported(_))));
    assert_eq!(s.outcome, Outcome::Unsupported);

    let mut boom = Registry::new();
    boom.register(Arc::new(Boom));
    let (_, s) = boom.parse_with_stats(bytes("x"), &ParseOptions::default());
    assert_eq!(s.outcome, Outcome::Internal);
}

#[test]
fn timeout_is_a_document_limit() {
    // SlowEcho sleeps (10 - 0) * 5 = 50 ms on "0".
    let r = echo_registry();
    let mut o = ParseOptions::default();
    o.timeout = Some(Duration::from_millis(5));
    let (res, s) = r.parse_with_stats(bytes("0"), &o);
    assert!(matches!(res, Err(ParseError::Timeout(_))));
    assert_eq!(
        (s.outcome, s.limits[&LimitKind::DocumentTimeout]),
        (Outcome::Timeout, 1)
    );
    assert!(s.wall_us >= 5_000);
}

#[test]
fn parse_many_with_stats_keeps_order_and_equals_single() {
    let r = echo_registry();
    let inputs = || (0..8).map(|i| bytes(&format!("{i} w"))).collect::<Vec<_>>();
    let many = r.parse_many_with_stats(inputs(), &ParseOptions::default());
    for ((m, ms), i) in many.iter().zip(inputs()) {
        let (s, ss) = r.parse_with_stats(i, &ParseOptions::default());
        assert_eq!(m.as_ref().unwrap().to_json(), s.unwrap().to_json());
        assert_eq!(ms.without_timings(), ss.without_timings());
    }
    let plain = r.parse_many(inputs(), &ParseOptions::default());
    assert_eq!(plain.len(), 8);
}
