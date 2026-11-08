//! A document's result must not depend on what the process parsed before it.
//!
//! pdf_oxide keeps a process-wide font cache (`GLOBAL_FONT_CACHE`) shared by
//! every `PdfDocument`; a key that misses a field `FontInfo` depends on lets a
//! later document get an earlier document's parsed font. The reference is a
//! parse with that cache cleared ("fresh process"); the checked parse runs
//! after another document on the same thread. One `#[test]` per binary keeps
//! other tests from filling the cache concurrently.
mod pdf_common;
use pdf_common::*;
use plaindoc_core::*;
use std::path::PathBuf;

/// One page "Hello world" in a non-embedded TrueType `Times New Roman` whose
/// `/Widths` is an indirect object (as Word writes it). `zero_widths` zeroes
/// every advance except the space, like a producer that zeroes unused codes.
fn times_pdf(zero_widths: bool) -> Vec<u8> {
    let widths: String = (32..=122u8)
        .map(|c| {
            if c == b' ' {
                "250"
            } else if zero_widths {
                "0"
            } else {
                "500"
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let content = b"BT /F1 12 Tf 72 720 Td (Hello world) Tj ET";
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 4 0 R >> >> /Contents 7 0 R >>"
            .to_string(),
        "<< /Type /Font /Subtype /TrueType /BaseFont /Times#20New#20Roman \
         /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 122 \
         /Widths 5 0 R /FontDescriptor 6 0 R >>"
            .to_string(),
        format!("[{widths}]"),
        "<< /Type /FontDescriptor /FontName /Times#20New#20Roman /Flags 34 \
         /FontBBox [-568 -216 2046 693] /ItalicAngle 0 /Ascent 891 /Descent -216 \
         /CapHeight 693 /StemV 80 >>"
            .to_string(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            std::str::from_utf8(content).unwrap()
        ),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn json(b: &[u8]) -> String {
    parse_bytes(b.to_vec(), &ParseOptions::default())
        .map(|d| d.to_json())
        .unwrap_or_else(|e| format!("error: {e}"))
}

/// `b` parsed with an empty process cache, on a thread of its own.
fn fresh(b: &[u8]) -> String {
    pdf_oxide::clear_global_font_cache();
    let b = b.to_vec();
    std::thread::spawn(move || json(&b)).join().unwrap()
}

/// `b` parsed right after `a` on one thread.
fn after(a: &[u8], b: &[u8]) -> String {
    pdf_oxide::clear_global_font_cache();
    let (a, b) = (a.to_vec(), b.to_vec());
    std::thread::spawn(move || {
        json(&a);
        json(&b)
    })
    .join()
    .unwrap()
}

/// Extra files from `DP_CROSS_DOC_FILES` (newline-separated paths), e.g. the
/// wild corpus: `r2cm_02.pdf` then `r2ia_46-55.pdf` reproduced the leak.
fn extra_files() -> Vec<Vec<u8>> {
    std::env::var("DP_CROSS_DOC_FILES")
        .map(|s| {
            s.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| std::fs::read(PathBuf::from(l.trim())).unwrap())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn document_after_another_equals_document_on_fresh_process() {
    let mut files = vec![times_pdf(true), times_pdf(false)];
    for name in ["ru.pdf", "notes.pdf", "imgtext.pdf"] {
        files.push(std::fs::read(fixture(name)).unwrap());
    }
    files.extend(extra_files());
    let refs: Vec<String> = files.iter().map(|f| fresh(f)).collect();
    let mut bad = Vec::new();
    for (i, a) in files.iter().enumerate() {
        for (j, b) in files.iter().enumerate() {
            if i != j && after(a, b) != refs[j] {
                bad.push((i, j));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "(A, B) pairs where B after A differs from B alone: {bad:?}"
    );
}
