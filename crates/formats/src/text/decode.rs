use encoding_rs::{Encoding, UTF_8};
use plaindoc_core::{Level, Warning, WarningKind};

/// Result of decoding raw bytes into text.
pub struct Decoded {
    pub text: String,
    pub encoding: &'static str,
    pub warnings: Vec<Warning>,
}

/// BOM, then strict UTF-8, then chardetng guess.
pub fn decode(bytes: &[u8]) -> Decoded {
    decode_with(bytes, None)
}

/// Like [`decode`], but a declared encoding (HTML `<meta charset>`, XML
/// `encoding=`) is used after the BOM. A declared UTF-8 that does not validate
/// falls back to detection. Callers pass an output encoding: map UTF-16 labels
/// to UTF-8 and ignore `replacement` themselves.
pub fn decode_with(bytes: &[u8], declared: Option<&'static Encoding>) -> Decoded {
    let utf8 = || std::str::from_utf8(bytes).is_ok();
    let (enc, body) = match Encoding::for_bom(bytes) {
        Some((enc, bom_len)) => (enc, &bytes[bom_len..]),
        None => match declared.filter(|d| *d != UTF_8 || utf8()) {
            Some(d) => (d, bytes),
            None if utf8() => (UTF_8, bytes),
            None => {
                let mut det = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
                det.feed(bytes, true);
                (det.guess(None, chardetng::Utf8Detection::Allow), bytes)
            }
        },
    };
    let (text, had_errors) = enc.decode_without_bom_handling(body);
    let text = text.into_owned();
    let mut warnings = Vec::new();
    if had_errors {
        warnings.push(Warning::new(
            WarningKind::SuspectedGarbage,
            Level::Warn,
            format!("invalid byte sequences for {}", enc.name()),
        ));
    }
    let total = text.chars().count();
    let control = text
        .chars()
        .filter(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\x0c' | '\x1b'))
        .count();
    if control * 10 > total {
        warnings.push(Warning::new(
            WarningKind::SuspectedGarbage,
            Level::Loss,
            format!("{control} of {total} characters are control characters; binary data?"),
        ));
    }
    if text.contains('\0') {
        warnings.push(Warning::new(
            WarningKind::SuspectedGarbage,
            Level::Loss,
            "text contains NUL characters; binary data?",
        ));
    }
    Decoded {
        text,
        encoding: enc.name(),
        warnings,
    }
}
