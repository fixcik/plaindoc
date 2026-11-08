//! Pictures embedded in documents: what a `Block::Image` knows about its picture
//! ([`ImageInfo`]), the bytes kept per document ([`StoredImage`]) and the limits
//! ([`ImageOptions`]).

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::sync::Arc;

/// The picture of a `Block::Image`. Parsers attach the raw bytes ([`ImageInfo::embedded`]) or an
/// external target ([`ImageInfo::external`]); the registry's post-pass (`collect_images`) then
/// numbers, deduplicates and limits them: the bytes move into [`crate::Document::images`] and
/// `file`, `mime`, `size` and `sha256` are filled. Fields that are `None` are omitted from JSON.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ImageInfo {
    /// Name of the stored picture (`image3.png`): one per distinct content, numbered in document
    /// order. `None` when the bytes are not kept (skipped, external, images off).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// MIME type sniffed from the bytes (`image/png`, `image/x-emf`, ...);
    /// `application/octet-stream` when unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<&'static str>,
    /// Size of the picture in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// SHA-256 of the bytes, lower-case hex.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Path of the picture inside the container (`Pictures/1.png`, `word/media/image1.png`),
    /// when the parser knows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// External target that is not downloaded: an `http`/`https` URL or a relative path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Why the bytes are not stored (`too_large`, `max_images`, `max_total_bytes`, `no_data`,
    /// `unsafe_link`, `bad_data`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<&'static str>,
    /// Bytes waiting for the post-pass; `None` after `collect_images` (which `Registry` runs; a
    /// parser called directly leaves them here).
    #[serde(skip)]
    pub(crate) data: Option<Arc<[u8]>>,
}

impl ImageInfo {
    /// A picture with its bytes, optionally with its path inside the container.
    pub fn embedded(data: impl Into<Arc<[u8]>>, source: Option<String>) -> Self {
        Self {
            data: Some(data.into()),
            source,
            ..Self::default()
        }
    }

    /// A picture that only points elsewhere. Kept as `url` when it is `http`/`https` or a
    /// relative path; any other target (`file:`, `javascript:`, absolute local paths, ...) is
    /// dropped and the picture is marked `skipped: unsafe_link`.
    pub fn external(target: &str) -> Self {
        let t = target.trim();
        if safe_link(t) {
            Self {
                url: Some(t.to_string()),
                ..Self::default()
            }
        } else {
            Self::skipped("unsafe_link", None)
        }
    }

    /// A picture whose bytes were not taken, with the reason and, if known, its size.
    pub fn skipped(reason: &'static str, size: Option<u64>) -> Self {
        Self {
            skipped: Some(reason),
            size,
            ..Self::default()
        }
    }

    /// The bytes still attached (before the post-pass).
    pub fn pending_data(&self) -> Option<&[u8]> {
        self.data.as_deref()
    }

    /// `N` of `imageN.ext`.
    pub(crate) fn number(&self) -> Option<usize> {
        let f = self.file.as_deref()?.strip_prefix("image")?;
        f.split('.').next()?.parse().ok()
    }
}

/// `http`/`https` URLs and relative paths (no scheme, no leading `/` or `\`, no drive letter).
fn safe_link(t: &str) -> bool {
    if t.is_empty() || t.chars().any(|c| c.is_control()) {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return true;
    }
    if t.starts_with(['/', '\\']) || t.contains('\\') {
        return false;
    }
    // A scheme is `alpha *( alpha / digit / "+" / "-" / "." ) ":"` before any `/`, `?` or `#`.
    let head = t.split(['/', '?', '#']).next().unwrap_or("");
    !head.contains(':')
}

/// One stored picture of a document: its file name and bytes.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct StoredImage {
    /// `imageN.ext`, as in [`ImageInfo::file`].
    pub file: String,
    pub mime: &'static str,
    pub data: Arc<[u8]>,
}

/// Short id of a picture set: the first `len` (at most 64) hex chars of the SHA-256 over the
/// SHA-256 digests of the pictures in document order. It depends on the bytes only, so the same
/// pictures give the same id for any file name and on every run. The CLI names the pictures
/// directory `img-<id>`.
pub fn image_set_id(images: &[StoredImage], len: usize) -> String {
    let mut all = Sha256::new();
    for img in images {
        all.update(Sha256::digest(&img.data));
    }
    let mut id = String::with_capacity(64);
    for byte in all.finalize() {
        let _ = write!(id, "{byte:02x}");
    }
    id.truncate(len.min(64));
    id
}

/// Picture extraction and its limits ([`crate::ParseOptions::images`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImageOptions {
    /// Off: pictures stay bare marks (`<!-- image -->`), nothing is read or stored.
    pub enabled: bool,
    /// Keep the bytes in [`crate::Document::images`]. Off: only names and metadata (the
    /// output is the same, the memory is not held).
    pub keep_data: bool,
    /// Distinct pictures stored per document; the rest are skipped (`max_images`).
    pub max_count: usize,
    /// Largest picture stored; larger ones are skipped (`too_large`).
    pub max_bytes: u64,
    /// Bytes stored per document in all; pictures past it are skipped (`max_total_bytes`).
    pub max_total_bytes: u64,
}

impl Default for ImageOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            keep_data: true,
            max_count: 1000,
            max_bytes: 32 * 1024 * 1024,
            max_total_bytes: 128 * 1024 * 1024,
        }
    }
}

/// `(mime, extension)` from the leading bytes.
pub fn sniff_image(b: &[u8]) -> (&'static str, &'static str) {
    let head = |m: &[u8]| b.starts_with(m);
    if head(b"\x89PNG\r\n\x1a\n") {
        ("image/png", "png")
    } else if head(b"\xff\xd8\xff") {
        ("image/jpeg", "jpg")
    } else if head(b"GIF87a") || head(b"GIF89a") {
        ("image/gif", "gif")
    } else if head(b"BM") && b.len() >= 14 {
        ("image/bmp", "bmp")
    } else if head(b"II*\0") || head(b"MM\0*") {
        ("image/tiff", "tif")
    } else if head(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        ("image/webp", "webp")
    } else if b.len() >= 44 && head(&[1, 0, 0, 0]) && &b[40..44] == b" EMF" {
        ("image/x-emf", "emf")
    } else if head(&[0xd7, 0xcd, 0xc6, 0x9a]) || head(&[1, 0, 9, 0]) || head(&[2, 0, 9, 0]) {
        ("image/x-wmf", "wmf")
    } else if is_svg(b) {
        ("image/svg+xml", "svg")
    } else {
        ("application/octet-stream", "bin")
    }
}

fn is_svg(b: &[u8]) -> bool {
    let head = &b[..b.len().min(512)];
    let s = String::from_utf8_lossy(head);
    let t = s.trim_start_matches('\u{feff}').trim_start();
    (t.starts_with("<?xml") || t.starts_with("<svg") || t.starts_with("<!--")) && s.contains("<svg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_common_types() {
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\nxxxx").1, "png");
        assert_eq!(sniff_image(b"\xff\xd8\xff\xe0").1, "jpg");
        assert_eq!(sniff_image(b"GIF89a....").1, "gif");
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 ").1, "webp");
        let mut emf = vec![1, 0, 0, 0];
        emf.resize(40, 0);
        emf.extend_from_slice(b" EMF");
        assert_eq!(sniff_image(&emf), ("image/x-emf", "emf"));
        assert_eq!(sniff_image(&[0xd7, 0xcd, 0xc6, 0x9a, 0, 0]).1, "wmf");
        assert_eq!(sniff_image(b"<?xml version='1.0'?><svg/>").1, "svg");
        assert_eq!(sniff_image(b"hello").1, "bin");
    }

    #[test]
    fn external_keeps_only_safe_targets() {
        for ok in [
            "https://x.org/a.png",
            "HTTP://x",
            "img/a.png",
            "../a b.png",
            "a.png?x=1:2",
        ] {
            assert_eq!(ImageInfo::external(ok).url.as_deref(), Some(ok), "{ok}");
        }
        for bad in [
            "javascript:alert(1)",
            "file:///C:/a.png",
            "C:\\a.png",
            "/etc/passwd",
            "data:image/png;base64,xx",
            "",
            "\\\\srv\\a.png",
        ] {
            let i = ImageInfo::external(bad);
            assert_eq!((i.url, i.skipped), (None, Some("unsafe_link")), "{bad}");
        }
    }

    #[test]
    fn number_of_file() {
        let mut i = ImageInfo::default();
        assert_eq!(i.number(), None);
        i.file = Some("image12.png".into());
        assert_eq!(i.number(), Some(12));
    }
}
