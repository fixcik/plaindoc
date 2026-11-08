//! Format detection by magic bytes, with the extension as a hint.

use crate::{Format, ParseError, Result};

/// How many leading bytes callers should pass to [`detect`].
pub const DETECT_HEAD_LEN: usize = 64 * 1024;

/// Signature of a CFB (OLE2) container: doc/xls/ppt/msg and encrypted OOXML.
pub const CFB_MAGIC: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Detects the format from the first bytes of a file.
///
/// Returns `Err(Encrypted)`, `Err(Unsupported(Msg | Unknown))` or `Err(Corrupt(..))`
/// when no supported format can be determined. Text is decided by content (D8);
/// HTML disguised as xls/xlsx yields `Html` (D6).
pub fn detect(head: &[u8], ext: Option<&str>) -> Result<Format> {
    let by_ext = ext.and_then(Format::from_extension);

    if contains(&head[..head.len().min(1024)], b"%PDF") {
        return Ok(Format::Pdf);
    }

    if head.starts_with(b"PK\x03\x04") {
        if contains(head, b"mimetypeapplication/vnd.oasis.opendocument.text") {
            return Ok(Format::Odt);
        }
        if contains(
            head,
            b"mimetypeapplication/vnd.oasis.opendocument.spreadsheet",
        ) {
            return Ok(Format::Ods);
        }
        if contains(head, b"[Content_Types].xml") {
            if contains(head, b"word/") {
                return Ok(Format::Docx);
            }
            if contains(head, b"xl/") {
                return Ok(Format::Xlsx);
            }
            if contains(head, b"ppt/") {
                return Ok(Format::Pptx);
            }
        }
        return match by_ext {
            Some(f @ (Format::Docx | Format::Xlsx | Format::Pptx | Format::Odt | Format::Ods)) => {
                Ok(f)
            }
            _ => Err(ParseError::Unsupported(Format::Unknown)),
        };
    }

    if head.starts_with(CFB_MAGIC) {
        return match by_ext {
            Some(f @ (Format::Doc | Format::Xls | Format::Ppt)) => Ok(f),
            Some(Format::Docx | Format::Xlsx | Format::Pptx) => Err(ParseError::Encrypted),
            Some(Format::Msg) => Err(ParseError::Unsupported(Format::Msg)),
            _ => Err(ParseError::Unsupported(Format::Unknown)),
        };
    }

    if head.starts_with(b"\x89PNG") {
        return Ok(Format::Png);
    }
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok(Format::Jpeg);
    }
    if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        return Ok(Format::Tiff);
    }
    if head.starts_with(b"{\\rtf") {
        return Ok(Format::Rtf);
    }
    if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
        return Ok(Format::Webp);
    }
    // "BM" also starts plain text ("BMW,2020,..."): only binary with a valid header is a BMP (I2).
    if !looks_like_text(head) && is_bmp(head) {
        return Ok(Format::Bmp);
    }

    // К-25: XML Spreadsheet 2003 saved as .xls is XML, not a broken binary.
    if matches!(by_ext, Some(Format::Xls | Format::Xlsx)) {
        if trimmed_lower(head).starts_with("<?xml") {
            return resolve_text(head, by_ext);
        }
        if looks_like_html(head) {
            return Ok(Format::Html);
        }
    }
    if let (Some(f), Some(e)) = (by_ext, ext)
        && !matches!(
            f,
            Format::Csv | Format::Tsv | Format::Txt | Format::Md | Format::Html | Format::Xml
        )
    {
        return Err(ParseError::Corrupt(format!(
            "content does not match .{e} signature"
        )));
    }
    resolve_text(head, by_ext)
}

/// Single place of the text-format policy (D8).
fn resolve_text(head: &[u8], by_ext: Option<Format>) -> Result<Format> {
    if !looks_like_text(head) {
        return Err(ParseError::Unsupported(Format::Unknown));
    }
    let s = trimmed_lower(head);
    if s.starts_with("<?xml") {
        // XHTML stays Html; any other XML passes through as-is.
        let xhtml = s.contains("<html") || s.contains("<!doctype html");
        return Ok(if xhtml { Format::Html } else { Format::Xml });
    }
    // For md/txt/csv/tsv the extension wins over a bare `<table`/`<p`: only a real
    // `<html`/`<!doctype html` makes such a file Html (READMEs with HTML headers).
    let text_hint = matches!(
        by_ext,
        Some(Format::Csv | Format::Tsv | Format::Md | Format::Txt)
    );
    let html = if text_hint {
        s.contains("<html") || s.contains("<!doctype html")
    } else {
        looks_like_html(head)
    };
    if html {
        return Ok(Format::Html);
    }
    Ok(match by_ext {
        Some(f @ (Format::Csv | Format::Tsv | Format::Md)) => f,
        _ => Format::Txt,
    })
}

/// BMP file header plus a known DIB header size, with the pixel offset after the headers.
fn is_bmp(head: &[u8]) -> bool {
    let u32le = |i: usize| u32::from_le_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]);
    head.len() >= 18
        && head.starts_with(b"BM")
        && head[6..10] == [0; 4] // bfReserved1/2
        && matches!(u32le(14), 12 | 40 | 52 | 56 | 108 | 124) // DIB header size
        && (14 + u32le(14)) <= u32le(10) // bfOffBits after the headers
        && (u32le(2) == 0 || u32le(10) < u32le(2)) // ... and inside bfSize
}

fn looks_like_text(head: &[u8]) -> bool {
    if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) {
        return true;
    }
    if head.contains(&0) {
        return false;
    }
    let control = head
        .iter()
        .filter(|&&b| b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0c | 0x1b))
        .count();
    control * 10 <= head.len()
}

fn trimmed_lower(head: &[u8]) -> String {
    let head = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    let head = &head[..head.len().min(1024)];
    String::from_utf8_lossy(head).trim_start().to_lowercase()
}

fn looks_like_html(head: &[u8]) -> bool {
    let s = trimmed_lower(head);
    s.starts_with('<')
        && ["<html", "<!doctype html", "<table"]
            .iter()
            .any(|m| s.contains(m))
}

/// Classifies a CFB container by its stream names (case-insensitive).
pub fn classify_cfb_streams(names: &[&str]) -> Result<Format> {
    let has = |n: &str| names.iter().any(|s| s.eq_ignore_ascii_case(n));
    if has("EncryptionInfo") && has("EncryptedPackage") {
        Err(ParseError::Encrypted)
    } else if has("WordDocument") {
        Ok(Format::Doc)
    } else if has("Workbook") || has("Book") {
        Ok(Format::Xls)
    } else if has("PowerPoint Document") {
        Ok(Format::Ppt)
    } else if has("__properties_version1.0") {
        Err(ParseError::Unsupported(Format::Msg))
    } else {
        Err(ParseError::Unsupported(Format::Unknown))
    }
}
