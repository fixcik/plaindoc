//! Helpers shared by the PDF integration tests (`pdf_corpus`, `pdf_odl`).
#![allow(dead_code)]

use pdf_oxide::editor::{EncryptionAlgorithm, EncryptionConfig, Permissions};
use pdf_oxide::writer::DocumentBuilder;
use plaindoc_core::*;
use plaindoc_formats::Registry;
use plaindoc_formats::pdf::{NeedOcrConfig, PdfConfig, PdfParser};
use std::path::{Path, PathBuf};

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pdf")
        .join(name)
}

/// Through the Registry: checks registration and detection.
pub fn parse_path(name: &str) -> Result<Document> {
    parse_path_abs(&fixture(name))
}

pub fn parse_path_abs(p: &Path) -> Result<Document> {
    Registry::with_defaults().parse(Input::Path(p.to_path_buf()), &ParseOptions::default())
}

/// The default `PdfConfig`: need-OCR classifier on (`RECALL`, bundled bigram table).
pub fn need_ocr_on() -> PdfConfig {
    let c = PdfConfig::default();
    assert_eq!(c.diagnostics.need_ocr, Some(NeedOcrConfig::RECALL));
    c
}

/// `PdfConfig` with the classifier switched off: the SP2 stage-1 rules table.
pub fn sp2_off() -> PdfConfig {
    let mut c = PdfConfig::default();
    c.diagnostics.need_ocr = None;
    c
}

/// Directly with `cfg`, default options.
pub fn parse_with(cfg: &PdfConfig, p: &Path) -> Result<Document> {
    PdfParser::with_config(cfg.clone()).parse(
        Input::Path(p.to_path_buf()),
        Format::Pdf,
        &ParseOptions::default(),
    )
}

/// Directly: bytes have no extension, and SP1 detect classifies 0 bytes as text.
pub fn parse_bytes(b: Vec<u8>, opts: &ParseOptions) -> Result<Document> {
    PdfParser::new().parse(Input::Bytes(b), Format::Pdf, opts)
}

/// `None` = document warning, `Some(index)` = page warning.
pub fn levels(d: &Document) -> Vec<(Option<usize>, WarningKind, Level)> {
    let doc = d.warnings.iter().map(|w| (None, w.kind, w.level));
    let pages = d
        .sections
        .iter()
        .flat_map(|s| s.warnings.iter().map(|w| (Some(s.index), w.kind, w.level)));
    doc.chain(pages).collect()
}

/// `ru.pdf[..3000]`.
pub fn truncated_bytes() -> Vec<u8> {
    let mut b = std::fs::read(fixture("ru.pdf")).unwrap();
    b.truncate(3000);
    b
}

/// Truncated bytes with an `%%EOF` marker appended.
pub fn mojibake_bytes() -> Vec<u8> {
    let mut b = truncated_bytes();
    b.extend_from_slice(b"\n%%EOF\n");
    b
}

/// AES-256, owner password `"owner"`, one page "Secret body text.".
pub fn encrypted_bytes(user: &str) -> Vec<u8> {
    let mut b = DocumentBuilder::new();
    b.letter_page()
        .font("Helvetica", 12.0)
        .at(72.0, 720.0)
        .paragraph("Secret body text.")
        .done();
    b.to_bytes_with_encryption(EncryptionConfig {
        user_password: user.into(),
        owner_password: "owner".into(),
        algorithm: EncryptionAlgorithm::Aes256,
        permissions: Permissions::all(),
    })
    .unwrap()
}

/// `heading(1, ..)` switches to Helvetica-Bold 24 pt and later paragraphs inherit it: reset to 12 pt.
pub fn builder_pdf_bytes(pages: usize) -> Vec<u8> {
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

/// Text of a `Heading`, `Paragraph` or `Note`.
pub fn block_text(b: &Block) -> Option<&str> {
    match b {
        Block::Heading { text, .. } | Block::Paragraph { text, .. } | Block::Note { text, .. } => {
            Some(text)
        }
        _ => None,
    }
}

/// Block texts; a table contributes its cell texts in row order (SP4), a list its items' texts
/// (SP8b S1).
pub fn texts(s: &Section) -> Vec<String> {
    blocks_texts(&s.blocks)
}

fn blocks_texts(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .filter_map(|b| match b {
            Block::Table { rows, .. } => Some(
                rows.iter()
                    .flatten()
                    .flat_map(|c| c.blocks.iter().filter_map(block_text))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            Block::List { items, .. } => Some(
                items
                    .iter()
                    .flat_map(|i| blocks_texts(&i.blocks))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => block_text(b).map(str::to_owned),
        })
        .collect()
}

/// All block texts joined by `'\n'`.
pub fn doc_text(d: &Document) -> String {
    d.sections
        .iter()
        .flat_map(texts)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn bbox_of(b: &Block) -> Option<&BBox> {
    match b {
        Block::Heading { bbox, .. }
        | Block::Paragraph { bbox, .. }
        | Block::List { bbox, .. }
        | Block::Table { bbox, .. }
        | Block::Image { bbox, .. }
        | Block::Code { bbox, .. }
        | Block::Note { bbox, .. } => bbox.as_ref(),
        _ => None,
    }
}

/// Sorted non-whitespace characters.
pub fn chars_of(s: &str) -> Vec<char> {
    let mut v: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
    v.sort_unstable();
    v
}

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Multiset word F1 = `2·|A∩B| / (|A|+|B|)`, 1.0 when both are empty.
pub fn word_f1(a: &str, b: &str) -> f64 {
    let (a, b) = (words(a), words(b));
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let mut counts = std::collections::HashMap::<&str, usize>::new();
    for w in &a {
        *counts.entry(w).or_default() += 1;
    }
    let mut common = 0;
    for w in &b {
        if let Some(n) = counts.get_mut(w.as_str())
            && *n > 0
        {
            *n -= 1;
            common += 1;
        }
    }
    2.0 * common as f64 / (a.len() + b.len()) as f64
}

/// `(ODL_BENCH_PDFS, ODL_BENCH_GT)`; panics when unset, no silent skip.
pub fn odl_dirs() -> (PathBuf, PathBuf) {
    match (
        std::env::var_os("ODL_BENCH_PDFS"),
        std::env::var_os("ODL_BENCH_GT"),
    ) {
        (Some(p), Some(g)) => (p.into(), g.into()),
        _ => panic!("set ODL_BENCH_PDFS and ODL_BENCH_GT (scripts/fetch-odl-bench.sh prints them)"),
    }
}

/// Sorted `*.pdf` files of `ODL_BENCH_PDFS`.
pub fn odl_files() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(odl_dirs().0)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "pdf"))
        .collect();
    v.sort();
    assert!(!v.is_empty(), "no *.pdf files in ODL_BENCH_PDFS");
    v
}
