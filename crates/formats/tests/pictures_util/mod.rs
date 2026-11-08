//! Tiny synthetic pictures and helpers for the picture-extraction tests.
#![allow(dead_code)]
use plaindoc_core::*;
use std::io::Write;

/// 1x1 red PNG.
pub const RED: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0xc9, 0xfe, 0x92, 0xef, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];
/// 1x1 blue PNG.
pub const BLUE: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0x60, 0xf8, 0x0f,
    0x00, 0x01, 0x03, 0x01, 0x00, 0x08, 0x89, 0xc2, 0xec, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];
/// Header of an EMF (enough for the type sniffing).
pub fn emf() -> Vec<u8> {
    let mut b = vec![1, 0, 0, 0];
    b.resize(40, 7);
    b.extend_from_slice(b" EMF");
    b.extend_from_slice(&[0; 20]);
    b
}

/// Pictures of every image block in document order: `(alt, info)`.
pub fn pictures(d: &Document) -> Vec<(Option<String>, Option<ImageInfo>)> {
    fn walk(bs: &[Block], out: &mut Vec<(Option<String>, Option<ImageInfo>)>) {
        for b in bs {
            match b {
                Block::Image { alt, image, .. } => {
                    out.push((alt.clone(), image.as_deref().cloned()))
                }
                Block::List { items, .. } => items.iter().for_each(|i| walk(&i.blocks, out)),
                Block::Table { rows, .. } => {
                    rows.iter().flatten().for_each(|c| walk(&c.blocks, out))
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for s in &d.sections {
        walk(&s.blocks, &mut out);
    }
    out
}

/// `file` of each picture (`None` for a bare mark or a skipped one).
pub fn files(d: &Document) -> Vec<Option<String>> {
    pictures(d)
        .into_iter()
        .map(|(_, i)| i.and_then(|i| i.file))
        .collect()
}

pub fn stored<'a>(d: &'a Document, file: &str) -> &'a [u8] {
    &d.images.iter().find(|s| s.file == file).unwrap().data
}

/// A zip with the given parts, deflated.
pub fn zip_of(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in parts {
        z.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(body).unwrap();
    }
    z.finish().unwrap().into_inner()
}

pub fn parse(bytes: Vec<u8>, opts: &ParseOptions) -> Document {
    plaindoc_formats::Registry::with_defaults()
        .parse(Input::Bytes(bytes), opts)
        .unwrap()
}

pub fn no_images() -> ParseOptions {
    let mut o = ParseOptions::default();
    o.images.enabled = false;
    o
}

/// An ODF package: `mimetype` first and stored, then the parts.
pub fn odf_of(mimetype: &str, parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    z.start_file("mimetype", stored).unwrap();
    z.write_all(mimetype.as_bytes()).unwrap();
    for (name, body) in parts {
        z.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(body).unwrap();
    }
    z.finish().unwrap().into_inner()
}
