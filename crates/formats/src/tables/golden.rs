//! Parity with the Python prototype: committed golden fixtures (synthetic, generated text only) and an
//! ignored full-corpus comparison against `table_golden.py` output kept outside the repo.

use super::PageInput;
use super::detect::{Candidate, CandidateKind, candidates};
use super::prims::{prims_with, words_of};
use super::select::{LOGIT_THRESHOLD, RejectReason, filter_reason, probability, use_planar};
use serde_json::Value;

/// Features are rounded to 4 decimals on both sides; one unit of the last digit may differ.
const FEAT_TOL: f64 = 1.5e-4;
/// Last-digit feature differences move the probability by up to ~5e-5 (measured on 7230 pages).
const P_TOL: f64 = 1e-4;

use super::dump::{f, page_from_dump};

#[cfg(feature = "tables-eval")]
mod metrics;
#[cfg(feature = "tables-eval")]
// Read only by the OCR scan harness (`pdf::ocr_stage_scan_eval`, feature `ocr`).
#[allow(unused_imports)]
pub(crate) use metrics::{eval_cfg, eval_rows, score_scan};

/// `TABLES_RASTER=1`: the scan geometry dumps carry no raster marker, so the corpus harnesses mark every
/// segment raster themselves (Task 12(c)).
fn raster_env() -> bool {
    std::env::var("TABLES_RASTER").is_ok_and(|v| v == "1")
}

/// Dump page as detector input, every segment marked raster when `raster`.
fn page_input(p: &Value, raster: bool) -> PageInput {
    let mut page = page_from_dump(p);
    for s in &mut page.segments {
        s.raster = raster;
    }
    page
}

fn reason_name(r: Option<RejectReason>) -> Option<&'static str> {
    r.map(|r| match r {
        RejectReason::Chart => "chart",
        RejectReason::Tiny => "tiny",
        RejectReason::Sparse => "sparse",
        RejectReason::TwoColumnProse => "twocol",
        RejectReason::LowProbability => "logit",
        RejectReason::Rotated => "rotated",
        RejectReason::ScanDecoy => "scan_decoy",
        RejectReason::BoxFlow => "box_flow",
    })
}

/// Differences between a Rust candidate and a Python record (empty = parity).
fn diff(c: &Candidate, py: &Value, cfg: &super::TableConfig) -> Vec<String> {
    let mut d = Vec::new();
    let kind = if c.kind == CandidateKind::Lined {
        "L"
    } else {
        "T"
    };
    if py["kind"] != kind {
        d.push(format!("kind {kind} vs {}", py["kind"]));
        return d;
    }
    let rows: Vec<Vec<String>> = serde_json::from_value(py["rows"].clone()).unwrap();
    if rows != c.rows {
        d.push(format!("rows {:?} vs {:?}", c.rows, rows));
    }
    for (name, v) in c.features.named() {
        let pv = f(&py["feat"][name]);
        if (v - pv).abs() > FEAT_TOL {
            d.push(format!("feat {name} {v} vs {pv}"));
        }
    }
    let p = probability(&c.features, c.kind);
    if (p - f(&py["p"])).abs() > P_TOL {
        d.push(format!("p {p} vs {}", py["p"]));
    }
    let reason = reason_name(filter_reason(c, p, cfg, LOGIT_THRESHOLD));
    if reason != py["reject"].as_str() {
        d.push(format!("reject {reason:?} vs {}", py["reject"]));
    }
    if use_planar(c) != py["use_planar"].as_bool().unwrap() {
        d.push(format!(
            "use_planar {} vs {}",
            use_planar(c),
            py["use_planar"]
        ));
    }
    match (&c.planar, py["planar"].is_null()) {
        (None, true) => {}
        (Some(pl), false) => {
            let pp = &py["planar"];
            let prow: Vec<Vec<(String, u32, u32)>> = pp["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    r.as_array()
                        .unwrap()
                        .iter()
                        .map(|c| {
                            (
                                c["t"].as_str().unwrap().to_string(),
                                f(&c["cs"]) as u32,
                                f(&c["rs"]) as u32,
                            )
                        })
                        .collect()
                })
                .collect();
            let rrow: Vec<Vec<(String, u32, u32)>> = pl
                .rows
                .iter()
                .map(|r| r.iter().map(|c| (c.text.clone(), c.cs, c.rs)).collect())
                .collect();
            if prow != rrow {
                d.push("planar rows".into());
            }
            let stats = (
                pl.nrows as f64,
                pl.ncols as f64,
                pl.ncells as f64,
                pl.nonrect as f64,
            );
            if stats
                != (
                    f(&pp["nrows"]),
                    f(&pp["ncols"]),
                    f(&pp["ncells"]),
                    f(&pp["nonrect"]),
                )
                || (pl.edge_cov - f(&pp["edge_cov"])).abs() > 1e-9
            {
                d.push(format!("planar stats {stats:?} cov {}", pl.edge_cov));
            }
        }
        (a, _) => d.push(format!(
            "planar presence {} vs {}",
            a.is_some(),
            !py["planar"].is_null()
        )),
    }
    d
}

/// Compares one page; returns `(candidates compared, mismatching candidates with reasons)`.
fn compare_page(input: &PageInput, py: &[Value], cfg: &super::TableConfig) -> (usize, Vec<String>) {
    let ws = words_of(input);
    let mut p = prims_with(input, cfg.short_vertical_stacks);
    if cfg.merge_lines {
        super::prims::merge_collinear(&mut p);
    }
    let cands = candidates(input, &ws, &p, cfg).expect("corpus pages are within the limits");
    if cands.len() != py.len() {
        return (
            py.len().max(cands.len()),
            vec![format!("count {} vs {}", cands.len(), py.len())],
        );
    }
    let bad = cands.iter().zip(py).filter_map(|(c, p)| {
        let d = diff(c, p, cfg);
        (!d.is_empty()).then(|| d.join("; "))
    });
    (py.len(), bad.collect())
}

#[test]
fn golden_fixtures_match_python() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/tables/golden.json"
    );
    let pages: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(pages.len(), 12, "fixture set changed");
    let mut total = 0;
    for pg in &pages {
        let (n, bad) = compare_page(
            &page_from_dump(&pg["input"]),
            pg["candidates"].as_array().unwrap(),
            &super::TableConfig::reference(),
        );
        assert!(bad.is_empty(), "{}:{} {bad:?}", pg["doc"], pg["page"]);
        total += n;
    }
    assert!(total >= 13, "only {total} candidates compared");
}

/// Task 22a: `fs_tolerances` is off in the shipped and reference configs, and the reference with the
/// flag explicitly off still matches the SP4 Python records of the golden pages byte for byte (rows,
/// features, probability, reject reason), as `golden_fixtures_match_python`.
#[test]
fn fs_tolerances_off_changes_nothing() {
    use super::TableConfig;
    assert!(!TableConfig::default().fs_tolerances && !TableConfig::reference().fs_tolerances);
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/tables/golden.json"
    );
    let pages: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let off = TableConfig {
        fs_tolerances: false,
        ..TableConfig::reference()
    };
    let mut total = 0;
    for pg in &pages {
        let (n, bad) = compare_page(
            &page_from_dump(&pg["input"]),
            pg["candidates"].as_array().unwrap(),
            &off,
        );
        assert!(bad.is_empty(), "{}:{} {bad:?}", pg["doc"], pg["page"]);
        total += n;
    }
    assert!(total >= 13, "only {total} candidates compared");
}

/// `TABLES_GEOM=<dump dir> TABLES_PYREF=<table_golden.py out dir> cargo test ... -- --ignored corpus_parity`
/// (`TABLES_RASTER=1` marks every dump segment raster, Task 12(c)).
#[test]
#[ignore = "needs TABLES_GEOM and TABLES_PYREF (corpora are kept outside the repo)"]
fn corpus_parity() {
    let (Ok(geom), Ok(pyref)) = (std::env::var("TABLES_GEOM"), std::env::var("TABLES_PYREF"))
    else {
        eprintln!("skip: set TABLES_GEOM and TABLES_PYREF (docs/tables-eval.md)");
        return;
    };
    // Diagnostics (not gates); proto2's rule groups are not a byte-exact port of Rust's.
    // `TABLES_CFG=default`: the shipped config without the SP4 follow-up rules, against proto2.py records
    // with `{rule_groups, merge_lines, rg_extend, dense_valleys, rg_split, comp_lines}` on (A3 oracle).
    // The shipped config also has `fill_groups` + `fill_split` (no proto2 equivalent) and the T14-T17
    // additions, so pages with fill stacks, wide, joined or borderless text blocks differ; the exact
    // oracles are the explicit modes below (`c14` is the shipped set without `fill_split`).
    // `TABLES_CFG=a2all`: the A2 oracle, every switch explicit, against proto2.py `run.py` with
    // `{"rule_groups","merge_lines","fill_groups","rg_extend","dense_valleys","rg_split"}` on (comp_lines off).
    // `TABLES_CFG=t14`: the T14 oracle, every switch explicit: the shipped set without `fill_split` (no
    // proto2 counterpart) and with `text_max_cols: 40, gmin_adapt, axis_guard`, against `run.py` variant
    // `g_v` (`N2` + `{tx_maxcols: 40, gmin_adapt}`); `t14off` is the same with the T14 fields off (`n2b`).
    // `TABLES_CFG=t16`: `t14` plus `text_join`, against `run.py` with `N2` + `g_v` + `g_b` (`tx_join`).
    // `TABLES_CFG=t17`: `t16` plus `row_newcols: 2, header_lines: 3, header_split`, against `run.py` with
    // `N2` + `g_v` + `g_b` + `g_g`; `t17p` the same with `row_newcols: 3` (T17').
    // `TABLES_CFG=c14`: `t17` plus the T15 fields `text_longseg, text_dense, gutter_prose: 3`, against
    // `run.py` variant `c14` (`N2` + all of T14..T17); `c11` the same with `gutter_prose: 1`.
    let t14 = |on: bool| super::TableConfig {
        filters: true,
        logit_threshold: super::select::LOGIT_THRESHOLD,
        hybrid: true,
        split_glued_words: false,
        rule_groups: true,
        merge_lines: true,
        fill_groups: true,
        rg_extend: true,
        dense_valleys: true,
        rg_split: true,
        fill_split: false,
        comp_lines: true,
        extend_first_column: false,
        short_vertical_stacks: false,
        text_max_cols: if on {
            super::WIDE_TEXT_MAX_COLS
        } else {
            super::PROTO_TEXT_MAX_COLS
        },
        gmin_adapt: on,
        axis_guard: true,
        text_join: false,
        row_newcols: 0,
        header_lines: 1,
        header_split: false,
        text_longseg: false,
        text_dense: false,
        gutter_prose: 0,
        // No proto2 counterpart (the guard there is `T`-only).
        frame_prose: false,
        region_model: None,
        used_lined_only: false,
        band_split: false,
        rg_tall_regular: false,
        rg_regular_rows: false,
        rg_fill_header: false,
        rg_split_multiseg: false,
        page_columns: false,
        uncertain_page_columns: false,
        wide_gap_columns: false,
        box_flow: false,
        lined_skip_background: false,
        scan_text_guards: false,
        fs_tolerances: false,
    };
    let t17 = |row_newcols: u8| super::TableConfig {
        text_join: true,
        row_newcols,
        header_lines: super::MAX_HEADER_LINES,
        header_split: true,
        ..t14(true)
    };
    let cfg = match std::env::var("TABLES_CFG").as_deref() {
        Ok("t14") => t14(true),
        Ok("t16") => super::TableConfig {
            text_join: true,
            ..t14(true)
        },
        Ok(m @ ("t17" | "t17p")) => t17(if m == "t17" { 2 } else { 3 }),
        Ok(m @ ("c14" | "c11")) => super::TableConfig {
            text_longseg: true,
            text_dense: true,
            gutter_prose: if m == "c14" { 3 } else { 1 },
            ..t17(2)
        },
        Ok("t14off") => t14(false),
        // proto2 has none of the rules turned on after its time: they stay off in this oracle.
        Ok("default") => super::TableConfig {
            used_lined_only: false,
            rg_regular_rows: false,
            rg_fill_header: false,
            rg_split_multiseg: false,
            lined_skip_background: false,
            ..super::TableConfig::default()
        },
        Ok("a2all") => super::TableConfig {
            filters: true,
            logit_threshold: super::select::LOGIT_THRESHOLD,
            hybrid: true,
            split_glued_words: false,
            rule_groups: true,
            merge_lines: true,
            fill_groups: true,
            rg_extend: true,
            dense_valleys: true,
            rg_split: true,
            fill_split: false,
            comp_lines: false,
            extend_first_column: false,
            short_vertical_stacks: false,
            text_max_cols: super::PROTO_TEXT_MAX_COLS,
            gmin_adapt: false,
            // run.py applies the axis guard unconditionally.
            axis_guard: true,
            text_join: false,
            row_newcols: 0,
            header_lines: 1,
            header_split: false,
            text_longseg: false,
            text_dense: false,
            gutter_prose: 0,
            frame_prose: false,
            region_model: None,
            used_lined_only: false,
            band_split: false,
            rg_tall_regular: false,
            rg_regular_rows: false,
            rg_fill_header: false,
            rg_split_multiseg: false,
            page_columns: false,
            uncertain_page_columns: false,
            wide_gap_columns: false,
            box_flow: false,
            lined_skip_background: false,
            scan_text_guards: false,
            fs_tolerances: false,
        },
        _ => super::TableConfig::reference(),
    };
    let raster = raster_env();
    let (mut pages, mut cands, mut bad) = (0usize, 0usize, Vec::new());
    let mut docs: Vec<_> = std::fs::read_dir(&pyref)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    docs.sort();
    for py_path in docs {
        let name = py_path.file_name().unwrap().to_owned();
        let py: Vec<Vec<Value>> =
            serde_json::from_str(&std::fs::read_to_string(&py_path).unwrap()).unwrap();
        let dump: Value = serde_json::from_str(
            &std::fs::read_to_string(std::path::Path::new(&geom).join(&name)).unwrap(),
        )
        .unwrap();
        for (i, (p, recs)) in dump["pages"]
            .as_array()
            .unwrap()
            .iter()
            .zip(&py)
            .enumerate()
        {
            let (n, b) = compare_page(&page_input(p, raster), recs, &cfg);
            pages += 1;
            cands += n;
            bad.extend(
                b.into_iter()
                    .map(|m| format!("{}:{i} {m}", name.to_string_lossy())),
            );
        }
    }
    println!(
        "parity: {pages} pages, {cands} candidates, {} mismatches",
        bad.len()
    );
    for m in &bad {
        println!("  {m}");
    }
    // Exact-threshold ties (e.g. |Δcy| == 0.5·fs in line grouping) may resolve differently because the
    // centre is computed from top-left instead of bottom-left coordinates: PT1M 2 of 1219, c150 1 of 179.
    // Readiness criterion 2: at most max(2, 0.2 %) per corpus, each one a tie explained in tables-eval.md.
    assert!(
        bad.len() <= (cands / 500).max(2),
        "{} mismatches > max(2, 0.2% of {cands})",
        bad.len()
    );
}

#[test]
fn nested_fixture_keeps_inner_text_once() {
    use super::{TableConfig, detect_tables};
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/tables/golden.json"
    );
    let pages: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let pg = pages.iter().find(|p| p["doc"] == "syn_324_nested").unwrap();
    let t = detect_tables(&page_from_dump(&pg["input"]), &TableConfig::default()).tables;
    assert_eq!(t.len(), 1, "two inner tables nested into the outer one");
    let cells: Vec<_> = t[0].rows.iter().flatten().collect();
    assert_eq!(cells.iter().map(|c| c.nested.len()).sum::<usize>(), 2);
    let all: String = cells
        .iter()
        .map(|c| c.flat_text())
        .collect::<Vec<_>>()
        .join(" ");
    let inner_word = t[0]
        .rows
        .iter()
        .flatten()
        .flat_map(|c| &c.nested)
        .next()
        .unwrap()
        .rows[1][0]
        .text
        .clone();
    assert_eq!(all.matches(inner_word.as_str()).count(), 1, "{inner_word}");
}

/// SP4-R1: with the region model on and no regions (or regions given with the model off) the golden pages
/// give exactly the output of the shipped and reference configs.
#[test]
fn without_model_regions_output_is_unchanged() {
    use super::{ModelRegion, Rect, RegionModelConfig, TableConfig, detect_tables};
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/tables/golden.json"
    );
    let pages: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut tables = 0;
    for base in [TableConfig::default(), TableConfig::reference()] {
        assert_eq!(base.region_model, None, "off by default");
        let on = TableConfig {
            region_model: Some(RegionModelConfig::default()),
            ..base.clone()
        };
        for pg in &pages {
            let page = page_from_dump(&pg["input"]);
            let want = format!("{:?}", detect_tables(&page, &base));
            assert_eq!(
                format!("{:?}", detect_tables(&page, &on)),
                want,
                "{}",
                pg["doc"]
            );
            let mut regions = page.clone();
            regions.model_regions = vec![ModelRegion::new(
                Rect::new(0.0, 0.0, page.width, page.height),
                1.0,
            )];
            assert_eq!(
                format!("{:?}", detect_tables(&regions, &base)),
                want,
                "{}",
                pg["doc"]
            );
            tables += detect_tables(&page, &base).tables.len();
        }
    }
    assert!(tables >= 13, "only {tables} tables compared");
}
