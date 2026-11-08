use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Instant;

use rayon::prelude::*;

use plaindoc_core::{
    CFB_MAGIC, DETECT_HEAD_LEN, Document, Format, Input, ParseError, ParseOptions, ParseStats,
    Parser, Result, apply_quality, classify_cfb_streams, detect, run_parser_with_stats, telemetry,
};

/// Lists stream names of a CFB (OLE) container. Implemented by the office
/// subproject (crate `cfb`); without it CFB files are classified by extension.
///
/// Names must be bare stream names from the container root: no leading `/`
/// and no storage path (e.g. `WordDocument`, not `/WordDocument`), since
/// `classify_cfb_streams` compares them verbatim (case-insensitively).
pub trait CfbProbe: Send + Sync {
    /// Should return `Err(Corrupt(..))` for a container it cannot read;
    /// the error is propagated from [`Registry::detect`] as is.
    fn stream_names(&self, input: &Input) -> Result<Vec<String>>;
}

/// Set of registered parsers with format detection and guarded parsing.
#[derive(Default)]
pub struct Registry {
    parsers: Vec<Arc<dyn Parser>>,
    cfb_probe: Option<Box<dyn CfbProbe>>,
}

impl Registry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registry with the parsers enabled by cargo features.
    pub fn with_defaults() -> Self {
        #[allow(unused_mut)]
        let mut r = Self {
            parsers: crate::default_parsers(),
            cfb_probe: None,
        };
        #[cfg(feature = "office")]
        r.set_cfb_probe(Box::new(crate::office::CfbStreams));
        r
    }

    /// [`Registry::with_defaults`] whose pdf, image and office parsers use `engine` for OCR.
    #[cfg(feature = "ocr")]
    pub fn with_ocr_engine(engine: Arc<dyn plaindoc_ocr::OcrEngine>) -> Self {
        let mut r = Self::with_defaults();
        let ocr = crate::ocr::OcrConfig::with_engine(engine);
        #[cfg(feature = "pdf")]
        r.register(Arc::new(crate::pdf::PdfParser::with_ocr(ocr.clone())));
        r.register(Arc::new(crate::ocr::ImageParser::new(ocr.clone())));
        #[cfg(feature = "office")]
        r.register(Arc::new(crate::office::OfficeParser::with_ocr(ocr)));
        r
    }

    /// Later registration wins for formats handled by several parsers.
    pub fn register(&mut self, parser: Arc<dyn Parser>) {
        self.parsers.push(parser);
    }

    /// Enables stream-based CFB discrimination (doc/xls/ppt/encrypted OOXML).
    pub fn set_cfb_probe(&mut self, probe: Box<dyn CfbProbe>) {
        self.cfb_probe = Some(probe);
    }

    /// Whether a registered parser handles `format`.
    #[must_use]
    pub fn supports(&self, format: Format) -> bool {
        self.parser_for(format).is_some()
    }

    /// Detects the format, reading at most `DETECT_HEAD_LEN` bytes.
    ///
    /// A panic (e.g. in a [`CfbProbe`]) becomes `ParseError::Internal`.
    pub fn detect(&self, input: &Input) -> Result<Format> {
        catch_internal(|| {
            let head = read_head(input)?;
            if head.starts_with(CFB_MAGIC)
                && let Some(probe) = &self.cfb_probe
            {
                let names = probe.stream_names(input)?;
                let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                return classify_cfb_streams(&refs);
            }
            detect(&head, input.extension().as_deref())
        })
    }

    /// Checks the size limit, detects the format and runs its parser under
    /// panic and timeout guards (which also collect the pictures), then fills
    /// `metadata.format` and `quality`.
    /// Same as [`Registry::parse_with_stats`] without the statistics.
    ///
    /// # Errors
    ///
    /// `TooLarge` over `opts.max_size`, `Io`, any [`Registry::detect`] error,
    /// `Unsupported(format)` when no parser is registered, the parser's own
    /// errors, `Internal` on a panic and `Timeout` past `opts.timeout`.
    pub fn parse(&self, input: Input, opts: &ParseOptions) -> Result<Document> {
        self.parse_with_stats(input, opts).0
    }

    /// [`Registry::parse`] plus its [`ParseStats`], also on failure. Runs under a `document` span
    /// (feature `telemetry`) whose parent is the caller's current span.
    pub fn parse_with_stats(
        &self,
        input: Input,
        opts: &ParseOptions,
    ) -> (Result<Document>, ParseStats) {
        let started = Instant::now();
        // Only in-memory sizes are known here (a path is not stat'ed a second time);
        // `telemetry::finished` records the real size.
        let span = telemetry::document(match &input {
            Input::Bytes(b) => b.len() as u64,
            _ => 0,
        });
        let (result, stats) = span.in_scope(|| {
            let mut st = ParseStats::default();
            let r = self.parse_inner(input, opts, &mut st);
            st.finish(&r, started.elapsed());
            (r, st)
        });
        telemetry::finished(&span, &stats);
        (result, stats)
    }

    fn parse_inner(
        &self,
        input: Input,
        opts: &ParseOptions,
        st: &mut ParseStats,
    ) -> Result<Document> {
        let input = buffer_non_regular(input, opts)?;
        st.input_bytes = input_size(&input)?;
        check_size(st.input_bytes, opts.max_size)?;
        let format = self.detect(&input)?;
        st.format = Some(format);
        let parser = self
            .parser_for(format)
            .ok_or(ParseError::Unsupported(format))?;
        let (r, parsed) = run_parser_with_stats(Arc::clone(parser), input, format, opts);
        st.absorb(parsed);
        let mut doc = r?;
        doc.metadata.format = Some(format);
        apply_quality(&mut doc, &opts.quality);
        Ok(doc)
    }

    /// Parses files in parallel on the current rayon pool; results keep input
    /// order and failures are isolated per file. Per D1 nothing inside a file
    /// is parallelized, so `page_parallel` is forced off.
    pub fn parse_many(&self, inputs: Vec<Input>, opts: &ParseOptions) -> Vec<Result<Document>> {
        self.parse_many_with_stats(inputs, opts)
            .into_iter()
            .map(|(r, _)| r)
            .collect()
    }

    /// [`Registry::parse_many`] with the statistics of each file; every file's `document` span is
    /// a child of the caller's current span.
    pub fn parse_many_with_stats(
        &self,
        inputs: Vec<Input>,
        opts: &ParseOptions,
    ) -> Vec<(Result<Document>, ParseStats)> {
        let mut opts = opts.clone();
        opts.page_parallel = false;
        let span = telemetry::Span::current();
        inputs
            .into_par_iter()
            .map(|i| span.in_scope(|| self.parse_with_stats(i, &opts)))
            .collect()
    }

    fn parser_for(&self, format: Format) -> Option<&Arc<dyn Parser>> {
        self.parsers
            .iter()
            .rev()
            .find(|p| p.formats().contains(&format))
    }
}

fn catch_internal<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|payload| {
        let msg = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_string());
        Err(ParseError::Internal(msg))
    })
}

/// A path that is not a regular file (FIFO, `/dev/stdin`, `<(...)`) cannot be read
/// twice (detect, then parse) and has no size: read it once, bounded by `max_size`
/// (or `max_text_bytes` when unset), and continue with the bytes.
fn buffer_non_regular(input: Input, opts: &ParseOptions) -> Result<Input> {
    let Input::Path(p) = &input else {
        return Ok(input);
    };
    if std::fs::metadata(p)?.is_file() {
        return Ok(input);
    }
    let limit = opts.max_size.unwrap_or(opts.max_text_bytes);
    let mut buf = Vec::new();
    std::fs::File::open(p)?
        .take(limit.saturating_add(1))
        .read_to_end(&mut buf)?;
    if buf.len() as u64 > limit {
        return Err(ParseError::TooLarge {
            size: buf.len() as u64,
            limit,
        });
    }
    Ok(Input::Bytes(buf))
}

fn read_head(input: &Input) -> Result<Vec<u8>> {
    match input {
        Input::Path(p) => {
            let mut buf = Vec::with_capacity(DETECT_HEAD_LEN);
            std::fs::File::open(p)?
                .take(DETECT_HEAD_LEN as u64)
                .read_to_end(&mut buf)?;
            Ok(buf)
        }
        Input::Bytes(b) => Ok(b[..b.len().min(DETECT_HEAD_LEN)].to_vec()),
        _ => Err(ParseError::Unsupported(Format::Unknown)), // Input is non_exhaustive
    }
}

fn input_size(input: &Input) -> Result<u64> {
    Ok(match input {
        Input::Path(p) => std::fs::metadata(p)?.len(),
        Input::Bytes(b) => b.len() as u64,
        _ => 0,
    })
}

fn check_size(size: u64, max: Option<u64>) -> Result<()> {
    match max {
        Some(limit) if size > limit => Err(ParseError::TooLarge { size, limit }),
        _ => Ok(()),
    }
}
