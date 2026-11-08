//! Spans and events of the `telemetry` feature under a global recording subscriber: hierarchy
//! across threads, no document content, events agree with `ParseStats`, the same events at any
//! thread count. Each test runs under its own root span ("the service request") and sees only its
//! descendants, so tests may run in parallel.

mod pdf_common;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use pdf_common::builder_pdf_bytes;
use pdf_oxide::PdfDocument;
use pdf_oxide::structured::StructuredPage;
use plaindoc_core::*;
use plaindoc_formats::Registry;
use plaindoc_formats::pdf::{DetectedTable, PdfConfig, PdfParser, TableDetector, TableInputCounts};
use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};
use tracing_core::span::Current;

const MARK: &str = "Qx7731SecretMarker";

#[derive(Clone, Debug, PartialEq)]
enum V {
    U(u64),
    I(i64),
    B(bool),
    S(String),
}

#[derive(Default)]
struct Fields(Vec<(String, V)>);

impl Visit for Fields {
    fn record_u64(&mut self, f: &Field, v: u64) {
        self.0.push((f.name().into(), V::U(v)));
    }
    fn record_i64(&mut self, f: &Field, v: i64) {
        self.0.push((f.name().into(), V::I(v)));
    }
    fn record_bool(&mut self, f: &Field, v: bool) {
        self.0.push((f.name().into(), V::B(v)));
    }
    fn record_str(&mut self, f: &Field, v: &str) {
        self.0.push((f.name().into(), V::S(v.into())));
    }
    fn record_debug(&mut self, f: &Field, v: &dyn std::fmt::Debug) {
        self.0.push((f.name().into(), V::S(format!("{v:?}"))));
    }
}

#[derive(Clone, Debug)]
struct SpanRec {
    name: &'static str,
    meta: &'static Metadata<'static>,
    target: String,
    parent: Option<u64>,
    fields: Vec<(String, V)>,
}

#[derive(Clone, Debug)]
struct EventRec {
    target: String,
    parent: Option<u64>,
    fields: Vec<(String, V)>,
}

impl EventRec {
    fn message(&self) -> &str {
        self.str("message").unwrap_or("")
    }
    fn str(&self, k: &str) -> Option<&str> {
        self.fields.iter().find_map(|(n, v)| match v {
            V::S(s) if n == k => Some(s.as_str()),
            _ => None,
        })
    }
    fn u(&self, k: &str) -> Option<u64> {
        self.fields.iter().find_map(|(n, v)| match v {
            V::U(x) if n == k => Some(*x),
            _ => None,
        })
    }
    /// Fields without durations: equal across runs.
    fn stable(&self) -> Vec<(String, V)> {
        self.fields
            .iter()
            .filter(|(n, _)| !n.ends_with("_us"))
            .cloned()
            .collect()
    }
}

#[derive(Default)]
struct Recorder {
    next: AtomicU64,
    spans: Mutex<HashMap<u64, SpanRec>>,
    events: Mutex<Vec<EventRec>>,
}

thread_local! {
    static STACK: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

fn current() -> Option<u64> {
    STACK.with(|s| s.borrow().last().copied())
}

impl Subscriber for Recorder {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::TRACE)
    }
    fn new_span(&self, a: &Attributes<'_>) -> Id {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let parent = match a.parent() {
            Some(p) => Some(p.into_u64()),
            None if a.is_contextual() => current(),
            None => None,
        };
        let mut f = Fields::default();
        a.record(&mut f);
        self.spans.lock().unwrap().insert(
            id,
            SpanRec {
                name: a.metadata().name(),
                meta: a.metadata(),
                target: a.metadata().target().into(),
                parent,
                fields: f.0,
            },
        );
        Id::from_u64(id)
    }
    fn record(&self, id: &Id, r: &Record<'_>) {
        let mut f = Fields::default();
        r.record(&mut f);
        if let Some(s) = self.spans.lock().unwrap().get_mut(&id.into_u64()) {
            s.fields.extend(f.0);
        }
    }
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    /// Without it `tracing::Span::current()` is always none and no span crosses a thread.
    fn current_span(&self) -> Current {
        let Some(id) = current() else {
            return Current::none();
        };
        let meta = self.spans.lock().unwrap().get(&id).map(|s| s.meta);
        meta.map_or_else(Current::none, |m| Current::new(Id::from_u64(id), m))
    }
    fn event(&self, e: &Event<'_>) {
        let parent = match e.parent() {
            Some(p) => Some(p.into_u64()),
            None if e.is_contextual() => current(),
            None => None,
        };
        let mut f = Fields::default();
        e.record(&mut f);
        self.events.lock().unwrap().push(EventRec {
            target: e.metadata().target().into(),
            parent,
            fields: f.0,
        });
    }
    fn enter(&self, id: &Id) {
        STACK.with(|s| s.borrow_mut().push(id.into_u64()));
    }
    fn exit(&self, id: &Id) {
        STACK.with(|s| {
            let mut s = s.borrow_mut();
            if let Some(i) = s.iter().rposition(|x| *x == id.into_u64()) {
                s.remove(i);
            }
        });
    }
}

fn recorder() -> &'static Arc<Recorder> {
    static R: OnceLock<Arc<Recorder>> = OnceLock::new();
    R.get_or_init(|| {
        let r = Arc::new(Recorder::default());
        tracing::subscriber::set_global_default(Arc::clone(&r)).expect("one global subscriber");
        r
    })
}

struct Captured {
    spans: Vec<(u64, SpanRec)>,
    events: Vec<EventRec>,
}

impl Captured {
    fn named(&self, name: &str) -> Vec<&SpanRec> {
        self.spans
            .iter()
            .filter(|(_, s)| s.name == name)
            .map(|(_, s)| s)
            .collect()
    }
    fn events(&self, message: &str) -> Vec<&EventRec> {
        self.events
            .iter()
            .filter(|e| e.message() == message)
            .collect()
    }
}

/// Runs `f` under a fresh root span and returns what was recorded below it.
fn capture<R>(f: impl FnOnce() -> R) -> (R, Captured) {
    let rec = recorder();
    static CASE: AtomicU64 = AtomicU64::new(1);
    let root = tracing::info_span!(target: "service", "request", case = CASE.fetch_add(1, Ordering::Relaxed));
    let root_id = root.id().expect("enabled").into_u64();
    let out = root.in_scope(f);
    let spans = rec.spans.lock().unwrap().clone();
    let under = |mut p: Option<u64>| {
        while let Some(x) = p {
            if x == root_id {
                return true;
            }
            p = spans.get(&x).and_then(|s| s.parent);
        }
        false
    };
    let mut sp: Vec<(u64, SpanRec)> = spans
        .iter()
        .filter(|(id, s)| **id != root_id && under(s.parent))
        .map(|(id, s)| (*id, s.clone()))
        .collect();
    sp.sort_by_key(|(id, _)| *id);
    let events = rec
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| under(e.parent))
        .cloned()
        .collect();
    (out, Captured { spans: sp, events })
}

/// A field string is a code: letters and '_' only, at most 40 chars.
fn is_code(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_alphabetic() || c == '_')
}

/// No marker anywhere (any target); string fields of our own target are codes. Other targets
/// (e.g. `ort`, which uses `tracing`, under `ocr-paddle`) may appear below our spans; `pdf_oxide`
/// logs through `log`, which does not reach a `tracing` subscriber without a bridge.
fn assert_content_free(c: &Captured) {
    let fields = c
        .spans
        .iter()
        .map(|(_, s)| (&s.target, &s.fields))
        .chain(c.events.iter().map(|e| (&e.target, &e.fields)));
    for (target, fs) in fields {
        for (name, v) in fs {
            assert!(!name.contains(MARK));
            if let V::S(s) = v {
                assert!(!s.contains(MARK), "{target}: {name} = {s}");
                if target == "plaindoc" {
                    assert!(is_code(s), "field {name} carries a non-code string: {s:?}");
                }
            }
        }
    }
    // Also every record of the process, not only those under the root: none may hold the marker.
    let rec = recorder();
    let spans = rec.spans.lock().unwrap();
    let events = rec.events.lock().unwrap();
    for fs in spans
        .values()
        .map(|s| &s.fields)
        .chain(events.iter().map(|e| &e.fields))
    {
        for (name, v) in fs {
            assert!(
                !name.contains(MARK) && !matches!(v, V::S(s) if s.contains(MARK)),
                "{name}"
            );
        }
    }
    assert!(
        c.spans
            .iter()
            .any(|(_, s)| s.target == "plaindoc" && s.name == "document")
    );
}

fn pdf_with_marker() -> Vec<u8> {
    let mut b = pdf_oxide::writer::DocumentBuilder::new();
    for i in 0..3 {
        b.letter_page()
            .font("Helvetica", 12.0)
            .at(72.0, 720.0)
            .paragraph(&format!("{MARK} body text on page {i}."))
            .done();
    }
    b.build().unwrap()
}

#[test]
fn spans_nest_under_the_callers_span_across_threads() {
    let r = Registry::with_defaults();
    let mut o = ParseOptions::default();
    o.timeout = Some(Duration::from_secs(60)); // run_parser thread + PDF_POOL
    o.page_parallel = true; // 40 >= min_parallel_pages 32: rayon per page
    let ((res, stats), c) = capture(|| r.parse_with_stats(Input::Bytes(builder_pdf_bytes(40)), &o));
    res.unwrap();
    assert!(stats.page_parallel);
    let docs = c.named("document");
    assert_eq!(docs.len(), 1);
    let pages = c.named("page");
    assert_eq!(pages.len(), 40, "stage-A page spans");
    let stages: std::collections::BTreeSet<&str> = c
        .named("stage")
        .iter()
        .filter_map(|s| {
            s.fields.iter().find_map(|(n, v)| match v {
                V::S(x) if n == "stage" => Some(x.as_str()),
                _ => None,
            })
        })
        .collect();
    for want in ["open", "text", "tables", "diagnose", "assemble"] {
        assert!(stages.contains(want), "{want}: {stages:?}");
    }
    let parsed = c.events("parsed");
    assert_eq!(parsed.len(), 1);
    assert_eq!(
        (parsed[0].str("outcome"), parsed[0].u("pages")),
        (Some("ok"), Some(40))
    );
    // parse_many: one document span per file, all under the request
    let ((), c) = capture(|| {
        let many = r.parse_many_with_stats(
            (0..3).map(|_| Input::Bytes(builder_pdf_bytes(2))).collect(),
            &ParseOptions::default(),
        );
        assert!(many.iter().all(|(r, _)| r.is_ok()));
    });
    assert_eq!(c.named("document").len(), 3);
}

/// The OCR pass runs pages of a batch on rayon: each page's `ocr` stage span still nests under
/// the request.
#[cfg(feature = "ocr")]
#[test]
fn ocr_stage_spans_nest_under_the_callers_span() {
    use plaindoc_formats::ocr::OcrConfig;
    use plaindoc_ocr::{FakeEngine, TextBox};
    let e = Arc::new(FakeEngine::new(vec![(
        TextBox::from_aabb(200.0, 2000.0, 900.0, 2060.0, 0.9),
        "synthetic".to_string(),
        0.9,
    )]));
    let mut cfg = PdfConfig::default();
    cfg.ocr = OcrConfig::with_engine(e); // FakeEngine: max_concurrency = usize::MAX, one batch
    cfg.min_parallel_pages = 2;
    let mut r = Registry::new();
    r.register(Arc::new(PdfParser::with_config(cfg)));
    let mut o = ParseOptions::default();
    o.ocr = OcrMode::Always;
    o.page_parallel = true;
    let ((res, stats), c) = capture(|| r.parse_with_stats(Input::Bytes(builder_pdf_bytes(4)), &o));
    res.unwrap();
    assert!(stats.page_parallel);
    assert_eq!(stats.pages.ocr_run, 4);
    let ocr = c
        .named("stage")
        .iter()
        .filter(|s| s.fields.contains(&("stage".into(), V::S("ocr".into()))))
        .count();
    assert_eq!(ocr, 4, "ocr stage spans under the request");
}

#[test]
fn no_document_content_in_spans_events_or_stats() {
    let r = Registry::with_defaults();
    let html = format!(
        "<html><head><title>{MARK}</title></head><body><p>{MARK} text</p><table><tr><td>{MARK}</td></tr></table></body></html>"
    );
    let mut corrupt = b"%PDF-1.4\n".to_vec();
    corrupt.extend(MARK.as_bytes());
    let mut limited = ParseOptions::default();
    limited.max_pages = Some(1);
    let cases: Vec<(Vec<u8>, ParseOptions)> = vec![
        (html.into_bytes(), ParseOptions::default()),
        (
            format!("{MARK} plain").into_bytes(),
            ParseOptions::default(),
        ),
        (pdf_with_marker(), limited),
        (corrupt, ParseOptions::default()),
    ];
    for (bytes, o) in cases {
        let ((_, stats), c) = capture(|| r.parse_with_stats(Input::Bytes(bytes), &o));
        assert_content_free(&c);
        assert!(!stats.to_json().contains(MARK) && !format!("{stats:?}").contains(MARK));
        assert_eq!(c.events("parsed").len(), 1);
    }
    #[cfg(feature = "ocr")]
    {
        use plaindoc_ocr::{FakeEngine, TextBox};
        let e = Arc::new(FakeEngine::new(vec![(
            TextBox::from_aabb(200.0, 2000.0, 900.0, 2060.0, 0.9),
            MARK.to_string(),
            0.9,
        )]));
        let r = Registry::with_ocr_engine(e);
        let mut o = ParseOptions::default();
        o.ocr = OcrMode::Always;
        let ((res, stats), c) =
            capture(|| r.parse_with_stats(Input::Bytes(builder_pdf_bytes(1)), &o));
        assert!(
            res.unwrap().to_json().contains(MARK),
            "the marker did reach the document"
        );
        assert_content_free(&c);
        assert!(!stats.to_json().contains(MARK));
    }
}

#[test]
fn limit_events_agree_with_stats() {
    let r = Registry::with_defaults();
    let mut o = ParseOptions::default();
    o.max_pages = Some(2);
    let ((_, stats), c) = capture(|| r.parse_with_stats(Input::Bytes(builder_pdf_bytes(5)), &o));
    assert_eq!(stats.limits.get(&LimitKind::MaxPages), Some(&1));
    let hits = c.events("limit_hit");
    for (k, n) in &stats.limits {
        assert_eq!(
            hits.iter()
                .filter(|e| e.str("kind") == Some(k.as_str()))
                .count() as u32,
            *n,
            "{k:?}"
        );
    }
    assert_eq!(hits.len() as u32, stats.limits.values().sum::<u32>());
}

/// Odd pages: a table-cell limit; page 3: a detector panic (the page survives).
struct Flaky;
impl TableDetector for Flaky {
    fn detect(&self, _: &PdfDocument, _: &StructuredPage) -> Result<Vec<DetectedTable>> {
        Ok(Vec::new())
    }
    fn detect_counted(
        &self,
        _: &PdfDocument,
        page: &StructuredPage,
        c: &mut TableInputCounts,
    ) -> Result<Vec<DetectedTable>> {
        c.words = page.page_index;
        if page.page_index % 2 == 1 {
            c.limit = Some(LimitKind::TableCells);
            c.panicked = page.page_index == 3;
            return Err(ParseError::Internal("x".into()));
        }
        Ok(Vec::new())
    }
}

#[test]
fn events_do_not_depend_on_thread_count() {
    let mut cfg = PdfConfig::default();
    cfg.tables = Arc::new(Flaky);
    cfg.min_parallel_pages = 2;
    let mut r = Registry::new();
    r.register(Arc::new(PdfParser::with_config(cfg)));
    let run = |parallel: bool| {
        let mut o = ParseOptions::default();
        o.page_parallel = parallel;
        capture(|| r.parse_with_stats(Input::Bytes(builder_pdf_bytes(12)), &o))
    };
    let ((_, s1), c1) = run(false);
    let ((_, s2), c2) = run(true);
    assert!(s2.page_parallel && !s1.page_parallel);
    let ev = |c: &Captured| c.events.iter().map(EventRec::stable).collect::<Vec<_>>();
    assert_eq!(ev(&c1), ev(&c2));
    let mut a = s2.without_timings();
    a.page_parallel = false;
    assert_eq!(a, s1.without_timings());
    assert_eq!(c1.events("panic_isolated").len(), 1);
    assert_eq!(c1.events("limit_hit").len(), 6);
}

#[test]
fn timeout_and_panic_are_reported() {
    struct Sleepy;
    impl Parser for Sleepy {
        fn formats(&self) -> &[Format] {
            &[Format::Txt]
        }
        fn parse(&self, _: Input, _: Format, _: &ParseOptions) -> Result<Document> {
            std::thread::sleep(Duration::from_millis(300));
            Ok(Document::default())
        }
    }
    struct Boom;
    impl Parser for Boom {
        fn formats(&self) -> &[Format] {
            &[Format::Txt]
        }
        fn parse(&self, _: Input, _: Format, _: &ParseOptions) -> Result<Document> {
            panic!("{MARK}")
        }
    }
    let mut r = Registry::new();
    r.register(Arc::new(Sleepy));
    let mut o = ParseOptions::default();
    o.timeout = Some(Duration::from_millis(20));
    let (_, c) = capture(|| r.parse_with_stats(Input::Bytes(b"x".to_vec()), &o));
    assert_eq!(c.events("parsed")[0].str("outcome"), Some("timeout"));
    assert_eq!(
        c.events("limit_hit")[0].str("kind"),
        Some("document_timeout")
    );

    let mut r = Registry::new();
    r.register(Arc::new(Boom));
    let (_, c) =
        capture(|| r.parse_with_stats(Input::Bytes(b"x".to_vec()), &ParseOptions::default()));
    assert_eq!(c.events("panic_isolated")[0].str("site"), Some("parser"));
    assert_content_free(&c);
}
