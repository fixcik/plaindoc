//! Table metrics of the spike (`docs/design/spikes/tables-corpus/scripts/metric2.py`): matching by
//! page (cell-text F1 > 0.3, greedy in ground-truth order), simplified TEDS (3-level DP over
//! table→row→cell, NOT Zhang-Shasha), exact shape, false tables on pages/documents without tables.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::AddAssign;

use serde_json::Value;

/// TEDS cell-pair cap of metric2.py (ODL, ICDAR 2013, PubTables-1M); beyond it TEDS counts as 0.
pub const TEDS_CAP_SPIKE: usize = 400_000;
/// Cap of metric_synth.py (synthetic corpus).
pub const TEDS_CAP_SYNTH: usize = 4_000_000;
/// A prediction matches a ground-truth table when the cell-text F1 exceeds this.
const MATCH_F1: f64 = 0.3;

/// `(colspan, rowspan, text)` like metric2 `rows_of`.
#[derive(Clone, Debug, PartialEq)]
pub struct EvalCell {
    pub colspan: u32,
    pub rowspan: u32,
    pub text: String,
}

pub type EvalRows = Vec<Vec<EvalCell>>;

#[derive(Clone, Debug, Default)]
pub struct GtDoc {
    pub pages: usize,
    /// Scored tables `(page, rows)`.
    pub tables: Vec<(usize, EvalRows)>,
    /// Pages holding any ground-truth table, scored or excluded (`excl_det`).
    pub table_pages: BTreeSet<usize>,
}

fn rows_from(v: &Value) -> EvalRows {
    v.as_array()
        .map(|rows| {
            rows.iter()
                .map(|r| {
                    r.as_array()
                        .unwrap_or(&Vec::new())
                        .iter()
                        .map(|c| match c {
                            Value::Array(a) => EvalCell {
                                colspan: a[0].as_u64().unwrap_or(1) as u32,
                                rowspan: a[1].as_u64().unwrap_or(1) as u32,
                                text: a[2].as_str().unwrap_or("").to_string(),
                            },
                            _ => EvalCell {
                                colspan: c["cs"].as_u64().unwrap_or(1) as u32,
                                rowspan: c["rs"].as_u64().unwrap_or(1) as u32,
                                text: c["t"].as_str().unwrap_or("").to_string(),
                            },
                        })
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Ground truth in the unified format of `gt_convert.py`:
/// `{doc: {pages, tables: [{page, rows: [[[cs, rs, text]]], excl_det?}]}}`.
pub fn load_gt(json: &str) -> BTreeMap<String, GtDoc> {
    let v: Value = serde_json::from_str(json).expect("ground truth JSON");
    let mut out = BTreeMap::new();
    for (name, d) in v.as_object().expect("object") {
        let mut g = GtDoc {
            pages: d["pages"].as_u64().unwrap_or(1) as usize,
            ..Default::default()
        };
        for t in d["tables"].as_array().into_iter().flatten() {
            let page = t["page"].as_u64().unwrap_or(0) as usize;
            g.table_pages.insert(page);
            if !t["excl_det"].as_bool().unwrap_or(false) {
                g.tables.push((page, rows_from(&t["rows"])));
            }
        }
        out.insert(name.clone(), g);
    }
    out
}

/// filters.py split: `test` = odd positions of the sorted document list, `train` = even, `all` = every document.
pub fn split_docs(gt: BTreeMap<String, GtDoc>, part: &str) -> BTreeMap<String, GtDoc> {
    gt.into_iter()
        .enumerate()
        .filter(|(i, _)| match part {
            "test" => i % 2 == 1,
            "train" => i % 2 == 0,
            _ => true,
        })
        .map(|(_, d)| d)
        .collect()
}

fn norm(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn lev(a: &[char], b: &[char]) -> usize {
    if a == b {
        return 0;
    }
    let (a, b) = if a.len() < b.len() { (b, a) } else { (a, b) };
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push(
                (prev[j + 1] + 1)
                    .min(cur[j] + 1)
                    .min(prev[j] + usize::from(ca != cb)),
            );
        }
        prev = cur;
    }
    prev[b.len()]
}

fn ccost(a: &EvalCell, b: &EvalCell) -> f64 {
    if a.colspan != b.colspan || a.rowspan != b.rowspan {
        return 1.0;
    }
    let (x, y) = (norm(&a.text), norm(&b.text));
    if x == y {
        return 0.0;
    }
    let (x, y): (Vec<char>, Vec<char>) = (x.chars().collect(), y.chars().collect());
    let m = x.len().max(y.len());
    (lev(&x[..x.len().min(2000)], &y[..y.len().min(2000)]) as f64 / m as f64).min(1.0)
}

fn rowdist(r1: &[EvalCell], r2: &[EvalCell]) -> f64 {
    let mut prev: Vec<f64> = (0..=r2.len()).map(|j| j as f64).collect();
    for i in 1..=r1.len() {
        let mut cur = vec![i as f64];
        for j in 1..=r2.len() {
            cur.push(
                (prev[j] + 1.0)
                    .min(cur[j - 1] + 1.0)
                    .min(prev[j - 1] + ccost(&r1[i - 1], &r2[j - 1])),
            );
        }
        prev = cur;
    }
    prev[r2.len()]
}

/// Simplified TEDS; `None` when the cell-pair product exceeds `cap`.
pub fn teds(a: &EvalRows, b: &EvalRows, cap: usize) -> Option<f64> {
    let (ca, cb) = (
        a.iter().map(Vec::len).sum::<usize>(),
        b.iter().map(Vec::len).sum::<usize>(),
    );
    if ca * cb > cap {
        return None;
    }
    let (na, nb) = ((1 + a.len() + ca) as f64, (1 + b.len() + cb) as f64);
    let mut prev = vec![0.0];
    for r in b {
        prev.push(prev[prev.len() - 1] + 1.0 + r.len() as f64);
    }
    for ra in a {
        let mut cur = vec![prev[0] + 1.0 + ra.len() as f64];
        for (j, rb) in b.iter().enumerate() {
            cur.push(
                (prev[j + 1] + 1.0 + ra.len() as f64)
                    .min(cur[j] + 1.0 + rb.len() as f64)
                    .min(prev[j] + rowdist(ra, rb)),
            );
        }
        prev = cur;
    }
    Some(1.0 - prev[b.len()] / na.max(nb))
}

/// `(rows, max sum of colspans)`.
pub fn shape(t: &EvalRows) -> (usize, u32) {
    (
        t.len(),
        t.iter()
            .map(|r| r.iter().map(|c| c.colspan).sum())
            .max()
            .unwrap_or(0),
    )
}

fn cells(t: &EvalRows) -> Vec<String> {
    t.iter()
        .flatten()
        .map(|c| norm(&c.text))
        .filter(|s| !s.is_empty())
        .collect()
}

fn f1(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut ca: HashMap<&str, usize> = HashMap::new();
    for x in a {
        *ca.entry(x).or_default() += 1;
    }
    let mut inter = 0;
    for x in b {
        if let Some(n) = ca.get_mut(x.as_str())
            && *n > 0
        {
            *n -= 1;
            inter += 1;
        }
    }
    let (p, r) = (inter as f64 / b.len() as f64, inter as f64 / a.len() as f64);
    if p + r > 0.0 {
        2.0 * p * r / (p + r)
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Summary {
    /// Scored ground-truth tables.
    pub n: usize,
    pub found: usize,
    pub shape: usize,
    pub teds_sum: f64,
    pub fp_tables: usize,
    pub neg_pages: usize,
    pub neg_pages_fp: usize,
    pub neg_docs: usize,
    pub neg_docs_fp: usize,
    /// Unmatched predictions on pages that have tables (not counted as false tables).
    pub excess: usize,
}

impl Summary {
    pub fn found_rate(&self) -> f64 {
        self.found as f64 / self.n.max(1) as f64
    }
    pub fn teds_all(&self) -> f64 {
        self.teds_sum / self.n.max(1) as f64
    }
    pub fn shape_rate(&self) -> f64 {
        self.shape as f64 / self.n.max(1) as f64
    }
    pub fn fp_page_rate(&self) -> f64 {
        self.neg_pages_fp as f64 / self.neg_pages.max(1) as f64
    }
    pub fn fp_doc_rate(&self) -> f64 {
        self.neg_docs_fp as f64 / self.neg_docs.max(1) as f64
    }
}

impl AddAssign for Summary {
    fn add_assign(&mut self, o: Self) {
        self.n += o.n;
        self.found += o.found;
        self.shape += o.shape;
        self.teds_sum += o.teds_sum;
        self.fp_tables += o.fp_tables;
        self.neg_pages += o.neg_pages;
        self.neg_pages_fp += o.neg_pages_fp;
        self.neg_docs += o.neg_docs;
        self.neg_docs_fp += o.neg_docs_fp;
        self.excess += o.excess;
    }
}

/// metric2.py `evaluate` for one document; `preds` are `(page, rows)` in detector order.
pub fn evaluate_doc(gt: &GtDoc, preds: &[(usize, EvalRows)], teds_cap: usize) -> Summary {
    let mut s = Summary {
        n: gt.tables.len(),
        ..Default::default()
    };
    let mut used = vec![false; preds.len()];
    let pc: Vec<Vec<String>> = preds.iter().map(|p| cells(&p.1)).collect();
    for (page, t) in &gt.tables {
        let tc = cells(t);
        let mut best: Option<(f64, usize)> = None;
        for (i, (pp, _)) in preds.iter().enumerate() {
            if used[i] || pp != page {
                continue;
            }
            let v = f1(&tc, &pc[i]);
            if best.is_none_or(|b| v > b.0) {
                best = Some((v, i));
            }
        }
        if let Some((v, i)) = best
            && v > MATCH_F1
        {
            used[i] = true;
            s.found += 1;
            s.shape += usize::from(shape(t) == shape(&preds[i].1));
            s.teds_sum += teds(t, &preds[i].1, teds_cap).unwrap_or(0.0);
        }
    }
    let mut fp_pages = BTreeSet::new();
    for (i, (pp, _)) in preds.iter().enumerate() {
        if used[i] {
            continue;
        }
        if gt.table_pages.contains(pp) {
            s.excess += 1;
        } else {
            s.fp_tables += 1;
            fp_pages.insert(*pp);
        }
    }
    s.neg_pages = gt.pages.saturating_sub(gt.table_pages.len());
    s.neg_pages_fp = fp_pages.len();
    if gt.table_pages.is_empty() {
        s.neg_docs = 1;
        s.neg_docs_fp = usize::from(!preds.is_empty());
    }
    s
}

/// 95% intervals of the rates (metric2.py `bootstrap`: documents resampled with replacement).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ci {
    pub found: (f64, f64),
    pub teds_all: (f64, f64),
    pub fp_page: (f64, f64),
    pub fp_doc: (f64, f64),
}

/// Bootstrap over per-document summaries; `b` resamples, fixed `seed` (xorshift64, not Python's RNG).
pub fn bootstrap(per_doc: &[Summary], b: usize, seed: u64) -> Ci {
    let mut x = seed.max(1);
    let mut next = |n: usize| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % n as u64) as usize
    };
    let mut v: [Vec<f64>; 4] = Default::default();
    for _ in 0..b {
        let mut s = Summary::default();
        for _ in 0..per_doc.len() {
            s += per_doc[next(per_doc.len())];
        }
        for (k, r) in [
            s.found_rate(),
            s.teds_all(),
            s.fp_page_rate(),
            s.fp_doc_rate(),
        ]
        .into_iter()
        .enumerate()
        {
            v[k].push(r);
        }
    }
    let ci = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        (
            v[(0.025 * b as f64) as usize],
            v[((0.975 * b as f64) as usize).min(b - 1)],
        )
    };
    let [mut a, mut t, mut p, mut d] = v;
    Ci {
        found: ci(&mut a),
        teds_all: ci(&mut t),
        fp_page: ci(&mut p),
        fp_doc: ci(&mut d),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(rows: &[&[&str]]) -> EvalRows {
        rows.iter()
            .map(|r| {
                r.iter()
                    .map(|c| EvalCell {
                        colspan: 1,
                        rowspan: 1,
                        text: c.to_string(),
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn bootstrap_brackets_the_point_estimate() {
        let docs: Vec<Summary> = (0..40)
            .map(|i| Summary {
                n: 1,
                found: usize::from(i % 4 != 0),
                teds_sum: 0.5,
                neg_pages: 2,
                neg_pages_fp: usize::from(i % 10 == 0),
                ..Default::default()
            })
            .collect();
        let ci = bootstrap(&docs, 500, 7);
        assert!(ci.found.0 < 0.75 && 0.75 < ci.found.1, "{ci:?}");
        assert_eq!(ci.teds_all, (0.5, 0.5));
        assert!(ci.fp_page.0 <= 0.05 && 0.05 <= ci.fp_page.1);
        assert_eq!(bootstrap(&docs, 500, 7), ci, "deterministic");
    }

    #[test]
    fn teds_is_one_for_equal_and_drops_with_errors() {
        let a = t(&[&["A", "B"], &["1", "2"]]);
        assert_eq!(teds(&a, &a, TEDS_CAP_SPIKE), Some(1.0));
        // metric2.teds([[[1,1,'A'],[1,1,'B']],[[1,1,'1'],[1,1,'2']]], [[[1,1,'A'],[1,1,'B']],[[1,1,'1'],[1,1,'X']]]) = 0.857142...
        let b = t(&[&["A", "B"], &["1", "X"]]);
        assert!((teds(&a, &b, TEDS_CAP_SPIKE).unwrap() - 6.0 / 7.0).abs() < 1e-12);
        assert_eq!(teds(&a, &b, 15), None, "4 × 4 cell pairs exceed the cap");
    }

    #[test]
    fn matching_counts_found_shape_fp_and_excess() {
        let gt = GtDoc {
            pages: 3,
            tables: vec![(0, t(&[&["a", "b"], &["c", "d"]]))],
            table_pages: [0].into(),
        };
        let preds = vec![
            (0, t(&[&["a", "b"], &["c", "d"]])),
            (0, t(&[&["x"]])),
            (2, t(&[&["y"]])),
        ];
        let s = evaluate_doc(&gt, &preds, TEDS_CAP_SPIKE);
        assert_eq!(
            (
                s.n,
                s.found,
                s.shape,
                s.excess,
                s.fp_tables,
                s.neg_pages,
                s.neg_pages_fp
            ),
            (1, 1, 1, 1, 1, 2, 1)
        );
        assert_eq!(s.teds_all(), 1.0);
        let empty = GtDoc {
            pages: 1,
            ..Default::default()
        };
        let s = evaluate_doc(&empty, &preds[1..2], TEDS_CAP_SPIKE);
        assert_eq!((s.neg_docs, s.neg_docs_fp, s.fp_doc_rate()), (1, 1, 1.0));
    }
}
