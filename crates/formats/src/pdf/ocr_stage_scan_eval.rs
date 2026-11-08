use super::*;
use crate::pdf::tables::{OwnTables, scan_config, scan_page_input};
use crate::tables::detect_tables;
use crate::tables::eval::{EvalRows, load_gt};
use crate::tables::golden::{eval_cfg, eval_rows, score_scan};
use plaindoc_ocr::{OcrParams, ocr_image};
use rayon::prelude::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

#[test]
#[ignore = "needs SCAN_EVAL, TABLES_GT and the PaddleOCR models"]
fn scan_tables_real_path() {
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let (Ok(root), Ok(gt_path)) = (std::env::var("SCAN_EVAL"), std::env::var("TABLES_GT")) else {
        eprintln!("skip: set SCAN_EVAL and TABLES_GT (docs/tables-eval.md)");
        return;
    };
    let root = Path::new(&root);
    let gt = load_gt(&std::fs::read_to_string(gt_path).unwrap());
    let meta: Vec<Value> = std::fs::read_to_string(root.join("img/deg/meta.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let engine = crate::ocr::paddle_engine(None, None);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(env("SCAN_EVAL_THREADS", "4").parse().unwrap())
        .build()
        .unwrap();
    for variant in env("SCAN_EVAL_VARIANTS", "c150").split(',') {
        // `<doc>_p<page>` of the ground-truth documents; pages without an image get no prediction.
        let pages: Vec<(String, usize, &Value)> = meta
            .iter()
            .filter(|m| m["variant"] == variant)
            .filter_map(|m| {
                let (doc, p) = m["stem"].as_str()?.rsplit_once("_p")?;
                gt.get(doc)?;
                Some((doc.to_string(), p.parse().ok()?, m))
            })
            .collect();
        // As `run_page_ocr` on a `replace` page: default params plus rules at the raster dpi, no tables
        // on an `OcrEmpty` page or one without words.
        let mut ocr: Vec<(String, usize, Option<ScanTableInputs>, f64)> = pool.install(|| {
            pages
                .par_iter()
                .map(|(doc, p, m)| {
                    let dpi = m["dpi"].as_f64().unwrap() as f32;
                    let img = image::open(root.join(m["img"].as_str().unwrap()))
                        .unwrap()
                        .to_rgb8();
                    let t = Instant::now();
                    let mut params = OcrParams::default();
                    let mut l = LinesParams::default();
                    l.dpi = dpi;
                    params.table_lines = Some(l);
                    let page = ocr_image(&*engine, &img, &params).unwrap();
                    let (w, _) = ocr_warnings(&[&page], "full page", 0.75);
                    let inputs = (!w.iter().any(|w| w.kind == WarningKind::OcrEmpty)
                        && !page.words.is_empty())
                    .then(|| scan_inputs(&page, *p, dpi / 72.0));
                    if let Some(i) = &inputs {
                        OwnTables::default().detect_scan(i).unwrap();
                    }
                    let ms = t.elapsed().as_secs_f64() * 1e3;
                    (doc.clone(), *p, inputs, ms)
                })
                .collect()
        });
        ocr.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        let mut ms: Vec<f64> = ocr.iter().map(|o| o.3).collect();
        ms.sort_by(f64::total_cmp);
        for spec in env("SCAN_EVAL_CFGS", "default").split(';') {
            let cfg = eval_cfg(spec);
            let det = OwnTables {
                config: cfg.clone(),
            };
            println!(
                "real {variant} cfg {spec}: pages {} (no tables input {}), ocr+tables ms/page median {:.0}",
                ocr.len(),
                ocr.iter().filter(|o| o.2.is_none()).count(),
                ms.get(ms.len() / 2).copied().unwrap_or(0.0)
            );
            let mut preds: BTreeMap<String, Vec<(usize, EvalRows)>> = BTreeMap::new();
            for (doc, p, inputs, _) in &ocr {
                let Some(i) = inputs else { continue };
                let found = detect_tables(&scan_page_input(i), &scan_config(&cfg));
                let n = if found.skipped.is_some() {
                    0
                } else {
                    found.tables.len()
                };
                assert_eq!(det.detect_scan(i).map_or(0, |t| t.len()), n, "{doc}:{p}");
                if n > 0 && std::env::var_os("SCAN_EVAL_LIST").is_some() {
                    println!("  page {doc}:{p} tables {n}");
                }
                let e = preds.entry(doc.clone()).or_default();
                e.extend(found.tables.iter().map(|t| (*p, eval_rows(t))));
            }
            score_scan(&gt, &preds);
        }
    }
}
