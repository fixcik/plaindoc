//! Post-pass over a parsed document: numbers, deduplicates and limits its pictures
//! (`ImageInfo` of `Block::Image`) and moves their bytes into `Document::images`.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt::Write;

use crate::image::sniff_image;
use crate::{Block, Document, ImageOptions, Level, StoredImage, Warning, WarningKind};

/// Fills `file`, `mime`, `size`, `sha256` of every picture in document order (sections, then
/// blocks, list items and table cells depth-first) and stores each distinct content once as
/// `imageN.ext`. Pictures over `opts` limits keep their mark and get `skipped`; one `ContentDropped`
/// warning per reason says how many. With `opts.enabled` off every `ImageInfo` is removed (the
/// output is that of a parse without picture support). Idempotent.
pub fn collect_images(doc: &mut Document, opts: &ImageOptions) {
    #[cfg(test)]
    if PANIC_IN_COLLECT.with(std::cell::Cell::get) {
        panic!("collect_images: injected panic");
    }
    let mut c = Collector {
        opts,
        seen: HashMap::new(),
        stored: std::mem::take(&mut doc.images),
        count: 0,
        total: 0,
        skipped: Vec::new(),
        external: 0,
    };
    c.count = c.stored.len();
    c.total = c.stored.iter().map(|s| s.data.len() as u64).sum();
    for s in &mut doc.sections {
        c.blocks(&mut s.blocks);
    }
    doc.images = std::mem::take(&mut c.stored);
    for (reason, n) in &c.skipped {
        // A link we refuse to follow is no loss of ours; the rest are pictures left out.
        let level = if *reason == "unsafe_link" {
            Level::Info
        } else {
            Level::Warn
        };
        doc.warnings.push(Warning::new(
            WarningKind::ContentDropped,
            level,
            format!("image skipped: {} ({n} images)", describe(reason, opts)),
        ));
    }
    if c.external > 0 {
        doc.warnings.push(Warning::new(
            WarningKind::ContentDropped,
            Level::Info,
            format!(
                "image not saved: {} linked to an external target, not downloaded",
                c.external
            ),
        ));
    }
}

fn describe(reason: &str, o: &ImageOptions) -> String {
    match reason {
        "too_large" => format!("larger than {} bytes", o.max_bytes),
        "max_images" => format!("more than {} images", o.max_count),
        "max_total_bytes" => format!("more than {} bytes of images", o.max_total_bytes),
        "no_data" => "picture bytes not available".to_string(),
        "unsafe_link" => "linked to a local or non-web target".to_string(),
        "bad_data" => "picture data cannot be decoded".to_string(),
        other => other.to_string(),
    }
}

struct Collector<'a> {
    opts: &'a ImageOptions,
    /// sha256 -> file
    seen: HashMap<String, String>,
    stored: Vec<StoredImage>,
    count: usize,
    total: u64,
    skipped: Vec<(&'static str, usize)>,
    external: usize,
}

impl Collector<'_> {
    fn blocks(&mut self, blocks: &mut [Block]) {
        for b in blocks {
            match b {
                Block::Image { image, .. } => {
                    if !self.opts.enabled {
                        *image = None;
                    } else if let Some(info) = image {
                        self.image(info);
                    }
                }
                Block::List { items, .. } => {
                    for it in items {
                        self.blocks(&mut it.blocks);
                    }
                }
                Block::Table { rows, .. } => {
                    for c in rows.iter_mut().flatten() {
                        self.blocks(&mut c.blocks);
                    }
                }
                _ => {}
            }
        }
    }

    fn skip(&mut self, reason: &'static str) {
        match self.skipped.iter_mut().find(|(r, _)| *r == reason) {
            Some((_, n)) => *n += 1,
            None => self.skipped.push((reason, 1)),
        }
    }

    fn image(&mut self, info: &mut crate::ImageInfo) {
        let Some(data) = info.data.take() else {
            if let Some(r) = info.skipped {
                self.skip(r);
            } else if info.url.is_some() && info.file.is_none() {
                self.external += 1;
            }
            return;
        };
        let (mime, ext) = sniff_image(&data);
        let size = data.len() as u64;
        let mut sha = String::with_capacity(64);
        for byte in Sha256::digest(&data) {
            let _ = write!(sha, "{byte:02x}");
        }
        info.mime = Some(mime);
        info.size = Some(size);
        if let Some(file) = self.seen.get(&sha) {
            info.file = Some(file.clone());
            info.sha256 = Some(sha);
            return;
        }
        info.sha256 = Some(sha.clone());
        let reason = if size > self.opts.max_bytes {
            Some("too_large")
        } else if self.count >= self.opts.max_count {
            Some("max_images")
        } else if self.total.saturating_add(size) > self.opts.max_total_bytes {
            Some("max_total_bytes")
        } else {
            None
        };
        if let Some(r) = reason {
            info.skipped = Some(r);
            self.skip(r);
            return;
        }
        self.count += 1;
        self.total += size;
        let file = format!("image{}.{ext}", self.count);
        info.file = Some(file.clone());
        if self.opts.keep_data {
            self.stored.push(StoredImage {
                file: file.clone(),
                mime,
                data,
            });
        }
        self.seen.insert(sha, file);
    }
}

#[cfg(test)]
thread_local! {
    /// Fault injection for the guard tests.
    pub(crate) static PANIC_IN_COLLECT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
#[path = "images_test.rs"]
mod tests;
