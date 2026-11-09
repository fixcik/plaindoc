use crate::ImageOptions;
use std::time::Duration;

#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OcrMode {
    /// OCR where text is missing: scanned PDF pages, PDF pages without a usable text layer,
    /// office sections without text. Images on PDF pages with a text layer are not recognised:
    /// they become pictures (`Block::Image`) and the page gets an `ImageTextPage`/Info warning.
    #[default]
    Auto,
    /// OCR of every page and image, merged with the text layer.
    Always,
    /// No OCR.
    Never,
    /// As `Auto`, plus OCR of the images on PDF pages with a text layer that the need-OCR
    /// classifier flags (the `Auto` of earlier versions); their pictures are linked too. Other
    /// formats: as `Auto`.
    Images,
}

/// Text the reader cannot see (white on white, microscopic, off-page, hidden layers, invisible
/// Unicode): a prompt-injection channel when the output goes to an LLM.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HiddenText {
    /// No checks.
    Off,
    /// Keep the text, report it with a `HiddenContent` warning (`Warn`).
    Flag,
    /// Remove text caught by reliable rules (`HiddenContent`, `Info`); heuristics only flag.
    #[default]
    Strip,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QualityWeights {
    pub warn: f32,
    pub loss: f32,
}

impl Default for QualityWeights {
    fn default() -> Self {
        Self {
            warn: 0.1,
            loss: 0.5,
        }
    }
}

/// No `Debug`: holds the password.
#[non_exhaustive]
#[derive(Clone)]
pub struct ParseOptions {
    pub ocr: OcrMode,
    pub min_quality: Option<f32>,
    pub timeout: Option<Duration>,
    pub max_pages: Option<usize>,
    pub max_size: Option<u64>,
    pub password: Option<String>,
    pub libreoffice_fallback: bool,
    pub quality: QualityWeights,
    /// Page-level parallelism inside one document; `parse_many` sets false.
    pub page_parallel: bool,
    /// With `page_parallel` on: at most this many pages of one PDF are parsed at once (PDFs of
    /// at least `PdfConfig::min_parallel_pages` pages, 32 by default; smaller ones go page by
    /// page). `None`: the library default, min(cores, 6). `Some(0)` counts as `Some(1)`: pages
    /// one by one. The output does not depend on it. Pages that need OCR are bounded by the OCR
    /// engine's session count instead.
    pub page_threads: Option<usize>,
    /// Input larger than this is not treated as text and yields `ParseError::TooLarge`
    /// (compared against input size, not extracted text).
    pub max_text_bytes: u64,
    /// OCR at most this many pages of a document (`None`: no limit). The pages are the first ones
    /// in page order that need OCR; the rest keep their text layer and the document gets
    /// `PartialContent`/Warn.
    pub ocr_max_pages: Option<usize>,
    /// Recognition time budget per page or image (`None`: no limit). Past it, recognition stops
    /// between chunks and the page gets `PartialContent`/Warn with the lines read so far.
    pub ocr_page_timeout: Option<Duration>,
    /// Hidden-text policy (PDF).
    pub hidden_text: HiddenText,
    /// Hyperlinks from link annotations (PDF `/Link` with a `/URI` action, `http`, `https` and
    /// `mailto` only) into `Block::{Heading, Paragraph}::links`, which Markdown shows as
    /// `[text](url)`. Off: annotations are not read.
    pub links: bool,
    /// Pictures embedded in office, ODF, RTF and HTML documents: extracted into
    /// `Document::images` and linked from `Block::Image`, within limits.
    pub images: ImageOptions,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            ocr: OcrMode::Auto,
            min_quality: None,
            timeout: None,
            max_pages: None,
            max_size: None,
            password: None,
            libreoffice_fallback: false,
            quality: QualityWeights::default(),
            page_parallel: false,
            page_threads: None,
            max_text_bytes: 64 * 1024 * 1024,
            ocr_max_pages: None,
            ocr_page_timeout: None,
            hidden_text: HiddenText::default(),
            links: true,
            images: ImageOptions::default(),
        }
    }
}
