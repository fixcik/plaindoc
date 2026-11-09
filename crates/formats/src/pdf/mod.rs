//! PDF parser built on the `pdf_oxide` fork.

mod bullets;
mod columns;
mod diagnose;
mod fields;
mod form_labels;
mod garbage;
mod heading_rank;
mod headings;
mod headings_number;
mod headings_style;
mod hidden;
#[cfg(feature = "ocr")]
mod hidden_ocr;
mod hidden_turned;
mod layout;
mod links;
mod list_marker;
mod lists;
mod need_ocr;
mod ocg;
#[cfg(feature = "ocr")]
mod ocr_stage;
mod open;
mod order;
mod outline;
mod pictures;
mod pipeline;
mod runner;
mod running;
mod spacing;
mod table_input;
mod table_place;
mod tables;

pub use diagnose::DiagnosticsConfig;
pub use layout::{LayoutConfig, ReadingOrderMode};
pub use need_ocr::{Bigrams, NeedOcrConfig};
#[cfg(feature = "ocr")]
#[doc(hidden)]
pub use need_ocr::{EvalPage, eval_page};
#[doc(hidden)]
pub use order::{OrderBox, xy_cut};
#[doc(hidden)]
pub use tables::WordFilter;
pub use tables::{
    DetectedTable, NoTables, OwnTables, ScanSegment, ScanTableInputs, ScanWord, TableDetector,
    TableInputCounts,
};

use open::open_input;
use pipeline::parse_opened;
use plaindoc_core::{
    Document, Format, Input, ParseError, ParseOptions, ParseStats, Parser, Result, Stage,
};
use runner::run_isolated;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// PDF parser settings.
#[non_exhaustive]
#[derive(Clone)]
pub struct PdfConfig {
    /// Minimum page count to process pages in parallel.
    pub min_parallel_pages: usize,
    /// Font-size heading detection settings.
    pub layout: LayoutConfig,
    /// Stage-1 page diagnostics thresholds.
    pub diagnostics: DiagnosticsConfig,
    /// Table detector hook; defaults to [`OwnTables`].
    pub tables: Arc<dyn TableDetector>,
    /// `PdfDocument::set_char_x_offset_refinement`. SP2 turned it off (~10% faster); SP4 keeps it on:
    /// without it `extract_words` glues words on some PDFs (PubTables-1M: 47% of table-free pages got
    /// false tables).
    pub char_x_refinement: bool,
    /// `PdfDocument::set_visibility_facts` (render mode, alpha, clip, paints) for the hidden-text
    /// rules that need them; only with `ParseOptions::hidden_text` other than `Off`. Off by
    /// default: the fork then reads content through its full operator parser and keeps runs with
    /// different facts apart, which costs time and changes the text of every PDF slightly.
    pub visibility_facts: bool,
    /// OCR engine and cascade settings; `engine: None` = OCR unavailable.
    #[cfg(feature = "ocr")]
    pub ocr: crate::ocr::OcrConfig,
}

impl Default for PdfConfig {
    fn default() -> Self {
        Self {
            min_parallel_pages: 32,
            layout: LayoutConfig::default(),
            diagnostics: DiagnosticsConfig::default(),
            tables: Arc::new(OwnTables::default()),
            char_x_refinement: true,
            visibility_facts: false,
            #[cfg(feature = "ocr")]
            ocr: crate::ocr::OcrConfig::default(),
        }
    }
}

/// PDF parser: runs each file on a dedicated big-stack thread pool (`PDF_POOL`).
///
/// `ParseOptions::timeout` is cooperative: on timeout the caller gets `Timeout` at once, but the
/// work stops only at the next page boundary. A page stuck inside one `pdf_oxide` call keeps its
/// `PDF_POOL` thread after the timeout, so once as many files have hung as the pool has threads,
/// later parses queue (and with `timeout = None` block). Long-running services should set a
/// timeout.
///
/// `parse_with_stats` (through `Parser` or `Registry`) also fills `ParseStats` with stage times,
/// page counts and sizes. If the timeout fires, the stage statistics are lost together with the
/// detached work. A direct `PdfParser::parse_with_stats` returns `Err(Timeout)` and leaves `stats`
/// untouched; through `Registry::parse_with_stats` the stats carry only `format`, `input_bytes`,
/// `wall_us`, `outcome: timeout` and the `document_timeout` limit.
pub struct PdfParser {
    config: PdfConfig,
}

impl PdfParser {
    /// Parser with [`PdfConfig::default`]; see [`PdfParser`] for timeout behaviour.
    pub fn new() -> Self {
        Self::with_config(PdfConfig::default())
    }

    /// Parser with a custom config; see [`PdfParser`] for timeout behaviour.
    pub fn with_config(config: PdfConfig) -> Self {
        Self { config }
    }

    /// Parser with the default config and `ocr` settings.
    #[cfg(feature = "ocr")]
    pub(crate) fn with_ocr(ocr: crate::ocr::OcrConfig) -> Self {
        Self::with_config(PdfConfig {
            ocr,
            ..Default::default()
        })
    }
}

impl Default for PdfParser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser for PdfParser {
    fn formats(&self) -> &[Format] {
        &[Format::Pdf]
    }

    fn parse(&self, input: Input, format: Format, opts: &ParseOptions) -> Result<Document> {
        self.parse_with_stats(input, format, opts, &mut ParseStats::default())
    }

    fn parse_with_stats(
        &self,
        input: Input,
        format: Format,
        opts: &ParseOptions,
        stats: &mut ParseStats,
    ) -> Result<Document> {
        if format != Format::Pdf {
            return Err(ParseError::Unsupported(format));
        }
        let (cfg, opts) = (self.config.clone(), opts.clone());
        // `run_isolated` carries the caller's span over to PDF_POOL.
        let (doc, s) = run_isolated(opts.timeout, move |ctx| {
            // Timed out while queued: do not open the document (same convention as per-page check).
            if ctx.cancel.load(Ordering::Relaxed) {
                return Err(ParseError::Timeout(Duration::ZERO));
            }
            let (opened, open) = pipeline::timed(Stage::Open, || open_input(input, &opts));
            let opened = opened?;
            let mut p = parse_opened(opened, &opts, &cfg, ctx)?;
            p.stats.add_stage(Stage::Open, open);
            Ok((p.document, p.stats))
        })?;
        *stats = s;
        Ok(doc)
    }
}

/// Bench-only hook (`benches/ocr.rs`): renders page 0 of `bytes` at `dpi` `iters` times inside
/// `run_isolated` (the document is opened once, outside the timing) and returns the render time.
#[cfg(feature = "ocr")]
#[doc(hidden)]
pub fn bench_render_page(bytes: Vec<u8>, dpi: u32, iters: u64) -> Result<Duration> {
    run_isolated(None, move |_| {
        let doc = pdf_oxide::PdfDocument::from_bytes(bytes)
            .map_err(|e| ParseError::Corrupt(e.to_string()))?;
        let t = std::time::Instant::now();
        for _ in 0..iters {
            let r = ocr_stage::render(&doc, 0, dpi).map_err(ParseError::Corrupt)?;
            std::hint::black_box(&r.img);
        }
        Ok(t.elapsed())
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pdf_oxide::writer::DocumentBuilder;

    pub(crate) fn one_page_pdf(text: &str) -> Vec<u8> {
        let mut b = DocumentBuilder::new();
        b.letter_page()
            .font("Helvetica", 12.0)
            .at(72.0, 720.0)
            .paragraph(text)
            .done();
        b.build().unwrap()
    }

    /// One-page PDF with the raw `content` stream and Helvetica as `/F1`.
    pub(crate) fn raw_pdf(media: &str, content: &str) -> Vec<u8> {
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [{media}] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            ),
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
                .to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offs = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offs.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let x = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
        for o in offs {
            out.extend(format!("{o:010} 00000 n \n").bytes());
        }
        out.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n",
                objs.len() + 1
            )
            .bytes(),
        );
        out
    }

    /// One page with a 3×4 ruled table (header Name/Qty/Price) under a paragraph.
    pub(crate) fn ruled_table_pdf() -> Vec<u8> {
        use pdf_oxide::elements::{ContentElement, PathContent};
        use pdf_oxide::layout::Color;
        let (xs, ys) = (
            [72.0f32, 200.0, 328.0, 456.0],
            [640.0f32, 620.0, 600.0, 580.0, 560.0],
        );
        let mut b = DocumentBuilder::new();
        let mut page = b
            .letter_page()
            .font("Helvetica", 11.0)
            .at(72.0, 700.0)
            .paragraph("Quarterly figures are listed below.");
        for &y in &ys {
            page = page.element(ContentElement::Path(
                PathContent::line(xs[0], y, xs[3], y).with_stroke(Color::black()),
            ));
        }
        for &x in &xs {
            page = page.element(ContentElement::Path(
                PathContent::line(x, ys[4], x, ys[0]).with_stroke(Color::black()),
            ));
        }
        let cells = [
            ["Name", "Qty", "Price"],
            ["Apples", "12", "3.50"],
            ["Pears", "7", "4.10"],
            ["Plums", "30", "1.25"],
        ];
        for (r, row) in cells.iter().enumerate() {
            for (c, t) in row.iter().enumerate() {
                page = page.at(xs[c] + 6.0, ys[r] - 14.0).text(t);
            }
        }
        page.done();
        b.build().unwrap()
    }

    /// `heading(1, ..)` switches to Helvetica-Bold 24 pt and later paragraphs inherit it: reset to 12 pt.
    pub(crate) fn builder_pdf_bytes(pages: usize) -> Vec<u8> {
        let mut b = DocumentBuilder::new();
        for i in 0..pages {
            b.letter_page()
                .font("Helvetica", 12.0)
                .at(72.0, 720.0)
                .heading(1, &format!("Chapter {i}"))
                .font("Helvetica", 12.0)
                .at(72.0, 690.0)
                .paragraph("First paragraph of body text on this page.")
                .at(72.0, 660.0)
                .paragraph("Second paragraph with a few more words in it.")
                .done();
        }
        b.build().unwrap()
    }

    #[test]
    fn pdf_parser_claims_pdf_only() {
        assert_eq!(PdfParser::new().formats(), &[Format::Pdf]);
        let r = PdfParser::new().parse(
            Input::Bytes(one_page_pdf("x")),
            Format::Txt,
            &ParseOptions::default(),
        );
        assert!(matches!(r, Err(ParseError::Unsupported(Format::Txt))));
    }

    #[test]
    fn parse_with_stats_adds_open_and_equals_parse() {
        let mut s = ParseStats::default();
        let a = PdfParser::new()
            .parse_with_stats(
                Input::Bytes(builder_pdf_bytes(2)),
                Format::Pdf,
                &ParseOptions::default(),
                &mut s,
            )
            .unwrap();
        let b = PdfParser::new()
            .parse(
                Input::Bytes(builder_pdf_bytes(2)),
                Format::Pdf,
                &ParseOptions::default(),
            )
            .unwrap();
        assert_eq!(a.to_json(), b.to_json());
        assert!(s.stages_us.contains_key(&Stage::Open));
    }

    #[test]
    fn default_config_threshold() {
        assert_eq!(PdfConfig::default().min_parallel_pages, 32);
    }

    #[test]
    fn default_config_keeps_char_refinement_for_tables() {
        // Off, extract_words glues words on some PDFs (PubTables-1M: false tables on 47% of pages).
        assert!(PdfConfig::default().char_x_refinement);
    }
}
