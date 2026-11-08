mod pdf_common;
use pdf_common::*;
use plaindoc_core::*;
use plaindoc_formats::pdf::{PdfConfig, PdfParser};

#[test]
#[ignore = "needs ODL_BENCH_PDFS/ODL_BENCH_GT"]
fn odl_all_parse_ok() {
    for f in odl_files() {
        assert!(parse_path_abs(&f).is_ok(), "{}", f.display());
    }
}

#[test]
#[ignore = "needs ODL_BENCH_PDFS/ODL_BENCH_GT"]
fn odl_text_regression_floor() {
    let (_, gt) = odl_dirs();
    let mut f1: Vec<f64> = odl_files()
        .iter()
        .map(|f| {
            let md = gt.join(f.file_stem().unwrap()).with_extension("md");
            let truth = std::fs::read_to_string(&md)
                .unwrap_or_else(|e| panic!("ground truth {}: {e}", md.display()));
            word_f1(&doc_text(&parse_path_abs(f).unwrap()), &truth)
        })
        .collect();
    f1.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let (mean, median, p10) = (
        f1.iter().sum::<f64>() / f1.len() as f64,
        f1[f1.len() / 2],
        f1[f1.len() / 10],
    );
    eprintln!("odl word F1: mean {mean:.4} median {median:.4} p10 {p10:.4}");
    // measured after D2: 0.9167 / 0.9567 / 0.7861; floors = measured - 0.005
    assert!(
        mean >= 0.911 && median >= 0.951 && p10 >= 0.781,
        "mean {mean} median {median} p10 {p10}"
    );
}

/// Stems of ODL files with a Warn or Loss warning other than `TableUncertain` (pinned separately, SP4).
fn odl_flagged(parse: impl Fn(&std::path::Path) -> Result<Document>) -> Vec<String> {
    let mut flagged: Vec<String> = odl_files()
        .iter()
        .filter(|f| {
            levels(&parse(f).unwrap())
                .iter()
                .any(|(_, k, l)| *l >= Level::Warn && *k != WarningKind::TableUncertain)
        })
        .map(|f| f.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    flagged.sort();
    flagged
}

#[test]
#[ignore = "needs ODL_BENCH_PDFS/ODL_BENCH_GT"]
fn odl_warn_loss_pages_are_pinned_with_sp2_table() {
    // precision: no new false alarms on clean files
    let c = sp2_off();
    assert_eq!(
        odl_flagged(|f| parse_with(&c, f)),
        vec!["01030000000141", "01030000000198"]
    );
}

/// Default config (classifier with the bundled bigram table); same pins as without the table.
/// 01030000000198 (SP2 `ImageTextPage`/Warn: full-page image under a good layer) is `TextOk`.
#[test]
#[ignore = "needs ODL_BENCH_PDFS/ODL_BENCH_GT"]
fn odl_warn_loss_pages_are_pinned_with_need_ocr() {
    let c = need_ocr_on();
    let flagged = odl_flagged(|f| parse_with(&c, f));
    // Without a raster 01030000000107 has no ink signal; with it, ink outside the layer (Warn),
    // lowered to Info once merge OCR reads it (an engine is built in).
    #[cfg(not(feature = "ocr"))]
    let want = vec!["01030000000141"];
    #[cfg(feature = "ocr")]
    let want = if c.ocr.engine.is_some() {
        vec!["01030000000141"]
    } else {
        vec!["01030000000107", "01030000000141"]
    };
    assert_eq!(flagged, want);
}

#[test]
#[ignore = "needs ODL_BENCH_PDFS/ODL_BENCH_GT"]
fn odl_table_uncertain_pages_are_pinned() {
    let mut tables: Vec<String> = odl_files()
        .iter()
        .filter(|f| {
            levels(&parse_path_abs(f).unwrap())
                .iter()
                .any(|(_, k, l)| *l >= Level::Warn && *k == WarningKind::TableUncertain)
        })
        .map(|f| f.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    tables.sort();
    // 162, 183 are false tables (numbered list, chart); 120 is a real table with p = 0.25; 043 is a chart
    // legend of filled bars (p = 0.27) that the per-subpath fill boxes of Task 10a expose, while the prose
    // false table of 157 is gone with them; 180 is a real table (version history, same cells) whose p
    // drops from 0.52 to 0.29 through the T17 `row_newcols` row rule (cells identical)
    assert_eq!(
        tables,
        [
            "01030000000043",
            "01030000000120",
            "01030000000162",
            "01030000000180",
            "01030000000183"
        ]
    );
}

/// Bullets and dashes a PDF list item drops (SP8b S1); check boxes stay in the text.
const BULLETS: &str = "•●▪◦►▸‣∙·–—-*";

/// What the lists of `blocks` may drop: per char, plus a pool of `.`/`)` and one of bullets. An
/// ordered list of n items drops the digits of 1..n and n separators; an unordered one at most one
/// bullet per item (none for a vector bullet or a kept marker: an upper bound).
#[derive(Default)]
struct Dropped {
    digits: std::collections::HashMap<char, i64>,
    seps: i64,
    bullets: i64,
}

fn dropped(blocks: &[Block], d: &mut Dropped) {
    for b in blocks {
        if let Block::List { ordered, items, .. } = b {
            for (n, it) in items.iter().enumerate() {
                if *ordered {
                    for c in (n + 1).to_string().chars() {
                        *d.digits.entry(c).or_default() += 1;
                    }
                    d.seps += 1;
                } else {
                    d.bullets += 1;
                }
                dropped(&it.blocks, d);
            }
        }
    }
}

/// Multiset difference `(missing from a, extra in a)` of the non-whitespace chars; missing chars
/// that the lists of the document may drop (`d`) are not counted.
fn char_diff(a: &str, b: &str, mut d: Dropped) -> (usize, usize) {
    let mut m = std::collections::HashMap::<char, i64>::new();
    for c in chars_of(a) {
        *m.entry(c).or_default() += 1;
    }
    for c in chars_of(b) {
        *m.entry(c).or_default() -= 1;
    }
    let mut lost = 0;
    for (c, v) in &m {
        let mut miss = (-v).max(0);
        let mut none = 0;
        let pool = if c.is_ascii_digit() {
            d.digits.entry(*c).or_default()
        } else if ".)".contains(*c) {
            &mut d.seps
        } else if BULLETS.contains(*c) {
            &mut d.bullets
        } else {
            &mut none
        };
        let take = miss.min(*pool);
        *pool -= take;
        miss -= take;
        lost += miss;
    }
    (
        lost as usize,
        m.values().filter(|v| **v > 0).sum::<i64>() as usize,
    )
}

#[test]
#[ignore = "needs ODL_BENCH_PDFS/ODL_BENCH_GT"]
fn odl_blocks_preserve_non_artifact_characters() {
    // SP4: tables take their text out of the regions; nothing may be lost, and text both in a
    // table and in a paragraph (span centre outside the table bbox) stays rare: 13 chars, file 078.
    // OCR off: with an engine (`bundled-models`) recognised image text would count as duplicates.
    // SP8: `kept` is unfiltered, so compare without the hidden-text filter (ODL-200 has nothing to
    // remove, but the invariant must not depend on that).
    let mut opts = ParseOptions::default();
    opts.ocr = OcrMode::Never;
    opts.hidden_text = HiddenText::Off;
    let mut dup = 0;
    for f in odl_files() {
        let doc = pdf_oxide::PdfDocument::open(&f).unwrap();
        let kept: String = (0..doc.page_count().unwrap())
            .flat_map(|p| doc.extract_structured(p).unwrap().regions)
            .filter(|r| r.kind != pdf_oxide::structured::RegionRole::Artifact)
            .map(|r| r.text)
            .collect();
        // SP8 task 9: form field runs of '_' shrink to `___` (01030000000117), so runs of two or
        // more '_' are not compared; a single '_' is
        let no_runs = |t: &str| {
            let (mut out, mut chars) = (String::with_capacity(t.len()), t.chars().peekable());
            while let Some(c) = chars.next() {
                let mut run = 0;
                while c == '_' && chars.next_if_eq(&'_').is_some() {
                    run += 1;
                }
                if run == 0 {
                    out.push(c);
                }
            }
            out
        };
        let parsed = PdfParser::new()
            .parse(Input::Path(f.clone()), Format::Pdf, &opts)
            .unwrap();
        // SP8b S1: list items drop their markers, nothing else
        let mut d = Dropped::default();
        for sec in &parsed.sections {
            dropped(&sec.blocks, &mut d);
        }
        let (lost, extra) = char_diff(&no_runs(&doc_text(&parsed)), &no_runs(&kept), d);
        assert_eq!(lost, 0, "{}", f.display());
        dup += extra;
    }
    assert!(dup <= 20, "{dup} duplicated chars");
}

#[test]
#[ignore = "needs ODL_BENCH_PDFS/ODL_BENCH_GT"]
fn odl_parallel_equals_sequential() {
    let mut c = PdfConfig::default();
    c.min_parallel_pages = 1;
    let parser = PdfParser::with_config(c);
    let (mut seq, mut par) = (ParseOptions::default(), ParseOptions::default());
    seq.page_parallel = false;
    par.page_parallel = true;
    for f in odl_files() {
        let a = parser
            .parse(Input::Path(f.clone()), Format::Pdf, &seq)
            .unwrap();
        let b = parser
            .parse(Input::Path(f.clone()), Format::Pdf, &par)
            .unwrap();
        assert_eq!(a.to_json(), b.to_json(), "{}", f.display());
    }
}

/// SP2 stage 1 (classifier off); the default config is timed by `odl_time_per_page_with_need_ocr`.
#[test]
#[ignore = "needs ODL_BENCH_PDFS/ODL_BENCH_GT; run with --release"]
fn odl_time_per_page_budget() {
    // SP4 ruling В-1: tables plus char refinement cost +25–30 % over SP2 (A/B in one process: 3.42 →
    // 4.39 ms/page; after the SP5 merge, pdf_oxide 3725d99, classifier off: 2.74 → 3.59 `pdf`,
    // 2.62 → 3.42 `ocr`, 2.70 → 3.44 all features, x1.27–1.31). The gate is relative to the SP2 configuration measured in the same process (machine
    // load cancels out), plus an absolute ceiling with headroom.
    let files = odl_files();
    let sp2 = {
        let mut c = sp2_off();
        c.tables = std::sync::Arc::new(plaindoc_formats::pdf::NoTables);
        c.char_x_refinement = false;
        PdfParser::with_config(c)
    };
    // Classifier off in both (its cost is gated by `odl_time_per_page_with_need_ocr`).
    let sp4 = PdfParser::with_config(sp2_off());
    let opts = ParseOptions::default();
    let time = |p: &PdfParser| {
        let t = std::time::Instant::now();
        let pages: usize = files
            .iter()
            .map(|f| {
                p.parse(Input::Path(f.clone()), Format::Pdf, &opts)
                    .unwrap()
                    .sections
                    .len()
            })
            .sum();
        t.elapsed().as_secs_f64() * 1000.0 / pages as f64
    };
    let (mut base, mut own) = (0.0, 0.0);
    for _ in 0..2 {
        base += time(&sp2) / 2.0;
        own += time(&sp4) / 2.0;
    }
    eprintln!(
        "odl: {own:.2} ms/page with tables, {base:.2} without (SP2), x{:.2}",
        own / base
    );
    assert!(own <= 1.5 * base, "{own:.2} vs {base:.2} ms/page");
    assert!(own <= 8.0, "{own:.2} ms/page");
}

#[test]
#[ignore = "needs ODL_BENCH_PDFS/ODL_BENCH_GT; run with --release"]
fn odl_time_per_page_with_need_ocr() {
    let (c, files) = (need_ocr_on(), odl_files());
    let t = std::time::Instant::now();
    let pages: usize = files
        .iter()
        .map(|f| parse_with(&c, f).unwrap().sections.len())
        .sum();
    let ms = t.elapsed().as_secs_f64() * 1000.0 / pages as f64;
    eprintln!("odl need_ocr on: {pages} pages, {ms:.2} ms/page sequential");
    // measured 11.9 ms/page with `ocr` (gated 50-dpi renders), 3.2 without (task 15); with the
    // bundled bigram table 11.9 / 3.1; after the SP4 merge (tables, char refinement) 11.5 / 3.75
    let budget = if cfg!(feature = "ocr") { 20.0 } else { 6.0 };
    assert!(ms <= budget, "{ms:.2} ms/page");
}

#[test]
fn parse_many_equals_single_parse() {
    let bytes: Vec<Vec<u8>> = (0..4).map(|_| builder_pdf_bytes(40)).collect();
    let r = plaindoc_formats::Registry::with_defaults();
    let mut o = ParseOptions::default();
    o.timeout = Some(std::time::Duration::from_secs(30));
    o.page_parallel = true; // parse: allowed (40 >= 32); parse_many forces false (D1)
    let many = r.parse_many(bytes.iter().cloned().map(Input::Bytes).collect(), &o);
    let single: Vec<_> = bytes
        .into_iter()
        .map(|b| r.parse(Input::Bytes(b), &o))
        .collect();
    for (m, s) in many.iter().zip(&single) {
        assert_eq!(m.as_ref().unwrap().to_json(), s.as_ref().unwrap().to_json());
    }
}
