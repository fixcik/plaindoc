//! Decoding of pictures written inline as text: base64 (HTML data URIs, ODF `office:binary-data`)
//! and hex (RTF `\pict`). The size limit is checked on the encoded length before decoding.

use plaindoc_core::ImageInfo;

/// Base64 (whitespace ignored, padding optional) into a pending picture; `too_large` past `max`
/// decoded bytes, `bad_data` when it does not decode.
#[cfg(any(feature = "markup", feature = "odf"))]
pub(crate) fn from_base64(text: &str, max: u64) -> ImageInfo {
    use base64::Engine;
    let clean: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let size = (clean.len() as u64 / 4) * 3;
    if size > max.saturating_add(3) {
        return ImageInfo::skipped("too_large", Some(size));
    }
    match base64::engine::general_purpose::STANDARD_PAD_INDIFFERENT.decode(&clean) {
        Ok(b) if b.is_empty() => ImageInfo::skipped("no_data", None),
        Ok(b) => ImageInfo::embedded(b, None),
        Err(_) => ImageInfo::skipped("bad_data", None),
    }
}

/// Hex digits (anything else ignored, as RTF readers do) into bytes; `None` past `max` bytes.
#[cfg(feature = "rtf")]
pub(crate) fn from_hex(text: &[u8], max: u64) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 2);
    let mut hi: Option<u8> = None;
    for &c in text {
        let v = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => continue,
        };
        match hi.take() {
            Some(h) => {
                if out.len() as u64 >= max {
                    return None;
                }
                out.push(h << 4 | v);
            }
            None => hi = Some(v),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(feature = "markup", feature = "odf"))]
    #[test]
    fn base64_with_whitespace_and_limits() {
        let i = from_base64("aGVs\n bG8=", 100);
        assert_eq!(i.pending_data(), Some(&b"hello"[..]));
        assert_eq!(
            from_base64("aGVsbG8", 100).pending_data(),
            Some(&b"hello"[..])
        );
        assert_eq!(from_base64("aGVsbG8=", 1).skipped, Some("too_large"));
        assert_eq!(from_base64("a$b", 100).skipped, Some("bad_data"));
        assert_eq!(from_base64("", 100).skipped, Some("no_data"));
    }

    #[cfg(feature = "rtf")]
    #[test]
    fn hex_skips_noise_and_caps() {
        assert_eq!(from_hex(b"89 50\r\n4e47", 10).unwrap(), b"\x89PNG");
        assert_eq!(from_hex(b"0102", 1), None);
    }
}
