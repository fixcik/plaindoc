//! T18 pre-model page gate on geometry dumps (see docs/tables-eval.md); split from `pdf_tables_eval.rs`.

use plaindoc_formats::tables::{
    TableConfig, detect_tables, dump::page_from_dump, gate_features, may_contain_table,
};

/// T18 pre-model gate on geometry dumps (docs/tables-eval.md, "T18 pre-model gate"):
/// `TABLES_GATE=<name>:<dump dir>:<labels.jsonl>`, several separated by `;`. The labels are the per-page
/// rows of `S/idea-tables2/scripts/gate_feats.py` (`doc, page, label, split` and the Python features);
/// rows of `split == "test"` are not scored (the test splits are run once, later). Prints recall of "may
/// contain a table", the share of table-free and of all pages passed to the model, the gate's own cost
/// and the feature differences against Python (`nw, n_h, n_v, run_g2`).
#[test]
#[ignore = "needs TABLES_GATE (corpora are kept outside the repo)"]
fn page_gate_on_dumps() {
    let Ok(spec) = std::env::var("TABLES_GATE") else {
        eprintln!("skip: set TABLES_GATE=<name>:<dump dir>:<labels.jsonl>[;...]");
        return;
    };
    let cfg = TableConfig::default();
    for item in spec.split(';').filter(|s| !s.is_empty()) {
        let parts: Vec<&str> = item.split(':').collect();
        let [name, geom, labels] = parts[..] else {
            panic!("TABLES_GATE item {item:?}: expected <name>:<dump dir>:<labels.jsonl>");
        };
        let rows: Vec<serde_json::Value> = std::fs::read_to_string(labels)
            .unwrap_or_else(|e| panic!("{labels}: {e}"))
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .filter(|r: &serde_json::Value| r["split"] != "test")
            .collect();
        // group -> (positives, passed positives, negatives, passed negatives)
        let mut acc: std::collections::BTreeMap<String, [usize; 4]> = Default::default();
        let (mut feat_diff, mut misses, mut ms) = ([0usize; 4], Vec::new(), Vec::new());
        let mut dumps: std::collections::HashMap<String, Option<serde_json::Value>> =
            Default::default();
        for r in &rows {
            let doc = r["doc"].as_str().unwrap();
            let dump = dumps.entry(doc.to_string()).or_insert_with(|| {
                std::fs::read_to_string(format!("{geom}/{doc}.json"))
                    .ok()
                    .map(|s| serde_json::from_str(&s).unwrap())
            });
            let Some(dump) = dump else { continue };
            let i = r["page"].as_u64().unwrap() as usize;
            let page = page_from_dump(&dump["pages"][i]);
            let found = detect_tables(&page, &cfg);
            let t = std::time::Instant::now();
            let pass = may_contain_table(&page, &found);
            ms.push(t.elapsed().as_secs_f64() * 1e3);
            let f = gate_features(&page).unwrap_or_default();
            let py = |k: &str| r[k].as_u64().unwrap_or(0) as usize;
            for (d, (a, b)) in feat_diff.iter_mut().zip([
                (f.words, py("nw")),
                (f.h_rulings, py("n_h")),
                (f.v_rulings, py("n_v")),
                (f.aligned_run, py("run_g2")),
            ]) {
                *d += usize::from(a != b);
            }
            if f.aligned_run != py("run_g2") {
                eprintln!(
                    "{name} {doc}:{i}: run_g2 {} vs Python {}",
                    f.aligned_run,
                    py("run_g2")
                );
            }
            let group = if r["split"] == "decoy" {
                "decoy".to_string()
            } else {
                name.to_string()
            };
            let a = acc.entry(group).or_default();
            if r["label"] == 1 {
                a[0] += 1;
                a[1] += usize::from(pass);
                if !pass {
                    misses.push(format!("{doc}:{i} {f:?}"));
                }
            } else {
                a[2] += 1;
                a[3] += usize::from(pass);
            }
        }
        for (g, a) in &acc {
            println!(
                "gate {g}: recall {}/{} = {:.3}, passed table-free {}/{} = {:.3}, passed all {}/{} = {:.3}",
                a[1],
                a[0],
                a[1] as f64 / a[0].max(1) as f64,
                a[3],
                a[2],
                a[3] as f64 / a[2].max(1) as f64,
                a[1] + a[3],
                a[0] + a[2],
                (a[1] + a[3]) as f64 / (a[0] + a[2]).max(1) as f64
            );
        }
        ms.sort_by(f64::total_cmp);
        let n = ms.len().max(1);
        println!(
            "gate {name}: {} pages, gate cost mean {:.4} ms, median {:.4}, p99 {:.4}, max {:.3}; features differing from Python (nw, n_h, n_v, run_g2): {feat_diff:?}; misses {misses:?}",
            ms.len(),
            ms.iter().sum::<f64>() / n as f64,
            ms.get(ms.len() / 2).copied().unwrap_or(0.0),
            ms.get(ms.len() * 99 / 100).copied().unwrap_or(0.0),
            ms.last().copied().unwrap_or(0.0)
        );
    }
}
