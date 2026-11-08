//! Pictures of ODF documents: `draw:image` (a part of the package, an external link or inline
//! `office:binary-data`) and reading the parts once the blocks are built.

use std::collections::HashMap;
use std::sync::Arc;

use plaindoc_core::{Block, ImageInfo, ImageOptions, Section};
use roxmltree::Node;

use super::ns::{DRAW, OFFICE, SVG, is};
use super::package::Package;
use crate::picture_data::from_base64;

const XLINK: &str = "http://www.w3.org/1999/xlink";

/// `svg:title`, else `svg:desc` of a `draw:frame`.
pub(crate) fn frame_alt(f: &Node<'_, '_>) -> Option<String> {
    ["title", "desc"].iter().find_map(|t| {
        f.children()
            .find(|a| is(a, SVG, t))
            .map(|a| a.text().unwrap_or("").trim().to_string())
            .filter(|a| !a.is_empty())
    })
}

/// A part path inside the package (`Pictures/1.png`, `./Pictures/1.png`); `None` for a link out.
fn internal(href: &str) -> Option<&str> {
    let h = href.trim().trim_start_matches("./");
    let scheme = h.split(['/', '?', '#']).next().unwrap_or("").contains(':');
    (!h.is_empty() && !scheme && !h.starts_with("../") && !h.starts_with('/')).then_some(h)
}

/// The picture of a `draw:image`: inline data is decoded now (limited by `max`), a package part
/// is only named in `source` and read by [`resolve`], anything else is an external link.
pub(crate) fn image_info(img: &Node<'_, '_>, max: u64) -> Box<ImageInfo> {
    if let Some(data) = img.children().find(|c| is(c, OFFICE, "binary-data")) {
        return Box::new(from_base64(data.text().unwrap_or(""), max));
    }
    let info = match img.attribute((XLINK, "href")) {
        Some(h) => match internal(h) {
            Some(part) => {
                let mut i = ImageInfo::default();
                i.source = Some(part.to_string());
                i
            }
            None => ImageInfo::external(h),
        },
        None => ImageInfo::skipped("no_data", None),
    };
    Box::new(info)
}

/// Pictures anchored in a sheet (in its cells or `table:shapes`), in document order, as image
/// blocks to follow the sheet's table.
pub(crate) fn sheet_pictures(table: Node<'_, '_>, max: u64) -> Vec<Block> {
    table
        .descendants()
        .filter(|n| is(n, DRAW, "frame"))
        .filter_map(|f| {
            let img = f.children().find(|c| is(c, DRAW, "image"))?;
            Some(Block::Image {
                alt: frame_alt(&f),
                ocr_text: None,
                bbox: None,
                image: Some(image_info(&img, max)),
            })
        })
        .collect()
}

/// Reads the package parts named by pending pictures (each part once). Parts past
/// `opts.max_total_bytes` read in all are not read (`max_total_bytes`).
pub(crate) fn resolve(sections: &mut [Section], pkg: &mut Package, opts: &ImageOptions) {
    if !opts.enabled {
        return;
    }
    let mut r = Resolver {
        pkg,
        opts,
        read: HashMap::new(),
        total: 0,
    };
    for s in sections {
        r.blocks(&mut s.blocks);
    }
}

struct Resolver<'a> {
    pkg: &'a mut Package,
    opts: &'a ImageOptions,
    read: HashMap<String, Result<Arc<[u8]>, &'static str>>,
    total: u64,
}

impl Resolver<'_> {
    fn blocks(&mut self, blocks: &mut [Block]) {
        for b in blocks {
            match b {
                Block::Image { image: Some(i), .. } => self.image(i),
                Block::List { items, .. } => {
                    items.iter_mut().for_each(|i| self.blocks(&mut i.blocks))
                }
                Block::Table { rows, .. } => rows
                    .iter_mut()
                    .flatten()
                    .for_each(|c| self.blocks(&mut c.blocks)),
                _ => {}
            }
        }
    }

    fn image(&mut self, info: &mut Box<ImageInfo>) {
        let pending = info.pending_data().is_none()
            && info.url.is_none()
            && info.skipped.is_none()
            && info.file.is_none();
        let Some(part) = info.source.clone().filter(|_| pending) else {
            return;
        };
        if !self.read.contains_key(&part) {
            let got = if self.total >= self.opts.max_total_bytes {
                Err("max_total_bytes")
            } else {
                self.pkg
                    .picture(&part, self.opts.max_bytes)
                    .map(Arc::<[u8]>::from)
            };
            if let Ok(b) = &got {
                self.total += b.len() as u64;
            }
            self.read.insert(part.clone(), got);
        }
        **info = match &self.read[&part] {
            Ok(b) => ImageInfo::embedded(Arc::clone(b), Some(part)),
            Err(reason) => {
                let mut i = ImageInfo::skipped(reason, None);
                i.source = Some(part);
                i
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_parts_and_links() {
        assert_eq!(internal("Pictures/a.png"), Some("Pictures/a.png"));
        assert_eq!(internal("./Pictures/a.png"), Some("Pictures/a.png"));
        assert_eq!(internal("../a.png"), None);
        assert_eq!(internal("http://e.org/a.png"), None);
        assert_eq!(internal("file:///a.png"), None);
        assert_eq!(internal("/abs/a.png"), None);
        assert_eq!(internal(""), None);
    }
}
