#![allow(dead_code)]
//! Shared helpers for office integration tests.

use plaindoc_core::{Block, Cell, Document, Input, NoteKind, ParseOptions, Result};
use plaindoc_formats::Registry;
use std::path::{Path, PathBuf};

/// Root of the office fixture corpus.
pub fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/office")
}

/// Path of a fixture relative to `tests/fixtures/office/`.
pub fn fixture(rel: &str) -> PathBuf {
    fixtures_root().join(rel)
}

/// All files in `generated/` and `poi/` except `NOTICE` and dotfiles, sorted.
pub fn all_fixtures() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in ["generated", "poi"] {
        let Ok(entries) = std::fs::read_dir(fixture(dir)) else {
            continue;
        };
        for e in entries {
            let path = e.unwrap().path();
            if path.is_file()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n != "NOTICE" && !n.starts_with('.'))
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// `Registry::with_defaults()`.
pub fn registry() -> Registry {
    Registry::with_defaults()
}

/// Parses a fixture (path relative to `tests/fixtures/office/`) with default options.
pub fn parse_fixture(rel: &str) -> Result<Document> {
    registry().parse(Input::Path(fixture(rel)), &ParseOptions::default())
}

fn collect_text(blocks: &[Block], out: &mut Vec<String>) {
    for b in blocks {
        match b {
            Block::Heading { text, .. }
            | Block::Paragraph { text, .. }
            | Block::Code { text, .. }
            | Block::Note { text, .. } => out.push(text.clone()),
            Block::List { items, .. } => {
                for it in items {
                    collect_text(&it.blocks, out);
                }
            }
            Block::Table { rows, .. } => {
                for c in rows.iter().flatten() {
                    collect_text(&c.blocks, out);
                }
            }
            Block::Image { alt, ocr_text, .. } => {
                out.extend(alt.iter().chain(ocr_text.iter()).cloned());
            }
            _ => {}
        }
    }
}

/// All block text (recursing into cells, list items and notes), joined with `\n`.
pub fn all_text(d: &Document) -> String {
    let mut out = Vec::new();
    for s in &d.sections {
        collect_text(&s.blocks, &mut out);
    }
    out.join("\n")
}

/// Top-level tables in section order.
pub fn tables(d: &Document) -> Vec<&Block> {
    d.sections
        .iter()
        .flat_map(|s| &s.blocks)
        .filter(|b| matches!(b, Block::Table { .. }))
        .collect()
}

/// Per row: (normalized cell text, colspan, rowspan).
pub fn table_signature(b: &Block) -> Vec<Vec<(String, u32, u32)>> {
    let Block::Table { rows, .. } = b else {
        panic!("not a table: {b:?}");
    };
    rows.iter()
        .map(|r| {
            r.iter()
                .map(|c| {
                    let t = c.text().split_whitespace().collect::<Vec<_>>().join(" ");
                    (t, c.colspan, c.rowspan)
                })
                .collect()
        })
        .collect()
}

/// Texts of top-level `Note` blocks of the given kind.
pub fn notes(d: &Document, kind: NoteKind) -> Vec<String> {
    d.sections
        .iter()
        .flat_map(|s| &s.blocks)
        .filter_map(|b| match b {
            Block::Note { kind: k, text, .. } if *k == kind => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// First top-level table cell whose `text()` equals `text`.
pub fn find_cell<'a>(d: &'a Document, text: &str) -> &'a Cell {
    tables(d)
        .into_iter()
        .filter_map(|b| match b {
            Block::Table { rows, .. } => Some(rows),
            _ => None,
        })
        .flatten()
        .flatten()
        .find(|c| c.text() == text)
        .unwrap_or_else(|| panic!("cell {text:?} not found"))
}
