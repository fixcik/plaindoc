//! Declared encoding of an HTML or XML prolog (`<meta charset>`, `<?xml encoding>`).

use encoding_rs::{Encoding, REPLACEMENT};

/// Encoding named in the first 1024 bytes by the nearest `charset=` or `encoding=`.
/// Mapped to its output encoding (UTF-16 labels yield UTF-8: a prolog readable as ASCII
/// is not UTF-16); `replacement` labels (iso-2022-kr, hz-gb-2312, ...) are no declaration.
pub(crate) fn declared_encoding(bytes: &[u8]) -> Option<&'static Encoding> {
    let head = bytes[..bytes.len().min(1024)].to_ascii_lowercase();
    let at = ["charset=", "encoding="]
        .iter()
        .filter_map(|key| {
            head.windows(key.len())
                .position(|w| w == key.as_bytes())
                .map(|i| i + key.len())
        })
        .min()?;
    let rest = &head[at..];
    let start = rest
        .iter()
        .position(|b| !matches!(b, b'"' | b'\'' | b' ' | b'\t'))?;
    let label: Vec<u8> = rest[start..]
        .iter()
        .copied()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
        .collect();
    let enc = Encoding::for_label(&label)?;
    (enc != REPLACEMENT).then(|| enc.output_encoding())
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::{WINDOWS_1251, WINDOWS_1252};

    #[test]
    fn meta_and_xml_declarations() {
        let enc = |s: &str| declared_encoding(s.as_bytes()).map(|e| e.name());
        assert_eq!(
            enc("<meta charset=\"windows-1251\">"),
            Some(WINDOWS_1251.name())
        );
        assert_eq!(
            enc("<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=cp1251\">"),
            Some(WINDOWS_1251.name())
        );
        assert_eq!(
            enc("<?xml version=\"1.0\" encoding='ISO-8859-1'?>"),
            Some(WINDOWS_1252.name())
        );
        assert_eq!(enc("<meta charset=utf-16>"), Some("UTF-8"));
        assert_eq!(
            enc("<?xml version=\"1.0\" encoding=\"UTF-16BE\"?>"),
            Some("UTF-8")
        );
        assert_eq!(enc("<meta charset=iso-2022-kr>"), None);
        assert_eq!(enc("<meta charset=hz-gb-2312>"), None);
        assert_eq!(enc("<meta charset=replacement>"), None);
        assert_eq!(enc("<meta charset=bogus>"), None);
        assert_eq!(enc("<p>no declaration</p>"), None);
        let late = format!("{}<meta charset=windows-1251>", " ".repeat(1100));
        assert_eq!(enc(&late), None);
    }
}
