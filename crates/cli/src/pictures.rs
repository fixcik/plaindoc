//! Pictures of a parsed document in the CLI: the `img-<id>/` directory next to the result, and
//! the count of pictures that are only named (output on stdout).

use plaindoc_core::{Block, Document, image_set_id};
use std::io;
use std::path::Path;

/// Id lengths tried for the pictures directory, in order.
const ID_LENGTHS: [usize; 3] = [6, 8, 10];

/// Writes the stored pictures of `doc` into `parent/img-<id>/` and returns the directory name
/// (`None` when there are no pictures). The id is a prefix of the picture-set hash
/// ([`image_set_id`]): 6 hex chars; when that directory already holds a different file under one
/// of our names, 8, then 10 are tried. A directory that holds the same bytes is reused.
pub(crate) fn write_images(doc: &Document, parent: &Path) -> io::Result<Option<String>> {
    if doc.images.is_empty() {
        return Ok(None);
    }
    for len in ID_LENGTHS {
        let name = format!("img-{}", image_set_id(&doc.images, len));
        let dir = parent.join(&name);
        let same = |img: &plaindoc_core::StoredImage| {
            std::fs::read(dir.join(&img.file)).is_ok_and(|b| b == *img.data)
        };
        let clash = |img: &plaindoc_core::StoredImage| dir.join(&img.file).exists() && !same(img);
        if doc.images.iter().any(clash) {
            continue;
        }
        std::fs::create_dir_all(&dir)?;
        for img in doc.images.iter().filter(|i| !same(i)) {
            std::fs::write(dir.join(&img.file), &img.data)?;
        }
        return Ok(Some(name));
    }
    Err(io::Error::other(
        "pictures directory name is taken by different files",
    ))
}

/// Distinct pictures the output links by file name (stored or, without -o, only named).
pub(crate) fn named_pictures(doc: &Document) -> usize {
    fn walk<'a>(blocks: &'a [Block], out: &mut std::collections::HashSet<&'a str>) {
        for b in blocks {
            match b {
                Block::Image { image: Some(i), .. } if i.skipped.is_none() => {
                    out.extend(i.file.as_deref());
                }
                Block::List { items, .. } => items.iter().for_each(|it| walk(&it.blocks, out)),
                Block::Table { rows, .. } => {
                    rows.iter().flatten().for_each(|c| walk(&c.blocks, out))
                }
                _ => {}
            }
        }
    }
    let mut files = std::collections::HashSet::new();
    for s in &doc.sections {
        walk(&s.blocks, &mut files);
    }
    files.len()
}
