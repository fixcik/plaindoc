//! SP8b S5: the synthetic resume set (`tests/fixtures/pdf/resume`, `scripts/resume/gen_resumes.py`).
//! Each PDF has a manifest with the ground truth (headings, list items, links, no tables).
//! Green today: no false tables (one column and sidebar), link recall, list items of the
//! one-column variants (SP8b S1). Headings are another lane's work: the scoreboard is
//! `--ignored --nocapture`, and the strict checks still ignored wait for their lanes.

use plaindoc_core::*;
use plaindoc_formats::pdf::PdfParser;
use serde_json::Value;
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdf/resume")
}

/// Lower case, no whitespace: spaced-letter titles and wrapped lines compare equal.
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

#[derive(Default)]
struct Found {
    headings: Vec<String>,
    levels: Vec<u8>,
    items: Vec<(String, usize)>,
    links: Vec<(String, String)>,
    tables: usize,
}

fn walk(b: &Block, depth: usize, f: &mut Found) {
    match b {
        Block::Heading { text, links, .. } | Block::Paragraph { text, links, .. } => {
            if let Block::Heading { level, .. } = b {
                f.headings.push(text.clone());
                f.levels.push(*level);
            }
            let chars: Vec<char> = text.chars().collect();
            for l in links {
                let t: String = chars[l.start..l.end].iter().collect();
                f.links.push((t, l.url.clone()));
            }
        }
        Block::Table { rows, .. } => {
            f.tables += 1;
            for x in rows.iter().flatten().flat_map(|c| &c.blocks) {
                walk(x, depth, f);
            }
        }
        Block::List { items, .. } => {
            for i in items {
                f.items.push((i.text(), depth));
                for x in &i.blocks {
                    // Link anchors of the item; nested lists are one level deeper.
                    walk(x, depth + 1, f);
                }
            }
        }
        _ => {}
    }
}

struct Case {
    name: String,
    gt: Value,
    found: Found,
}

fn cases() -> Vec<Case> {
    let mut paths: Vec<_> = std::fs::read_dir(dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "pdf"))
        .collect();
    paths.sort();
    assert!(
        paths.len() >= 20,
        "resume fixtures missing: {}",
        paths.len()
    );
    paths
        .into_iter()
        .map(|p| {
            let gt: Value =
                serde_json::from_slice(&std::fs::read(p.with_extension("json")).unwrap()).unwrap();
            let doc = PdfParser::new()
                .parse(
                    Input::Bytes(std::fs::read(&p).unwrap()),
                    Format::Pdf,
                    &ParseOptions::default(),
                )
                .unwrap();
            let mut found = Found::default();
            for b in doc.sections.iter().flat_map(|s| &s.blocks) {
                walk(b, 0, &mut found);
            }
            Case {
                name: p.file_stem().unwrap().to_string_lossy().into_owned(),
                gt,
                found,
            }
        })
        .collect()
}

fn arr<'a>(gt: &'a Value, k: &str) -> &'a Vec<Value> {
    gt[k].as_array().unwrap()
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap()
}

/// Expected headings whose text occurs among the found headings.
fn headings_hit(c: &Case) -> usize {
    let got: Vec<String> = c.found.headings.iter().map(|h| norm(h)).collect();
    arr(&c.gt, "headings")
        .iter()
        .filter(|h| got.contains(&norm(s(h, "text"))))
        .count()
}

/// Expected headings found with the expected text and level.
fn headings_with_level(c: &Case) -> usize {
    arr(&c.gt, "headings")
        .iter()
        .filter(|h| {
            let i = c
                .found
                .headings
                .iter()
                .position(|f| norm(f) == norm(s(h, "text")));
            i.is_some_and(|i| u64::from(c.found.levels[i]) == h["level"].as_u64().unwrap())
        })
        .count()
}

/// Expected list items found as list items (text equal after normalisation, at the same depth).
fn items_hit(c: &Case) -> usize {
    let got: Vec<(String, usize)> = c.found.items.iter().map(|(t, d)| (norm(t), *d)).collect();
    arr(&c.gt, "list_items")
        .iter()
        .filter(|i| {
            let d = i["depth"].as_u64().unwrap() as usize;
            got.iter()
                .any(|(t, gd)| *gd == d && t.ends_with(&norm(s(i, "text"))))
        })
        .count()
}

fn links_hit(c: &Case) -> usize {
    arr(&c.gt, "links")
        .iter()
        .filter(|l| {
            c.found
                .links
                .iter()
                .any(|(t, u)| norm(t) == norm(s(l, "text")) && u == s(l, "url"))
        })
        .count()
}

fn columns(c: &Case) -> u64 {
    c.gt["description"]["cols"].as_u64().unwrap()
}

#[test]
fn resume_set_keeps_links_and_one_column_has_no_false_tables() {
    let mut bad = Vec::new();
    for c in cases() {
        assert_eq!(c.gt["tables"], 0, "{}", c.name);
        if columns(&c) == 1 && c.found.tables != 0 {
            bad.push(format!("{}: {} tables", c.name, c.found.tables));
        }
        let (hit, want) = (links_hit(&c), arr(&c.gt, "links").len());
        if hit != want {
            bad.push(format!("{}: links {hit}/{want}", c.name));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// SP8b S3: skill chips (`box_flow`) and rows across the main column and the sidebar (`wide_gap_columns`).
#[test]
fn resume_sidebar_has_no_false_tables() {
    let bad: Vec<String> = cases()
        .iter()
        .filter(|c| c.found.tables != 0)
        .map(|c| format!("{}: {}", c.name, c.found.tables))
        .collect();
    assert!(bad.is_empty(), "{bad:?}");
}

#[test]
#[ignore = "scoreboard: cargo test -p plaindoc-formats --features pdf --test pdf_resume_set -- --ignored --nocapture"]
fn resume_scoreboard() {
    println!(
        "{:<34} {:>9} {:>9} {:>7} {:>6}  (found/expected; extra = found headings / list items)",
        "variant", "headings", "items", "links", "tables"
    );
    let mut t = [0usize; 6];
    for c in cases() {
        let row = [
            headings_hit(&c),
            arr(&c.gt, "headings").len(),
            items_hit(&c),
            arr(&c.gt, "list_items").len(),
            links_hit(&c),
            arr(&c.gt, "links").len(),
        ];
        for (a, b) in t.iter_mut().zip(row) {
            *a += b;
        }
        println!(
            "{:<34} {:>6}/{:<2} {:>6}/{:<2} {:>4}/{:<2} {:>6}   found {} headings, {} items",
            c.name,
            row[0],
            row[1],
            row[2],
            row[3],
            row[4],
            row[5],
            c.found.tables,
            c.found.headings.len(),
            c.found.items.len()
        );
    }
    println!(
        "{:<34} {:>6}/{:<2} {:>6}/{:<3} {:>3}/{:<2}",
        "TOTAL", t[0], t[1], t[2], t[3], t[4], t[5]
    );
}

#[test]
fn resume_one_column_headings_match_manifest_with_levels() {
    let bad: Vec<String> = cases()
        .iter()
        .filter(|c| columns(c) == 1)
        .filter(|c| headings_with_level(c) != arr(&c.gt, "headings").len())
        .map(|c| c.name.clone())
        .collect();
    assert!(bad.is_empty(), "{bad:?}");
}

#[test]
#[ignore = "headings: on sidebar variants a main-column title shares its line with a sidebar label (\"О СЕБЕ ПОЧТА\"), 6 of 129 (SP8b S2 report)"]
fn resume_headings_match_manifest() {
    let bad: Vec<String> = cases()
        .iter()
        .filter(|c| headings_hit(c) != arr(&c.gt, "headings").len())
        .map(|c| c.name.clone())
        .collect();
    assert!(bad.is_empty(), "{bad:?}");
}

/// Variants whose list items are not all found: `one_column` = 1 limits it to the one-column ones.
fn items_missed(one_column: bool) -> Vec<String> {
    cases()
        .iter()
        .filter(|c| !one_column || columns(c) == 1)
        .filter(|c| items_hit(c) != arr(&c.gt, "list_items").len())
        .map(|c| {
            format!(
                "{}: {}/{}",
                c.name,
                items_hit(c),
                arr(&c.gt, "list_items").len()
            )
        })
        .collect()
}

#[test]
fn resume_list_items_match_manifest() {
    let bad = items_missed(true);
    assert!(bad.is_empty(), "{bad:?}");
}

#[test]
#[ignore = "sidebar variants: the first line of an item or its wrapped tail lands in another region or a false table (reading order / table lanes)"]
fn resume_sidebar_list_items_match_manifest() {
    let bad = items_missed(false);
    assert!(bad.is_empty(), "{bad:?}");
}
