//! Pictures of OOXML documents: `ImageInfo` for IR images (one copy per distinct picture, within
//! the total budget), their paths inside the package, and header/footer pictures.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use office_oxide::ir::{Element, Image};
use plaindoc_core::{Block, ImageInfo, ImageOptions};

/// Picture settings of one parse, the media parts of the package and the copies made so far.
#[derive(Clone, Debug, Default)]
pub(crate) struct Pictures {
    /// `ImageOptions::enabled`; off (the default) maps images as bare marks.
    pub enabled: bool,
    pub max_bytes: u64,
    pub max_total_bytes: u64,
    /// `(size, crc32, name)` of the `*/media/*` parts, from the zip central directory.
    media: Vec<(u64, u32, String)>,
    /// Copies of the IR bytes by `(size, crc32)`: a picture referenced many times is copied once.
    copies: HashMap<(u64, u32), Vec<Arc<[u8]>>>,
    /// Bytes copied in all (distinct pictures).
    copied: u64,
    /// Header/footer pictures already emitted (by content or link), once per document.
    hf_seen: HashSet<(u64, u32, Option<String>)>,
}

/// Media parts listed at most (the central directory is read, nothing is inflated).
const MAX_MEDIA: usize = 4096;

impl Pictures {
    pub fn new(opts: &ImageOptions) -> Self {
        Self {
            enabled: opts.enabled,
            max_bytes: opts.max_bytes,
            max_total_bytes: opts.max_total_bytes,
            ..Self::default()
        }
    }

    /// Lists the media parts of an OOXML package from its central directory; any error leaves
    /// the list empty. `r` is left at position 0.
    pub fn read_media<R: Read + Seek>(&mut self, r: &mut R) {
        if self.enabled
            && let Ok(mut zip) = zip::ZipArchive::new(&mut *r)
        {
            for i in 0..zip.len() {
                let Ok(e) = zip.by_index_raw(i) else { continue };
                let name = e.name().map(|n| n.to_string()).unwrap_or_default();
                if name.contains("/media/") && !name.ends_with('/') {
                    self.media.push((e.size(), e.crc32(), name));
                    if self.media.len() >= MAX_MEDIA {
                        break;
                    }
                }
            }
        }
        let _ = r.seek(SeekFrom::Start(0));
    }

    /// The picture of an IR image: its bytes (with the media part they came from), or the linked
    /// target. `None` when pictures are off or the image has neither (a shape's alternative text,
    /// an unresolved relationship): it stays a bare mark as before.
    pub fn info(&mut self, i: &Image) -> Option<Box<ImageInfo>> {
        if !self.enabled {
            return None;
        }
        let info = match (&i.data, &i.source_url) {
            (Some(d), _) if d.len() as u64 > self.max_bytes => {
                ImageInfo::skipped("too_large", Some(d.len() as u64))
            }
            (Some(d), _) => {
                let crc = crc32fast::hash(d);
                match self.copy(d, crc) {
                    Some(a) => ImageInfo::embedded(a, self.source_of(d.len() as u64, crc)),
                    None => ImageInfo::skipped("max_total_bytes", Some(d.len() as u64)),
                }
            }
            (None, Some(url)) => ImageInfo::external(url),
            (None, None) => return None,
        };
        Some(Box::new(info))
    }

    /// The shared copy of `d`, made on first sight; `None` once `max_total_bytes` would be passed.
    fn copy(&mut self, d: &[u8], crc: u32) -> Option<Arc<[u8]>> {
        let key = (d.len() as u64, crc);
        if let Some(a) = self
            .copies
            .get(&key)
            .and_then(|v| v.iter().find(|a| ***a == *d))
        {
            return Some(Arc::clone(a));
        }
        let total = self.copied.saturating_add(d.len() as u64);
        if total > self.max_total_bytes {
            return None;
        }
        self.copied = total;
        let a: Arc<[u8]> = Arc::from(d);
        self.copies.entry(key).or_default().push(Arc::clone(&a));
        Some(a)
    }

    fn source_of(&self, size: u64, crc: u32) -> Option<String> {
        self.media
            .iter()
            .find(|(s, c, _)| *s == size && *c == crc)
            .map(|(_, _, n)| n.clone())
    }

    /// Image blocks of the pictures in headers and footers, each distinct one once per document.
    pub fn header_footer(&mut self, parts: &[&[Element]]) -> Vec<Block> {
        if !self.enabled {
            return Vec::new();
        }
        let mut imgs = Vec::new();
        for els in parts {
            images(els, &mut imgs);
        }
        let mut out = Vec::new();
        for i in imgs {
            let key = match &i.data {
                Some(d) => (d.len() as u64, crc32fast::hash(d), None),
                None => (0, 0, i.source_url.clone()),
            };
            if !self.hf_seen.insert(key) {
                continue;
            }
            if let Some(info) = self.info(i) {
                out.push(Block::Image {
                    alt: i.alt_text.clone(),
                    ocr_text: None,
                    bbox: None,
                    image: Some(info),
                });
            }
        }
        out
    }
}

/// Images anywhere under `els`, in order.
fn images<'a>(els: &'a [Element], out: &mut Vec<&'a Image>) {
    for e in els {
        match e {
            Element::Image(i) => out.push(i),
            Element::TextBox(tb) => images(&tb.content, out),
            Element::Table(t) => t
                .rows
                .iter()
                .flat_map(|r| &r.cells)
                .for_each(|c| images(&c.content, out)),
            Element::List(l) => list_images(l, out),
            _ => {}
        }
    }
}

fn list_images<'a>(l: &'a office_oxide::ir::List, out: &mut Vec<&'a Image>) {
    for it in &l.items {
        images(&it.content, out);
        if let Some(n) = &it.nested {
            list_images(n, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(data: Option<&[u8]>, url: Option<&str>) -> Image {
        Image {
            data: data.map(<[u8]>::to_vec),
            source_url: url.map(String::from),
            ..Default::default()
        }
    }

    #[test]
    fn info_by_kind_of_image() {
        let mut off = Pictures::default();
        assert!(off.info(&img(Some(b"x"), None)).is_none());
        let mut on = Pictures::new(&ImageOptions::default());
        on.media
            .push((3, crc32fast::hash(b"abc"), "word/media/image1.png".into()));
        let i = on.info(&img(Some(b"abc"), None)).unwrap();
        assert_eq!(i.source.as_deref(), Some("word/media/image1.png"));
        assert_eq!(i.pending_data(), Some(&b"abc"[..]));
        assert!(on.info(&img(Some(b"abd"), None)).unwrap().source.is_none());
        let l = on.info(&img(None, Some("https://e.org/p.png"))).unwrap();
        assert_eq!(l.url.as_deref(), Some("https://e.org/p.png"));
        assert!(on.info(&img(None, None)).is_none());
        on.max_bytes = 2;
        let big = on.info(&img(Some(b"abc"), None)).unwrap();
        assert_eq!((big.skipped, big.size), (Some("too_large"), Some(3)));
        assert!(big.pending_data().is_none());
    }

    #[test]
    fn one_copy_per_distinct_picture_within_the_total() {
        let mut p = Pictures::new(&ImageOptions::default());
        p.max_total_bytes = 5;
        let a = img(Some(b"abc"), None);
        let ptrs: Vec<*const u8> = (0..300)
            .map(|_| p.info(&a).unwrap().pending_data().unwrap().as_ptr())
            .collect();
        assert!(ptrs.iter().all(|x| *x == ptrs[0]));
        assert_eq!(p.copied, 3);
        // a second distinct picture would pass the total: not copied
        let b = p.info(&img(Some(b"xyz"), None)).unwrap();
        assert_eq!(
            (b.skipped, b.pending_data()),
            (Some("max_total_bytes"), None)
        );
    }

    #[test]
    fn header_footer_pictures_once_per_document() {
        let mut p = Pictures::new(&ImageOptions::default());
        let logo = Element::Image(img(Some(b"logo"), None));
        let els = vec![logo.clone(), logo];
        assert_eq!(p.header_footer(&[&els, &els]).len(), 1);
        assert!(p.header_footer(&[&els]).is_empty());
        assert!(Pictures::default().header_footer(&[&els]).is_empty());
    }
}
