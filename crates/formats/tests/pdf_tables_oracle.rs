//! SP4-R1 / SP4-R3 region-model oracle gate (see docs/tables-eval.md); split from `pdf_tables_eval.rs`.

use plaindoc_formats::tables::eval::{
    EvalCell, EvalRows, GtDoc, Summary, TEDS_CAP_SPIKE, bootstrap, evaluate_doc, load_gt,
};
use plaindoc_formats::tables::{
    ModelRegion, Rect, RegionModelConfig, TableConfig, detect_tables, dump::page_from_dump,
};

/// Per-table `(found, teds)` of `evaluate_doc`: its greedy matching goes in ground-truth order, so table `k`
/// is decided by the first `k + 1` tables alone and the difference of two prefix runs isolates it.
fn per_table(g: &GtDoc, preds: &[(usize, EvalRows)], cap: usize) -> Vec<(bool, f64)> {
    let prefix = |k: usize| {
        let sub = GtDoc {
            pages: g.pages,
            tables: g.tables[..k].to_vec(),
            table_pages: g.table_pages.clone(),
        };
        evaluate_doc(&sub, preds, cap)
    };
    let mut prev = prefix(0);
    (1..=g.tables.len())
        .map(|k| {
            let cur = prefix(k);
            let r = (cur.found > prev.found, cur.teds_sum - prev.teds_sum);
            prev = cur;
            r
        })
        .collect()
}

/// SP4-R1 oracle (docs/tables-eval.md, "SP4-R1 model regions (oracle)"): the ground-truth table regions of
/// the region spike's digital set fed as `model_regions` (score 1) with the region model on, against the
/// same pages with it off.
/// `TABLES_ORACLE=<geometry dumps (S/ocr-tables/geom/digital)>:<gt_scan.json>:<gt_regions.json>`.
/// Gates (flag on): borderless found >= 80 %, TEDS_all >= 0.80, excess <= 35 on the 158 scored tables; with
/// `inner_bounds` (SP4-R3, docs "SP4-R3") borderless found >= 90 %, TEDS_all >= 0.80 and not worse than R1.
#[test]
#[ignore = "needs TABLES_ORACLE (corpora are kept outside the repo)"]
fn oracle_regions_gate() {
    let Ok(spec) = std::env::var("TABLES_ORACLE") else {
        eprintln!("skip: set TABLES_ORACLE=<geom dir>:<gt_scan.json>:<gt_regions.json>");
        return;
    };
    let parts: Vec<&str> = spec.split(':').collect();
    let [geom, gt, regions] = parts[..] else {
        panic!("TABLES_ORACLE={spec:?}: expected <geom dir>:<gt_scan.json>:<gt_regions.json>");
    };
    let gt = load_gt(&std::fs::read_to_string(gt).unwrap_or_else(|e| panic!("{gt}: {e}")));
    let regions: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(regions).unwrap_or_else(|e| panic!("{regions}: {e}")),
    )
    .expect("gt_regions.json");
    let tokens = |rows: &EvalRows| -> usize {
        rows.iter()
            .flatten()
            .map(|c| c.text.split_whitespace().count())
            .sum()
    };
    let on = TableConfig {
        region_model: Some(RegionModelConfig::default()),
        ..TableConfig::default()
    };
    // SP4-R3: zebra rows and edge-cluster columns inside the regions (`inner_bounds`, off by default).
    let mut r3 = RegionModelConfig::default();
    r3.inner_bounds = true;
    let r3 = TableConfig {
        region_model: Some(r3),
        ..TableConfig::default()
    };
    let mut result = Vec::new();
    for (name, cfg) in [("off", TableConfig::default()), ("on", on), ("on+R3", r3)] {
        let (mut all, mut bl) = (Vec::new(), Vec::new());
        for (doc, g) in &gt {
            let dump: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(format!("{geom}/{doc}.json"))
                    .unwrap_or_else(|e| panic!("{geom}/{doc}.json: {e}")),
            )
            .unwrap();
            let mut preds = Vec::new();
            for (i, p) in dump["pages"].as_array().unwrap().iter().enumerate() {
                let mut page = page_from_dump(p);
                let reg = &regions[format!("{doc}_p{i}")]["tables"];
                page.model_regions = reg
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|t| {
                        let b: Vec<f64> = serde_json::from_value(t["bbox"].clone()).unwrap();
                        ModelRegion::new(Rect::new(b[0], b[1], b[2] - b[0], b[3] - b[1]), 1.0)
                    })
                    .collect();
                for t in detect_tables(&page, &cfg).tables {
                    let rows = t
                        .rows
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
                        .collect();
                    preds.push((i, rows));
                }
            }
            all.push(evaluate_doc(g, &preds, TEDS_CAP_SPIKE));
            // Borderless flag of each scored table: the region of the same page with the same token count
            // (gt_scan.json and gt_regions.json order the tables of a page differently on 9 pages).
            let mut used = std::collections::HashSet::new();
            let mut b = Summary::default();
            for ((page, rows), (found, teds)) in
                g.tables.iter().zip(per_table(g, &preds, TEDS_CAP_SPIKE))
            {
                let list = regions[format!("{doc}_p{page}")]["tables"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let n = tokens(rows) as i64;
                let k = (0..list.len())
                    .filter(|k| !used.contains(&(*page, *k)))
                    .min_by_key(|&k| (list[k]["n_tokens"].as_i64().unwrap_or(0) - n).abs())
                    .expect("a region per scored table");
                used.insert((*page, k));
                if list[k]["borderless"].as_bool().unwrap_or(false) {
                    b.n += 1;
                    b.found += usize::from(found);
                    b.teds_sum += teds;
                }
            }
            bl.push(b);
        }
        let sum = |v: &[Summary]| {
            let mut s = Summary::default();
            for d in v {
                s += *d;
            }
            s
        };
        let (s, b) = (sum(&all), sum(&bl));
        let (ci, bci) = (bootstrap(&all, 500, 1), bootstrap(&bl, 500, 1));
        println!(
            "oracle {name}: tables {} found {} ({:.3}, CI95 {:.3}-{:.3}) shape {} teds_all {:.4} (CI95 {:.3}-{:.3}) excess {} fp tables {} | borderless {} found {} ({:.3}, CI95 {:.3}-{:.3}) teds {:.4} (CI95 {:.3}-{:.3})",
            s.n,
            s.found,
            s.found_rate(),
            ci.found.0,
            ci.found.1,
            s.shape,
            s.teds_all(),
            ci.teds_all.0,
            ci.teds_all.1,
            s.excess,
            s.fp_tables,
            b.n,
            b.found,
            b.found_rate(),
            bci.found.0,
            bci.found.1,
            b.teds_all(),
            bci.teds_all.0,
            bci.teds_all.1
        );
        result.push((s, b));
    }
    let (s, b) = result[1];
    assert!(
        b.found_rate() >= 0.80 && s.teds_all() >= 0.80 && s.excess <= 35,
        "SP4-R1 oracle gate not met: borderless found {:.3}, TEDS_all {:.4}, excess {}",
        b.found_rate(),
        s.teds_all(),
        s.excess
    );
    let (r1, rb1) = result[1];
    let (s, b) = result[2];
    assert!(
        b.found_rate() >= 0.90
            && s.teds_all() >= 0.80
            && s.found >= r1.found
            && b.found >= rb1.found
            && s.teds_all() >= r1.teds_all() - 1e-9,
        "SP4-R3 oracle gate not met (or worse than R1): borderless found {:.3}, TEDS_all {:.4}",
        b.found_rate(),
        s.teds_all()
    );
}
