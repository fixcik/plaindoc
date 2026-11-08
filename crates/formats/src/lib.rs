//! Format parsers and the Registry.

#[cfg(feature = "text")]
pub mod text;

#[cfg(feature = "ocr")]
pub mod ocr;
#[cfg(feature = "office")]
pub mod office;
#[cfg(feature = "pdf")]
pub mod pdf;
#[cfg(feature = "tables")]
pub mod tables;

#[cfg(any(feature = "text", feature = "rtf"))]
mod read;

#[cfg(any(feature = "markup", feature = "odf", feature = "rtf"))]
mod common;
#[cfg(any(feature = "markup", feature = "odf", feature = "rtf"))]
mod picture_data;

#[cfg(feature = "markup")]
pub mod markup;

#[cfg(feature = "odf")]
pub mod odf;

#[cfg(feature = "rtf")]
pub mod rtf;

mod registry;
pub use registry::{CfbProbe, Registry};

use plaindoc_core::Parser;
use std::sync::Arc;

/// Parsers enabled by cargo features. Each subproject adds its own `#[cfg]` block here.
// A single-feature build pushes once right after creation.
#[allow(clippy::vec_init_then_push)]
fn default_parsers() -> Vec<Arc<dyn Parser>> {
    #[allow(unused_mut)]
    let mut v: Vec<Arc<dyn Parser>> = Vec::new();
    #[cfg(feature = "text")]
    {
        v.push(Arc::new(text::PlainTextParser));
        v.push(Arc::new(text::CsvParser));
    }
    // One default engine (one session pool) shared by every OCR-capable parser. Without an engine
    // (no `ocr-paddle`) each image gives `OcrUnavailable`.
    #[cfg(feature = "ocr")]
    let ocr_config = ocr::OcrConfig::default_engine();
    #[cfg(feature = "pdf")]
    {
        #[cfg(feature = "ocr")]
        v.push(Arc::new(pdf::PdfParser::with_ocr(ocr_config.clone())));
        #[cfg(not(feature = "ocr"))]
        v.push(Arc::new(pdf::PdfParser::new()));
    }
    #[cfg(feature = "ocr")]
    v.push(Arc::new(ocr::ImageParser::new(ocr_config.clone())));
    #[cfg(feature = "office")]
    {
        #[cfg(feature = "ocr")]
        v.push(Arc::new(office::OfficeParser::with_ocr(ocr_config.clone())));
        #[cfg(not(feature = "ocr"))]
        v.push(Arc::new(office::OfficeParser::new()));
    }
    #[cfg(feature = "markup")]
    {
        v.push(Arc::new(markup::HtmlParser));
        v.push(Arc::new(markup::XmlParser));
    }
    #[cfg(feature = "odf")]
    v.push(Arc::new(odf::OdfParser));
    #[cfg(feature = "rtf")]
    v.push(Arc::new(rtf::RtfParser));
    v
}
