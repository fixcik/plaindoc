//! Task 14a: recall gate of the need-OCR classifier on Rust features (research PDFs stay outside
//! the repository).
//!
//! `NEED_OCR_EVAL_SET` is a JSONL manifest, one page per line:
//! `{"set": "r2_test_half" | "cross63", "path", "page", "label": "yes" | "no", "stratum", "born_digital"}`.
//! The gate is decided on the shipped configuration (`DiagnosticsConfig::default()`: `RECALL` with
//! the bundled bigram table). The run is repeated without the table (information only: the
//! degraded build without `data/need_ocr/bigrams.json`).
//! `NEED_OCR_EVAL_OUT` (optional) receives one JSON line per page and configuration.
//!
//! ```sh
//! NEED_OCR_EVAL_SET=… cargo test --release -p plaindoc-formats --features ocr \
//!     --test need_ocr_eval -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use plaindoc_formats::pdf::{Bigrams, DiagnosticsConfig, eval_page};
use serde_json::{Value, json};

struct Row {
    set: String,
    path: String,
    page: usize,
    yes: bool,
    stratum: String,
    born_digital: bool,
}

/// Wilson 95 % interval of `k / n`.
fn wilson(k: usize, n: usize) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let (z, n, p) = (1.96f64, n as f64, k as f64 / n as f64);
    let d = 1.0 + z * z / n;
    let c = (p + z * z / (2.0 * n)) / d;
    let h = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / d;
    ((c - h).max(0.0), (c + h).min(1.0))
}

fn ratio(k: usize, n: usize) -> String {
    let (lo, hi) = wilson(k, n);
    let v = if n == 0 {
        f64::NAN
    } else {
        k as f64 / n as f64
    };
    format!("{k}/{n} = {v:.3} [{lo:.2}, {hi:.2}]")
}

#[derive(Default)]
struct Counts {
    tp: usize,
    fn_: usize,
    fp: usize,
    tn: usize,
}

impl Counts {
    fn add(&mut self, yes: bool, pred: bool) {
        match (yes, pred) {
            (true, true) => self.tp += 1,
            (true, false) => self.fn_ += 1,
            (false, true) => self.fp += 1,
            (false, false) => self.tn += 1,
        }
    }
}

/// Recall of `set` and born-digital false-trigger rate.
struct SetResult {
    recall: (usize, usize),
    ftr_bd: (usize, usize),
}

fn run(
    rows: &[Row],
    bigrams: Option<&Bigrams>,
    name: &str,
    out: &mut String,
) -> BTreeMap<String, SetResult> {
    let c = DiagnosticsConfig::default()
        .need_ocr
        .expect("classifier on by default");
    let mut by_set: BTreeMap<String, Vec<(&Row, bool)>> = BTreeMap::new();
    let (mut misses, mut skipped) = (Vec::new(), 0);
    for r in rows {
        let e = match eval_page(Path::new(&r.path), r.page, bigrams, &c) {
            Ok(e) => e,
            Err(e) if e == "unsupported file" => {
                skipped += 1;
                continue;
            }
            Err(e) => panic!("{} p{}: {e}", r.path, r.page),
        };
        if e.needs_ocr != r.yes {
            misses.push(format!(
                "  {:14} {:24} label={} -> {} {} p{}\n      {}",
                r.set,
                r.stratum,
                if r.yes { "yes" } else { "no" },
                e.reason,
                r.path,
                r.page,
                e.signals
            ));
        }
        out.push_str(
            &json!({"config": name, "set": r.set, "path": r.path, "page": r.page, "label": r.yes,
                "stratum": r.stratum, "needs_ocr": e.needs_ocr, "reason": e.reason, "signals": e.signals})
            .to_string(),
        );
        out.push('\n');
        by_set
            .entry(r.set.clone())
            .or_default()
            .push((r, e.needs_ocr));
    }
    println!("\n===== config {name} =====");
    if skipped > 0 {
        println!("skipped {skipped} unsupported file(s): not counted in any metric");
    }
    let mut res = BTreeMap::new();
    for (set, v) in &by_set {
        let (mut all, mut bd) = (Counts::default(), Counts::default());
        let mut strata: BTreeMap<&str, Counts> = BTreeMap::new();
        for (r, pred) in v {
            all.add(r.yes, *pred);
            if r.born_digital {
                bd.add(r.yes, *pred);
            }
            strata.entry(&r.stratum).or_default().add(r.yes, *pred);
        }
        let acc = all.tp + all.tn;
        println!("--- {set}: {} pages", v.len());
        println!("  recall     {}", ratio(all.tp, all.tp + all.fn_));
        println!("  precision  {}", ratio(all.tp, all.tp + all.fp));
        println!("  accuracy   {}", ratio(acc, v.len()));
        println!("  FTR born-digital {}", ratio(bd.fp, bd.fp + bd.tn));
        println!("  FTR all negatives {}", ratio(all.fp, all.fp + all.tn));
        println!(
            "  {:24} {:>4} {:>28} {:>28}",
            "stratum", "n", "recall (yes)", "FTR (no)"
        );
        for (s, k) in &strata {
            let n = k.tp + k.fn_ + k.fp + k.tn;
            let opt = |k: usize, n: usize| if n > 0 { ratio(k, n) } else { "-".into() };
            let (rec, ftr) = (opt(k.tp, k.tp + k.fn_), opt(k.fp, k.fp + k.tn));
            println!("  {s:24} {n:>4} {rec:>28} {ftr:>28}");
        }
        res.insert(
            set.clone(),
            SetResult {
                recall: (all.tp, all.tp + all.fn_),
                ftr_bd: (bd.fp, bd.fp + bd.tn),
            },
        );
    }
    println!("--- misclassified ({}):", misses.len());
    for m in &misses {
        println!("{m}");
    }
    res
}

fn load(path: &str) -> Vec<Row> {
    let s = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            let yes = match v["label"].as_str() {
                Some("yes") => true,
                Some("no") => false,
                other => panic!("label must be \"yes\" or \"no\", got {other:?}: {l}"),
            };
            Row {
                set: v["set"].as_str().unwrap().into(),
                path: v["path"].as_str().unwrap().into(),
                page: v["page"].as_u64().unwrap() as usize,
                yes,
                stratum: v["stratum"].as_str().unwrap().into(),
                born_digital: v["born_digital"].as_bool().unwrap(),
            }
        })
        .collect()
}

#[test]
#[ignore = "needs NEED_OCR_EVAL_SET"]
fn need_ocr_recall_gate() {
    let set = std::env::var("NEED_OCR_EVAL_SET")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            panic!("NEED_OCR_EVAL_SET is not set: give the JSONL manifest of the evaluation pages")
        });
    let rows = load(&set);
    assert!(
        rows.iter().any(|r| r.set == "r2_test_half"),
        "no r2_test_half rows"
    );
    assert!(rows.iter().any(|r| r.set == "cross63"), "no cross63 rows");
    let mut out = String::new();

    run(&rows, None, "without bigram table (information)", &mut out);
    let table = DiagnosticsConfig::default().bigrams;
    let name = match table {
        Some(_) => "shipped (bundled bigram table)",
        None => "shipped (built without the bigram table)",
    };
    let shipped = run(&rows, table.as_deref(), name, &mut out);
    if let Ok(p) = std::env::var("NEED_OCR_EVAL_OUT") {
        std::fs::write(&p, &out).unwrap();
    }

    // Gate as ruled by the user (2026-11-07): not worse than the first bundled table. The plan's
    // cross-check 27/29 is out of reach of every measured table without a false trigger (miss
    // 001157 p3: pdf_oxide decodes its font differently from PyMuPDF), so 26/29 is accepted.
    let r2 = &shipped["r2_test_half"];
    let cross = &shipped["cross63"];
    assert!(
        r2.recall.1 > 0 && r2.ftr_bd.1 > 0 && cross.recall.1 > 0,
        "gate denominators must be positive: R2 positives {}, R2 born-digital negatives {}, cross63 positives {}",
        r2.recall.1,
        r2.ftr_bd.1,
        cross.recall.1
    );
    let verdict = [
        (
            format!(
                "R2 test-half recall {}/{} >= 71/73",
                r2.recall.0, r2.recall.1
            ),
            r2.recall.0 * 73 >= 71 * r2.recall.1,
        ),
        (
            format!(
                "R2 test-half born-digital FTR {}/{} <= 1/104",
                r2.ftr_bd.0, r2.ftr_bd.1
            ),
            r2.ftr_bd.0 * 104 <= r2.ftr_bd.1,
        ),
        (
            format!(
                "cross63 recall {}/{} >= 26/29",
                cross.recall.0, cross.recall.1
            ),
            cross.recall.0 * 29 >= 26 * cross.recall.1,
        ),
    ];
    println!("\n===== gate (shipped config) =====");
    for (what, ok) in &verdict {
        println!("  {} {what}", if *ok { "PASS" } else { "FAIL" });
    }
    assert!(verdict.iter().all(|v| v.1), "need-OCR recall gate failed");
}
