//! Corpus metrics harnesses (`tables-eval`): D11 metrics against the Python records and the scan
//! geometry scorer shared with the OCR scan harness.

use super::{f, page_from_dump, page_input, raster_env};
use serde_json::Value;

/// Corpus metrics from dumps, Rust D11 pipeline vs the Python records (`table_golden.py`):
/// `TABLES_GEOM=<dumps> TABLES_PYREF=<records> TABLES_GT=<unified gt.json> [TABLES_TEDS_CAP=4000000]`.
#[cfg(feature = "tables-eval")]
#[test]
#[ignore = "needs TABLES_GEOM, TABLES_PYREF and TABLES_GT"]
fn corpus_metrics_match_python() {
    use crate::tables::eval::{EvalCell, EvalRows, Summary, evaluate_doc, load_gt};
    use crate::tables::{TableConfig, detect_tables};
    if ["TABLES_GEOM", "TABLES_PYREF", "TABLES_GT"]
        .iter()
        .any(|k| std::env::var(k).is_err())
    {
        eprintln!("skip: set TABLES_GEOM, TABLES_PYREF and TABLES_GT (docs/tables-eval.md)");
        return;
    }
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k}"));
    let part = std::env::var("TABLES_SPLIT").unwrap_or_else(|_| "all".into());
    let gt = crate::tables::eval::split_docs(
        load_gt(&std::fs::read_to_string(env("TABLES_GT")).unwrap()),
        &part,
    );
    let cap: usize = std::env::var("TABLES_TEDS_CAP")
        .map_or(crate::tables::eval::TEDS_CAP_SPIKE, |v| v.parse().unwrap());
    type Preds = Vec<(usize, EvalRows)>;
    // TABLES_CFG=default measures the shipped config (with rule groups) instead of the Python reference.
    let shipped = std::env::var("TABLES_CFG").is_ok_and(|v| v == "default");
    let cfg = if shipped {
        TableConfig::default()
    } else {
        TableConfig::reference()
    };
    let cell = |t: &str, cs: u32, rs: u32| EvalCell {
        colspan: cs,
        rowspan: rs,
        text: t.to_string(),
    };
    let (mut rs, mut py) = (Summary::default(), Summary::default());
    let mut pages = 0;
    for (doc, g) in &gt {
        let dump: Value = serde_json::from_str(
            &std::fs::read_to_string(format!("{}/{doc}.json", env("TABLES_GEOM"))).unwrap(),
        )
        .unwrap();
        let recs: Vec<Vec<Value>> = serde_json::from_str(
            &std::fs::read_to_string(format!("{}/{doc}.json", env("TABLES_PYREF"))).unwrap(),
        )
        .unwrap();
        let (mut rp, mut pp): (Preds, Preds) = (Vec::new(), Vec::new());
        for (i, p) in dump["pages"].as_array().unwrap().iter().enumerate() {
            pages += 1;
            for t in detect_tables(&page_from_dump(p), &cfg).tables {
                rp.push((
                    i,
                    t.rows
                        .iter()
                        .map(|r| {
                            r.iter()
                                .map(|c| cell(&c.flat_text(), c.colspan, c.rowspan))
                                .collect()
                        })
                        .collect(),
                ));
            }
            for r in recs[i].iter().filter(|r| r["reject"].is_null()) {
                let rows = if r["use_planar"].as_bool().unwrap() {
                    r["planar"]["rows"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|row| {
                            row.as_array()
                                .unwrap()
                                .iter()
                                .map(|c| {
                                    cell(
                                        c["t"].as_str().unwrap(),
                                        f(&c["cs"]) as u32,
                                        f(&c["rs"]) as u32,
                                    )
                                })
                                .collect()
                        })
                        .collect()
                } else {
                    r["rows"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|row| {
                            row.as_array()
                                .unwrap()
                                .iter()
                                .map(|c| cell(c.as_str().unwrap(), 1, 1))
                                .collect()
                        })
                        .collect()
                };
                pp.push((i, rows));
            }
        }
        rs += evaluate_doc(g, &rp, cap);
        py += evaluate_doc(g, &pp, cap);
    }
    for (name, s) in [("rust", rs), ("python", py)] {
        println!(
            "{name}: docs {} pages {pages} tables {} found {} ({:.3}) shape {} ({:.3}) teds_all {:.4} | fp tables {} pages {}/{} ({:.4}) docs {}/{} ({:.4}) excess {}",
            gt.len(),
            s.n,
            s.found,
            s.found_rate(),
            s.shape,
            s.shape_rate(),
            s.teds_all(),
            s.fp_tables,
            s.neg_pages_fp,
            s.neg_pages,
            s.fp_page_rate(),
            s.neg_docs_fp,
            s.neg_docs,
            s.fp_doc_rate(),
            s.excess
        );
    }
    if shipped {
        return;
    }
    assert_eq!(
        (rs.found, rs.shape, rs.fp_tables),
        (py.found, py.shape, py.fp_tables)
    );
    assert!((rs.teds_all() - py.teds_all()).abs() < 1e-6);
}

/// Defaults turned on after Task 12(c) (T19, SP4-R4 B/C/D, T-bg, two-column prose fix).
#[cfg(feature = "tables-eval")]
const AFTER_12C: [&str; 6] = [
    "frame_prose",
    "used_lined_only",
    "rg_regular_rows",
    "rg_fill_header",
    "rg_split_multiseg",
    "lined_skip_background",
];

/// `TABLES_CFG` of the scan harnesses: `extend` (shipped + `extend_first_column`), `reference`, `scan`
/// (shipped + `short_vertical_stacks`), `fs` (shipped + `fs_tolerances`), `pre_t19` (the [`AFTER_12C`]
/// defaults off), `off:<flag>,...` (those of them off), anything else the shipped config.
#[cfg(feature = "tables-eval")]
pub(crate) fn eval_cfg(spec: &str) -> crate::tables::TableConfig {
    use crate::tables::TableConfig;
    let off: Vec<&str> = match spec {
        "extend" => {
            return TableConfig {
                extend_first_column: true,
                ..TableConfig::default()
            };
        }
        "reference" => return TableConfig::reference(),
        "scan" => {
            return TableConfig {
                short_vertical_stacks: true,
                ..TableConfig::default()
            };
        }
        "fs" => {
            return TableConfig {
                fs_tolerances: true,
                ..TableConfig::default()
            };
        }
        "pre_t19" => AFTER_12C.to_vec(),
        s => s
            .strip_prefix("off:")
            .map_or(Vec::new(), |l| l.split(',').collect()),
    };
    let mut c = TableConfig::default();
    for f in off {
        *match f {
            "frame_prose" => &mut c.frame_prose,
            "used_lined_only" => &mut c.used_lined_only,
            "rg_regular_rows" => &mut c.rg_regular_rows,
            "rg_fill_header" => &mut c.rg_fill_header,
            "rg_split_multiseg" => &mut c.rg_split_multiseg,
            "lined_skip_background" => &mut c.lined_skip_background,
            "uncertain_page_columns" => &mut c.uncertain_page_columns,
            _ => panic!("TABLES_CFG: unknown flag {f}"),
        } = false;
    }
    c
}

/// A detected grid as evaluation rows.
#[cfg(feature = "tables-eval")]
pub(crate) fn eval_rows(t: &crate::tables::DetectedGrid) -> crate::tables::eval::EvalRows {
    use crate::tables::eval::EvalCell;
    t.rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|c| EvalCell {
                    colspan: c.colspan,
                    rowspan: c.rowspan,
                    text: c.flat_text(),
                })
                .collect()
        })
        .collect()
}

/// Scores `preds` (doc -> `(page, rows)`) against `gt` and prints the `all` line, then (with `TABLES_PAGES`)
/// one line per table style, and the table-free pages with a table when `gt` has any.
#[cfg(feature = "tables-eval")]
pub(crate) fn score_scan(
    gt: &std::collections::BTreeMap<String, crate::tables::eval::GtDoc>,
    preds: &std::collections::BTreeMap<String, Vec<(usize, crate::tables::eval::EvalRows)>>,
) {
    use crate::tables::eval::{Summary, TEDS_CAP_SPIKE, evaluate_doc};
    use std::collections::BTreeMap;
    let mut styles: BTreeMap<(String, usize), Vec<String>> = BTreeMap::new();
    if let Ok(path) = std::env::var("TABLES_PAGES") {
        for l in std::fs::read_to_string(path).unwrap().lines() {
            let j: Value = serde_json::from_str(l).unwrap();
            styles.insert(
                (
                    j["doc"].as_str().unwrap().to_string(),
                    j["page"].as_u64().unwrap() as usize,
                ),
                j["styles"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect(),
            );
        }
    }
    let mut all = Summary::default();
    let mut slices: BTreeMap<String, Summary> = BTreeMap::new();
    let none = Vec::new();
    for (doc, g) in gt {
        let preds = preds.get(doc).unwrap_or(&none);
        all += evaluate_doc(g, preds, TEDS_CAP_SPIKE);
        for style in styles
            .iter()
            .filter(|((d, _), _)| d == doc)
            .flat_map(|(_, v)| v.iter())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let on = |pg: &usize| {
                styles
                    .get(&(doc.clone(), *pg))
                    .is_some_and(|v| v.contains(style))
            };
            let sub = crate::tables::eval::GtDoc {
                pages: g.pages,
                tables: g.tables.iter().filter(|t| on(&t.0)).cloned().collect(),
                table_pages: g.table_pages.iter().copied().filter(&on).collect(),
            };
            let sp: Vec<_> = preds.iter().filter(|p| on(&p.0)).cloned().collect();
            *slices.entry(style.clone()).or_default() += evaluate_doc(&sub, &sp, TEDS_CAP_SPIKE);
        }
    }
    let line = |name: &str, s: &Summary| {
        println!(
            "{name}: tables {} found {} ({:.3}) shape {} ({:.3}) teds_all {:.4} | fp tables {} excess {}",
            s.n,
            s.found,
            s.found_rate(),
            s.shape,
            s.shape_rate(),
            s.teds_all(),
            s.fp_tables,
            s.excess
        );
    };
    line("all", &all);
    for (k, v) in &slices {
        line(&format!("  style {k}"), v);
    }
    if all.neg_pages > 0 {
        println!(
            "table-free pages {} with a table {} ({:.3})",
            all.neg_pages,
            all.neg_pages_fp,
            all.fp_page_rate()
        );
    }
}

/// Scan geometry (SP5 spike dumps, `S/ocr-tables/geom/<variant>`) against `gt_scan.json`; no Python record:
/// `TABLES_GEOM=<dump dir> TABLES_GT=<gt_scan.json> [TABLES_PAGES=<pages.jsonl>] [TABLES_CFG=...]`
/// ([`eval_cfg`]). With `TABLES_PAGES` also prints the slice of pages carrying each table style
/// (`rounded_cells`, ...).
#[cfg(feature = "tables-eval")]
#[test]
#[ignore = "needs TABLES_GEOM and TABLES_GT"]
fn scan_metrics() {
    use crate::tables::detect_tables;
    use crate::tables::eval::load_gt;
    let (Ok(geom), Ok(gt_path)) = (std::env::var("TABLES_GEOM"), std::env::var("TABLES_GT")) else {
        eprintln!("skip: set TABLES_GEOM and TABLES_GT (docs/tables-eval.md)");
        return;
    };
    let cfg = eval_cfg(&std::env::var("TABLES_CFG").unwrap_or_default());
    let raster = raster_env();
    let gt = load_gt(&std::fs::read_to_string(gt_path).unwrap());
    let preds = gt
        .keys()
        .map(|doc| {
            let dump: Value = serde_json::from_str(
                &std::fs::read_to_string(format!("{geom}/{doc}.json")).unwrap(),
            )
            .unwrap();
            let mut p = Vec::new();
            for (i, pg) in dump["pages"].as_array().unwrap().iter().enumerate() {
                for t in detect_tables(&page_input(pg, raster), &cfg).tables {
                    p.push((i, eval_rows(&t)));
                }
            }
            (doc.clone(), p)
        })
        .collect();
    println!(
        "scan {geom} docs {} tables incl. excluded {}",
        gt.len(),
        gt.values().map(|g| g.table_pages.len()).sum::<usize>()
    );
    score_scan(&gt, &preds);
}
