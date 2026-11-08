use plaindoc_core::*;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Panics;
impl Parser for Panics {
    fn formats(&self) -> &[Format] {
        &[Format::Txt]
    }
    fn parse(&self, _: Input, _: Format, _: &ParseOptions) -> Result<Document> {
        panic!("boom {}", 42)
    }
}

struct Sleeps(Duration);
impl Parser for Sleeps {
    fn formats(&self) -> &[Format] {
        &[Format::Txt]
    }
    fn parse(&self, _: Input, _: Format, _: &ParseOptions) -> Result<Document> {
        std::thread::sleep(self.0);
        Ok(Document::new(vec![]))
    }
}

#[test]
fn panic_becomes_internal_with_message() {
    let r = catch_parse(
        &Panics,
        Input::Bytes(vec![]),
        Format::Txt,
        &ParseOptions::default(),
    );
    assert!(matches!(r, Err(ParseError::Internal(m)) if m == "boom 42"));
}

#[test]
fn panic_inside_timeout_thread_is_internal_too() {
    let mut o = ParseOptions::default();
    o.timeout = Some(Duration::from_secs(5));
    let r = run_parser(Arc::new(Panics), Input::Bytes(vec![]), Format::Txt, &o);
    assert!(matches!(r, Err(ParseError::Internal(m)) if m == "boom 42"));
}

#[test]
fn timeout_returns_promptly() {
    let mut o = ParseOptions::default();
    o.timeout = Some(Duration::from_millis(50));
    let t = Instant::now();
    let r = run_parser(
        Arc::new(Sleeps(Duration::from_secs(3))),
        Input::Bytes(vec![]),
        Format::Txt,
        &o,
    );
    assert!(matches!(r, Err(ParseError::Timeout(d)) if d == Duration::from_millis(50)));
    assert!(t.elapsed() < Duration::from_secs(1));
}

#[test]
fn fast_parser_within_timeout_succeeds() {
    let mut o = ParseOptions::default();
    o.timeout = Some(Duration::from_secs(5));
    assert!(
        run_parser(
            Arc::new(Sleeps(Duration::ZERO)),
            Input::Bytes(vec![]),
            Format::Txt,
            &o
        )
        .is_ok()
    );
    let no_limit = ParseOptions::default();
    assert!(
        run_parser(
            Arc::new(Sleeps(Duration::ZERO)),
            Input::Bytes(vec![]),
            Format::Txt,
            &no_limit
        )
        .is_ok()
    );
}
