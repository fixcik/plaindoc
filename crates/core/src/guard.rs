use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};

use crate::telemetry::{self, PanicSite};
use crate::{Document, Format, Input, ParseError, ParseOptions, ParseStats, Parser, Result};

/// Runs `parser.parse` under `catch_unwind`; a panic becomes `ParseError::Internal`.
pub fn catch_parse(
    parser: &dyn Parser,
    input: Input,
    format: Format,
    opts: &ParseOptions,
) -> Result<Document> {
    catch_parse_with_stats(parser, input, format, opts, &mut ParseStats::default())
}

/// [`catch_parse`] over [`Parser::parse_with_stats`], then [`crate::collect_images`] inside the
/// same guard (and, under [`run_parser`], the same timeout); a panic also emits
/// `panic_isolated(Parser)`.
pub fn catch_parse_with_stats(
    parser: &dyn Parser,
    input: Input,
    format: Format,
    opts: &ParseOptions,
    stats: &mut ParseStats,
) -> Result<Document> {
    match catch_unwind(AssertUnwindSafe(|| {
        parser
            .parse_with_stats(input, format, opts, stats)
            .map(|mut d| {
                crate::collect_images(&mut d, &opts.images);
                d
            })
    })) {
        Ok(result) => result,
        Err(payload) => {
            telemetry::panic_isolated(PanicSite::Parser, None);
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic payload".to_string());
            Err(ParseError::Internal(msg))
        }
    }
}

/// `catch_parse` plus `opts.timeout`. With a timeout the parser runs in a separate
/// thread; on expiry the thread is detached and finishes on its own.
pub fn run_parser(
    parser: Arc<dyn Parser>,
    input: Input,
    format: Format,
    opts: &ParseOptions,
) -> Result<Document> {
    run_parser_with_stats(parser, input, format, opts).0
}

/// [`run_parser`] returning the parser's in-flight statistics. With a timeout the parser runs on
/// its own thread under the caller's span; on expiry the thread is detached and its statistics are
/// lost (the registry still records `outcome: timeout`). The detached thread keeps running inside
/// the caller's span, so `limit_hit`/`panic_isolated` events may still arrive after `parsed`.
pub fn run_parser_with_stats(
    parser: Arc<dyn Parser>,
    input: Input,
    format: Format,
    opts: &ParseOptions,
) -> (Result<Document>, ParseStats) {
    let Some(limit) = opts.timeout else {
        let mut stats = ParseStats::default();
        let r = catch_parse_with_stats(parser.as_ref(), input, format, opts, &mut stats);
        return (r, stats);
    };
    let (tx, rx) = mpsc::channel();
    let thread_opts = opts.clone();
    let span = telemetry::Span::current();
    let spawned = std::thread::Builder::new()
        .name("plaindoc-timeout".into())
        .spawn(move || {
            span.in_scope(|| {
                let mut s = ParseStats::default();
                let r =
                    catch_parse_with_stats(parser.as_ref(), input, format, &thread_opts, &mut s);
                let _ = tx.send((r, s));
            });
        });
    if let Err(e) = spawned {
        return (Err(e.into()), ParseStats::default());
    }
    match rx.recv_timeout(limit) {
        Ok(rs) => rs,
        Err(RecvTimeoutError::Timeout) => (Err(ParseError::Timeout(limit)), ParseStats::default()),
        Err(RecvTimeoutError::Disconnected) => (
            Err(ParseError::Internal(
                "parser thread exited without a result".into(),
            )),
            ParseStats::default(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Section, SectionKind, Stage};
    use std::time::Duration;

    /// Fills one in-flight counter; sleeps `ms`; panics on "boom".
    struct Counting(u64);
    impl Parser for Counting {
        fn formats(&self) -> &[Format] {
            &[Format::Txt]
        }
        fn parse(&self, input: Input, f: Format, o: &ParseOptions) -> Result<Document> {
            self.parse_with_stats(input, f, o, &mut ParseStats::default())
        }
        fn parse_with_stats(
            &self,
            input: Input,
            _: Format,
            _: &ParseOptions,
            stats: &mut ParseStats,
        ) -> Result<Document> {
            if input.into_bytes()? == b"boom" {
                panic!("boom");
            }
            std::thread::sleep(Duration::from_millis(self.0));
            stats.add_stage(Stage::Text, Duration::from_micros(5));
            stats.sizes.spans = 11;
            Ok(Document::new(vec![Section::new(SectionKind::Body, 0)]))
        }
    }

    fn run(p: Counting, input: &[u8], timeout: Option<Duration>) -> (Result<Document>, ParseStats) {
        let o = ParseOptions {
            timeout,
            ..ParseOptions::default()
        };
        run_parser_with_stats(Arc::new(p), Input::Bytes(input.to_vec()), Format::Txt, &o)
    }

    #[test]
    fn parser_stats_come_back_with_and_without_timeout() {
        for t in [None, Some(Duration::from_secs(10))] {
            let (r, s) = run(Counting(0), b"x", t);
            assert!(r.is_ok());
            assert_eq!((s.sizes.spans, s.stages_us[&Stage::Text]), (11, 5), "{t:?}");
        }
    }

    /// A document with one pending picture; panics in `collect_images` when `PANIC_IN_COLLECT`.
    struct Pictured(bool);
    impl Parser for Pictured {
        fn formats(&self) -> &[Format] {
            &[Format::Txt]
        }
        fn parse(&self, _: Input, _: Format, _: &ParseOptions) -> Result<Document> {
            crate::images::PANIC_IN_COLLECT.with(|p| p.set(self.0));
            let mut s = Section::new(SectionKind::Body, 0);
            s.blocks.push(crate::Block::Image {
                alt: None,
                ocr_text: None,
                bbox: None,
                image: Some(Box::new(crate::ImageInfo::embedded(
                    &b"\x89PNG\r\n\x1a\n"[..],
                    None,
                ))),
            });
            Ok(Document::new(vec![s]))
        }
    }

    #[test]
    fn pictures_are_collected_inside_the_guard() {
        for t in [None, Some(Duration::from_secs(10))] {
            let o = ParseOptions {
                timeout: t,
                ..ParseOptions::default()
            };
            let run =
                |p: bool| run_parser(Arc::new(Pictured(p)), Input::Bytes(vec![]), Format::Txt, &o);
            let d = run(false).unwrap();
            assert_eq!(d.images.len(), 1, "{t:?}");
            assert!(
                matches!(run(true), Err(ParseError::Internal(m)) if m.contains("collect_images")),
                "{t:?}"
            );
            crate::images::PANIC_IN_COLLECT.with(|p| p.set(false));
        }
    }

    #[test]
    fn timeout_returns_empty_stats() {
        let (r, s) = run(Counting(300), b"x", Some(Duration::from_millis(20)));
        assert!(matches!(r, Err(ParseError::Timeout(_))));
        assert_eq!(s, ParseStats::default());
    }

    #[test]
    fn panic_is_internal_with_stats_too() {
        for t in [None, Some(Duration::from_secs(10))] {
            let (r, _) = run(Counting(0), b"boom", t);
            assert!(matches!(r, Err(ParseError::Internal(m)) if m.contains("boom")));
        }
    }

    #[test]
    fn default_parse_with_stats_leaves_stats_alone() {
        struct Plain;
        impl Parser for Plain {
            fn formats(&self) -> &[Format] {
                &[Format::Txt]
            }
            fn parse(&self, _: Input, _: Format, _: &ParseOptions) -> Result<Document> {
                Ok(Document::default())
            }
        }
        let mut s = ParseStats::default();
        Plain
            .parse_with_stats(
                Input::Bytes(vec![]),
                Format::Txt,
                &ParseOptions::default(),
                &mut s,
            )
            .unwrap();
        assert_eq!(s, ParseStats::default());
    }
}
