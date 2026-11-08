//! `<img src>`: a `data:` URI becomes the picture's bytes, an `http`/`https` or relative `src` stays
//! a link (nothing is downloaded), anything else is skipped.

use plaindoc_core::ImageInfo;

use crate::picture_data::from_base64;

/// The picture of `<img src=..>`; `max`: the largest picture decoded.
pub(super) fn image_info(src: Option<&str>, max: u64) -> Box<ImageInfo> {
    let src = src.map(str::trim).unwrap_or("");
    let info = if src.is_empty() {
        ImageInfo::skipped("no_data", None)
    } else if src
        .get(..5)
        .is_some_and(|h| h.eq_ignore_ascii_case("data:"))
    {
        data_uri(&src[5..], max)
    } else {
        ImageInfo::external(src)
    };
    Box::new(info)
}

/// `[mime][;param]*[;base64],payload`; a payload that is not base64 is percent-decoded.
fn data_uri(rest: &str, max: u64) -> ImageInfo {
    let Some((head, payload)) = rest.split_once(',') else {
        return ImageInfo::skipped("bad_data", None);
    };
    if head
        .rsplit(';')
        .next()
        .is_some_and(|p| p.trim().eq_ignore_ascii_case("base64"))
    {
        return from_base64(payload, max);
    }
    if payload.len() as u64 > max.saturating_mul(3) {
        return ImageInfo::skipped("too_large", None);
    }
    let bytes = percent_decode(payload.as_bytes());
    if bytes.len() as u64 > max {
        ImageInfo::skipped("too_large", Some(bytes.len() as u64))
    } else if bytes.is_empty() {
        ImageInfo::skipped("no_data", None)
    } else {
        ImageInfo::embedded(bytes, None)
    }
}

fn percent_decode(s: &[u8]) -> Vec<u8> {
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%'
            && let (Some(h), Some(l)) = (
                s.get(i + 1).copied().and_then(hex),
                s.get(i + 2).copied().and_then(hex),
            )
        {
            out.push(h << 4 | l);
            i += 3;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_uris_links_and_bad_sources() {
        let i = image_info(Some(" data:image/png;base64,aGVs bG8= "), 100);
        assert_eq!(i.pending_data(), Some(&b"hello"[..]));
        let s = image_info(Some("DATA:image/svg+xml,%3Csvg%2F%3E"), 100);
        assert_eq!(s.pending_data(), Some(&b"<svg/>"[..]));
        assert_eq!(image_info(Some("data:,"), 100).skipped, Some("no_data"));
        assert_eq!(
            image_info(Some("data:image/png"), 100).skipped,
            Some("bad_data")
        );
        assert_eq!(
            image_info(Some("data:image/png;base64,aGVsbG8="), 2).skipped,
            Some("too_large")
        );
        assert_eq!(
            image_info(Some("img/a.png"), 1).url.as_deref(),
            Some("img/a.png")
        );
        assert_eq!(
            image_info(Some("javascript:x"), 1).skipped,
            Some("unsafe_link")
        );
        assert_eq!(image_info(None, 1).skipped, Some("no_data"));
    }
}
