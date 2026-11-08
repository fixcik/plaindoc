//! D10 readiness gate for PDF tables on corpora kept outside the repo (see docs/tables-eval.md):
//! `TABLES_EVAL=<name>:<pdf dir>:<unified gt.json>[:<split>]`, several separated by `;`, or
//! `TABLES_DATA=<root>` with `<root>/{odl,icdar,pt1m,synth}/{pdf/,gt.json}` (`scripts/tables/fetch-all.sh`).
//! Names `odl`, `icdar`, `pt1m` get their D10 thresholds (point estimates; the CI is printed, ruling
//! В-8); `synth` and any other name are informative.

use std::path::Path;

use plaindoc_core::{Block, Format, Input, ParseOptions, Parser};
use plaindoc_formats::pdf::{OwnTables, PdfConfig, PdfParser};
use plaindoc_formats::tables::TableConfig;
use plaindoc_formats::tables::eval::{
    EvalCell, EvalRows, Summary, TEDS_CAP_SPIKE, TEDS_CAP_SYNTH, bootstrap, evaluate_doc, load_gt,
    split_docs,
};
use rayon::prelude::*;

/// D10 (contract-decisions.md): found ≥, TEDS ≥, false tables ≤ (per file for ODL, per page otherwise).
struct Gate {
    found: f64,
    teds: f64,
    fp: f64,
    fp_per_doc: bool,
}

fn gate(name: &str) -> Option<Gate> {
    match name {
        "odl" => Some(Gate {
            found: 0.70,
            teds: 0.65,
            fp: 0.05,
            fp_per_doc: true,
        }),
        "icdar" => Some(Gate {
            found: 0.70,
            teds: 0.65,
            fp: 0.02,
            fp_per_doc: false,
        }),
        "pt1m" => Some(Gate {
            found: 0.60,
            teds: 0.45,
            fp: 0.02,
            fp_per_doc: false,
        }),
        _ => None,
    }
}

/// `TABLES_CFG=reference` measures the Python-parity config, `split` the shipped one with glued-word
/// splitting on (ruling В-4 turned it off), `merge`/`nomerge` the shipped one with `merge_lines` forced
/// on/off (A1 A/B), `extend` the shipped one with `extend_first_column` (12b), `scan` the shipped one with
/// `short_vertical_stacks` (12c; PDF segments are not raster, so it must equal the default); default: shipped.
fn parser() -> PdfParser {
    let mut cfg = PdfConfig::default();
    let tables = match std::env::var("TABLES_CFG").as_deref() {
        Ok("reference") => TableConfig::reference(),
        Ok("split") => TableConfig {
            split_glued_words: true,
            ..TableConfig::default()
        },
        Ok("merge") => TableConfig {
            merge_lines: true,
            ..TableConfig::default()
        },
        Ok("extend") => TableConfig {
            extend_first_column: true,
            ..TableConfig::default()
        },
        Ok("scan") => TableConfig {
            short_vertical_stacks: true,
            ..TableConfig::default()
        },
        Ok("nomerge") => TableConfig {
            merge_lines: false,
            ..TableConfig::default()
        },
        _ => TableConfig::default(),
    };
    // `TABLES_A2=<comma list>`: of the A2 fields only the listed ones on (empty: all off), for bisection.
    let tables = match std::env::var("TABLES_A2") {
        Ok(on) => {
            let has = |f: &str| on.split(',').any(|x| x.trim() == f);
            TableConfig {
                fill_groups: has("fill_groups"),
                rg_extend: has("rg_extend"),
                dense_valleys: has("dense_valleys"),
                rg_split: has("rg_split"),
                fill_split: has("fill_split"),
                ..tables
            }
        }
        Err(_) => tables,
    };
    // `TABLES_T14=<comma list of maxcols, gmin_adapt, axis_guard>`: of the T14 changes (all on in the
    // shipped config) only the listed ones on, for A/B (empty: all off; `maxcols` off is the reference 15).
    let tables = match std::env::var("TABLES_T14") {
        Ok(on) => {
            let has = |f: &str| on.split(',').any(|x| x.trim() == f);
            TableConfig {
                text_max_cols: if has("maxcols") {
                    plaindoc_formats::tables::WIDE_TEXT_MAX_COLS
                } else {
                    TableConfig::reference().text_max_cols
                },
                gmin_adapt: has("gmin_adapt"),
                axis_guard: has("axis_guard"),
                ..tables
            }
        }
        Err(_) => tables,
    };
    // `TABLES_NEW=<comma list of new flag names>`: the listed new flags on; `TABLES_OFF=<comma list>`: the
    // listed ones off (same eleven names; absent: their config value, eight are on in `default()`). A name in
    // both lists is off.
    let (on, off) = (
        std::env::var("TABLES_NEW").unwrap_or_default(),
        std::env::var("TABLES_OFF").unwrap_or_default(),
    );
    let flag = |cur: bool, f: &str| {
        let has = |l: &str| l.split(',').any(|x| x.trim() == f);
        (cur || has(&on)) && !has(&off)
    };
    let tables = TableConfig {
        used_lined_only: flag(tables.used_lined_only, "used_lined_only"),
        band_split: flag(tables.band_split, "band_split"),
        rg_tall_regular: flag(tables.rg_tall_regular, "rg_tall_regular"),
        rg_regular_rows: flag(tables.rg_regular_rows, "rg_regular_rows"),
        rg_fill_header: flag(tables.rg_fill_header, "rg_fill_header"),
        rg_split_multiseg: flag(tables.rg_split_multiseg, "rg_split_multiseg"),
        page_columns: flag(tables.page_columns, "page_columns"),
        uncertain_page_columns: flag(tables.uncertain_page_columns, "uncertain_page_columns"),
        lined_skip_background: flag(tables.lined_skip_background, "lined_skip_background"),
        wide_gap_columns: flag(tables.wide_gap_columns, "wide_gap_columns"),
        box_flow: flag(tables.box_flow, "box_flow"),
        ..tables
    };
    // `TABLES_T16=1`/`0`: `text_join` on/off (T16 A/B).
    let tables = TableConfig {
        text_join: std::env::var("TABLES_T16").map_or(tables.text_join, |v| v != "0"),
        ..tables
    };
    // `TABLES_T17=<row_newcols>[,hdr]`: T17 A/B; `0` off, `2`/`3` the row rule, `,hdr` also
    // `header_lines: 3` + `header_split` (`hdr` alone: only those).
    let tables = match std::env::var("TABLES_T17") {
        Ok(v) => {
            let hdr = v.split(',').any(|x| x.trim() == "hdr");
            TableConfig {
                row_newcols: v
                    .split(',')
                    .find_map(|x| x.trim().parse().ok())
                    .unwrap_or(0),
                header_lines: if hdr {
                    plaindoc_formats::tables::MAX_HEADER_LINES
                } else {
                    1
                },
                header_split: hdr,
                ..tables
            }
        }
        Err(_) => tables,
    };
    // `TABLES_T15=<gutter mode>`: T15 A/B; `0` off, `1`/`3` `text_longseg` + `text_dense` + that
    // `gutter_prose` mode.
    let tables = match std::env::var("TABLES_T15") {
        Ok(v) => {
            let mode: u8 = v
                .trim()
                .parse()
                .unwrap_or_else(|_| panic!("TABLES_T15={v:?}: expected a gutter mode 0, 1 or 3"));
            TableConfig {
                text_longseg: mode > 0,
                text_dense: mode > 0,
                gutter_prose: mode,
                ..tables
            }
        }
        Err(_) => tables,
    };
    // `TABLES_FRAME=0`/`1`: `frame_prose` off/on (two-column prose in a page frame, A/B).
    let tables = TableConfig {
        frame_prose: std::env::var("TABLES_FRAME").map_or(tables.frame_prose, |v| v != "0"),
        ..tables
    };
    // `TABLES_A3=0`: comp_lines off (A/B for the A3 gate).
    let tables = TableConfig {
        comp_lines: std::env::var("TABLES_A3").map_or(tables.comp_lines, |v| v != "0"),
        ..tables
    };
    cfg.tables = std::sync::Arc::new(OwnTables { config: tables });
    PdfParser::with_config(cfg)
}

/// Cell text with nested tables flattened row by row (as the ground truth stores them).
fn flat(blocks: &[Block]) -> String {
    let parts: Vec<String> = blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph { text, .. } => text.clone(),
            Block::Table { rows, .. } => rows
                .iter()
                .flatten()
                .map(|c| flat(&c.blocks))
                .collect::<Vec<_>>()
                .join(" "),
            _ => String::new(),
        })
        .filter(|t| !t.trim().is_empty())
        .collect();
    parts.join(" ")
}

fn tables_of(pdf: &Path) -> Vec<(usize, EvalRows)> {
    let doc = parser()
        .parse(
            Input::Path(pdf.to_path_buf()),
            Format::Pdf,
            &ParseOptions::default(),
        )
        .unwrap_or_else(|e| panic!("{}: {e}", pdf.display()));
    let mut out = Vec::new();
    for s in &doc.sections {
        for b in &s.blocks {
            if let Block::Table { rows, .. } = b {
                let rows = rows
                    .iter()
                    .map(|r| {
                        r.iter()
                            .map(|c| EvalCell {
                                colspan: c.colspan,
                                rowspan: c.rowspan,
                                text: flat(&c.blocks),
                            })
                            .collect()
                    })
                    .collect();
                out.push((s.index, rows));
            }
        }
    }
    out
}

/// `TABLES_EVAL`, or the D10 set under `TABLES_DATA`; `None` when neither is set.
fn eval_spec() -> Option<String> {
    if let Ok(s) = std::env::var("TABLES_EVAL") {
        return Some(s);
    }
    let root = std::env::var("TABLES_DATA").ok()?;
    let mut items: Vec<String> = [("odl", ""), ("icdar", ":test"), ("pt1m", ":test")]
        .iter()
        .map(|(n, split)| format!("{n}:{root}/{n}/pdf:{root}/{n}/gt.json{split}"))
        .collect();
    if Path::new(&root).join("synth/gt.json").exists() {
        items.push(format!("synth:{root}/synth/pdf:{root}/synth/gt.json"));
    }
    Some(items.join(";"))
}

/// Document ids of the exclusion list (first column, `#` comments skipped); empty when absent.
fn incomplete_gt() -> std::collections::HashSet<String> {
    let path = std::env::var("TABLES_EXCLUDE").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/tables/lists/incomplete_gt.txt"
        )
        .to_string()
    });
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect()
}

#[test]
#[ignore = "needs TABLES_EVAL or TABLES_DATA (corpora are kept outside the repo)"]
fn d10_table_gate() {
    let Some(spec) = eval_spec() else {
        eprintln!("skip: set TABLES_DATA=<root> or TABLES_EVAL=<name>:<pdfs>:<gt.json>[:<split>]");
        return;
    };
    let items: Vec<Vec<&str>> = spec
        .split(';')
        .filter(|s| !s.is_empty())
        .map(|i| i.split(':').collect())
        .collect();
    // TABLES_GATE_OFF=1: print only (A/B runs on subsets or without the test splits).
    // The gate is only meaningful on all three D10 corpora with the D10 splits; other splits (PT1M
    // train/all) and names are printed, not gated.
    const D10: [[&str; 2]; 3] = [["odl", "all"], ["icdar", "test"], ["pt1m", "test"]];
    let gated = std::env::var_os("TABLES_GATE_OFF").is_none();
    if gated {
        for need in D10 {
            assert!(
                items
                    .iter()
                    .any(|p| p[0] == need[0] && p.get(3).copied().unwrap_or("all") == need[1]),
                "D10 gate needs odl, icdar:test, pt1m:test (missing {}:{})",
                need[0],
                need[1]
            );
        }
    }
    let mut failed = Vec::new();
    for parts in &items {
        let (name, pdfs, gt) = (parts[0], parts[1], parts[2]);
        let split = parts.get(3).copied().unwrap_or("all");
        let gt = split_docs(
            load_gt(
                &std::fs::read_to_string(gt).unwrap_or_else(|e| panic!("ground truth {gt}: {e}")),
            ),
            split,
        );
        let cap = if name == "synth" {
            TEDS_CAP_SYNTH
        } else {
            TEDS_CAP_SPIKE
        };
        let per_doc: Vec<(&String, Summary)> = gt
            .par_iter()
            .map(|(doc, g)| {
                (
                    doc,
                    evaluate_doc(
                        g,
                        &tables_of(&Path::new(pdfs).join(format!("{doc}.pdf"))),
                        cap,
                    ),
                )
            })
            .collect();
        let mut s = Summary::default();
        let docs: Vec<Summary> = per_doc.iter().map(|(_, d)| *d).collect();
        // Informative second false-page figure without documents with known incomplete ground truth
        // (`TABLES_EXCLUDE=<list>`, default scripts/tables/lists/incomplete_gt.txt); never gated.
        let excluded = incomplete_gt();
        let kept: Vec<Summary> = per_doc
            .iter()
            .filter(|(doc, _)| !excluded.contains(doc.as_str()))
            .map(|(_, d)| *d)
            .collect();
        for (doc, d) in per_doc {
            if std::env::var_os("TABLES_EVAL_VERBOSE").is_some() && d.n + d.neg_pages_fp > d.found {
                println!(
                    "  {doc}: missed {} fp_pages {}",
                    d.n - d.found,
                    d.neg_pages_fp
                );
            }
            s += d;
        }
        println!(
            "{name}/{split}: tables {} found {} ({:.3}) shape {} ({:.3}) teds_all {:.3} | fp pages {}/{} ({:.4}) fp docs {}/{} ({:.4}) excess {}",
            s.n,
            s.found,
            s.found_rate(),
            s.shape,
            s.shape_rate(),
            s.teds_all(),
            s.neg_pages_fp,
            s.neg_pages,
            s.fp_page_rate(),
            s.neg_docs_fp,
            s.neg_docs,
            s.fp_doc_rate(),
            s.excess
        );
        let ci = bootstrap(&docs, 500, 1);
        println!(
            "  CI95 found {:.3}-{:.3} teds {:.3}-{:.3} fp pages {:.4}-{:.4} fp docs {:.4}-{:.4}",
            ci.found.0,
            ci.found.1,
            ci.teds_all.0,
            ci.teds_all.1,
            ci.fp_page.0,
            ci.fp_page.1,
            ci.fp_doc.0,
            ci.fp_doc.1
        );
        if name == "pt1m" && kept.len() < docs.len() {
            let mut k = Summary::default();
            for d in &kept {
                k += *d;
            }
            let kci = bootstrap(&kept, 500, 1);
            println!(
                "  without {} docs with incomplete ground truth: fp pages {}/{} ({:.4}, CI95 {:.4}-{:.4})",
                docs.len() - kept.len(),
                k.neg_pages_fp,
                k.neg_pages,
                k.fp_page_rate(),
                kci.fp_page.0,
                kci.fp_page.1
            );
        }
        if let Some(g) = gate(name).filter(|_| D10.contains(&[name, split])) {
            let fp = if g.fp_per_doc {
                s.fp_doc_rate()
            } else {
                s.fp_page_rate()
            };
            if s.found_rate() < g.found || s.teds_all() < g.teds || fp > g.fp {
                failed.push(format!("{name}/{split}"));
            }
        }
    }
    if !gated && !failed.is_empty() {
        eprintln!("D10 gate not met (TABLES_GATE_OFF): {failed:?}");
    }
    assert!(failed.is_empty() || !gated, "D10 gate not met: {failed:?}");
}
