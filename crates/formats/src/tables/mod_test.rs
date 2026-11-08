use super::testkit::{hline, input_word, vline};
use super::*;

fn page(w: f64, h: f64) -> PageInput {
    PageInput {
        width: w,
        height: h,
        ..Default::default()
    }
}

/// T13: a page of `MAX_WORDS` words in one text block must stay linearithmic (was ~seconds).
#[test]
fn detector_is_fast_on_a_20k_word_text_block() {
    let mut p = page(612.0, 30_000.0);
    // 10 columns x 2000 rows on a tall page. Every x edge is distinct (~40 000 elementary intervals).
    // The grid is then skipped as over MAX_CELLS, after the valleys.
    p.words = (0..MAX_WORDS)
        .map(|i| {
            let x = 20.0 + 50.0 * (i % 10) as f64 + (i as f64 * 0.0137) % 5.0;
            input_word("ab", x, 4.0 + 12.0 * (i / 10) as f64, 10.0)
        })
        .collect();
    let t = std::time::Instant::now();
    let r = detect_tables(&p, &TableConfig::default());
    let ms = t.elapsed().as_secs_f64() * 1e3;
    eprintln!(
        "detect_tables 20000 words: {ms:.2} ms, skipped {:?}",
        r.skipped.is_some()
    );
    assert!(r.skipped.is_some(), "expected the MAX_CELLS skip");
    // Measured 2-3 ms (gate 5 ms); the bound leaves room for a loaded CI box, the quadratic version took ~575 ms.
    if !cfg!(debug_assertions) {
        assert!(ms <= 50.0, "detector took {ms} ms");
    }
}

/// The region model's rules are off unless asked for (T18 gate, SP4-R3 bounds).
#[test]
fn region_model_config_default_is_all_off() {
    let d = RegionModelConfig::default();
    assert_eq!((d.page_gate, d.inner_bounds), (false, false));
}

/// SP4-R1: over [`MAX_MODEL_REGIONS`] the page is skipped before any work; at the limit, regions each
/// covering a page of `MAX_WORDS` table words (10 columns of 3-word cells, under `MAX_CELLS`) stay
/// bounded.
#[test]
fn model_regions_are_bounded() {
    let mut p = page(612.0, 8100.0);
    p.words = (0..MAX_WORDS)
        .map(|i| {
            let (cell, k) = (i / 3, i % 3);
            let x = 20.0 + 58.0 * (cell % 10) as f64 + 14.0 * k as f64;
            input_word("ab", x, 4.0 + 12.0 * (cell / 10) as f64, 10.0)
        })
        .collect();
    let region = ModelRegion::new(Rect::new(0.0, 0.0, 612.0, 8100.0), 1.0);
    let cfg = TableConfig {
        region_model: Some(RegionModelConfig::default()),
        ..TableConfig::default()
    };
    p.model_regions = vec![region; MAX_MODEL_REGIONS + 1];
    let r = detect_tables(&p, &cfg);
    assert_eq!(r.skipped.map(|l| l.what), Some("model regions on the page"));
    assert!(
        detect_tables(&p, &TableConfig::default()).skipped.is_none(),
        "regions are ignored with the switch off"
    );
    p.model_regions.truncate(MAX_MODEL_REGIONS);
    let t = std::time::Instant::now();
    let r = detect_tables(&p, &cfg);
    let ms = t.elapsed().as_secs_f64() * 1e3;
    eprintln!(
        "{MAX_MODEL_REGIONS} page-wide model regions, {MAX_WORDS} words: {ms:.2} ms, tables {}, skipped {:?}",
        r.tables.len(),
        r.skipped
    );
    if !cfg!(debug_assertions) {
        // Measured ~40 ms (one T pass over the words); the bound leaves 5x for a loaded box.
        assert!(ms <= 200.0, "detector took {ms} ms");
    }
}

/// Hostile pages under the input limits (T13b); `measure_hostile` prints the time per config.
fn hostile_pages() -> Vec<(&'static str, PageInput)> {
    let mut out = Vec::new();
    // R1: 4000 small 2x2 grids (6 rulings, 4 words each) on a wide, tall page.
    let mut p = page(5000.0, 6000.0);
    for k in 0..4000usize {
        let (x, y) = (
            10.0 + 120.0 * (k % 40) as f64,
            10.0 + 60.0 * (k / 40) as f64,
        );
        for dy in [0.0, 20.0, 40.0] {
            p.paths.push(hline(x, x + 100.0, y + dy));
        }
        for dx in [0.0, 50.0, 100.0] {
            p.paths.push(vline(x + dx, y, y + 40.0));
        }
        for (dx, dy) in [(5.0, 5.0), (55.0, 5.0), (5.0, 25.0), (55.0, 25.0)] {
            p.words.push(input_word("ab", x + dx, y + dy, 10.0));
        }
    }
    out.push(("R1 4000 small grids", p));
    // R2: 6000 regions of 3 rulings each, 3 words each (words in regions, 18 000 words).
    let mut p = page(5000.0, 6000.0);
    for k in 0..6000usize {
        let (x, y) = (
            10.0 + 120.0 * (k % 40) as f64,
            10.0 + 40.0 * (k / 40) as f64,
        );
        p.paths.push(hline(x, x + 100.0, y));
        p.paths.push(hline(x, x + 100.0, y + 25.0));
        p.paths.push(vline(x, y, y + 25.0));
        for dx in [5.0, 40.0, 70.0] {
            p.words.push(input_word("ab", x + dx, y + 8.0, 10.0));
        }
    }
    out.push(("R2 6000 regions x 3 words", p));
    // R3: vertical stack of 12000 full-width rulings (prims::regions sweep).
    let mut p = page(612.0, 60_000.0);
    for k in 0..12_000usize {
        p.paths.push(hline(20.0, 520.0, 5.0 + 5.0 * k as f64));
    }
    out.push(("R3 12000 stacked full-width hrules", p));
    // R3b: 24000 of them with 20000 words.
    let mut p = page(612.0, 120_000.0);
    for k in 0..24_000usize {
        p.paths.push(hline(20.0, 520.0, 5.0 + 5.0 * k as f64));
    }
    for k in 0..20_000usize {
        p.words.push(input_word(
            "ab",
            30.0 + 100.0 * (k % 4) as f64,
            7.0 + 6.0 * k as f64,
            10.0,
        ));
    }
    out.push(("R3b 24000 hrules + 20000 words", p));
    // R4: 2000 thick filled bands (fill stacks) in 600 stacks of ~3, 20000 words inside.
    let mut p = page(612.0, 12_000.0);
    for k in 0..1998usize {
        let (st, b) = (k / 3, k % 3);
        let y = 10.0 + 18.0 * st as f64 * 3.0 + 18.0 * b as f64;
        p.paths.push(InputPath {
            rect: Rect::new(20.0, y, 500.0, 17.0),
            stroke: false,
            fill: true,
        });
    }
    for k in 0..20_000usize {
        p.words.push(input_word(
            "ab",
            30.0 + 100.0 * (k % 4) as f64,
            12.0 + 0.5 * (k / 4) as f64 % 10_000.0,
            8.0,
        ));
    }
    out.push(("R4 600 fill stacks + 20000 words", p));
    // R5: 25000 vertical rulings side by side with 20000 words (one region).
    let mut p = page(30_000.0, 800.0);
    for k in 0..12_000usize {
        p.paths.push(vline(5.0 + 2.0 * k as f64, 10.0, 700.0));
    }
    for k in 0..12_000usize {
        p.paths
            .push(hline(5.0, 24_000.0, 10.0 + 50.0 * (k % 14) as f64));
    }
    for k in 0..20_000usize {
        p.words.push(input_word(
            "ab",
            6.0 + 1.1 * k as f64,
            12.0 + 40.0 * (k % 15) as f64,
            10.0,
        ));
    }
    out.push(("R5 grid 12000 v + 12000 h, 20000 words", p));
    out
}

/// T13b: per-region scans stay near-linear. Measured 0.6-5 ms (before: 69-276 ms); the 50 ms bound is
/// loose for a loaded CI box and only checked in release.
#[test]
fn per_region_scans_stay_fast_on_hostile_pages() {
    for (name, p) in hostile_pages() {
        if !["R2", "R3"].iter().any(|k| name.starts_with(k)) {
            continue;
        }
        let t = std::time::Instant::now();
        let r = detect_tables(&p, &TableConfig::default());
        let ms = t.elapsed().as_secs_f64() * 1e3;
        assert!(r.skipped.is_none(), "{name}");
        if !cfg!(debug_assertions) {
            assert!(ms <= 50.0, "{name}: {ms} ms");
        }
    }
}

/// T15: `MAX_WORDS` words of two-column prose (5 words per column and line, distinct x edges) on a
/// tall page: every line is a candidate, so one text block crosses the page gutter and the guard
/// runs on it, after one page-wide `valleys` + `text_lines`.
pub(crate) fn two_column_prose() -> PageInput {
    let mut p = page(612.0, 30_000.0);
    p.words = (0..MAX_WORDS)
        .map(|i| {
            let (line, k) = (i / 10, i % 10);
            let jit = (i as f64 * 0.0137) % 3.0;
            let x = if k < 5 { 60.0 } else { 330.0 } + 34.0 * (k % 5) as f64 + jit;
            input_word("lorem", x, 4.0 + 12.0 * line as f64, 10.0)
        })
        .collect();
    p
}

#[test]
fn gutter_guard_is_fast_on_20k_words_of_two_column_prose() {
    let p = two_column_prose();
    let time = |cfg: &TableConfig| {
        let t = std::time::Instant::now();
        let r = detect_tables(&p, cfg);
        (t.elapsed().as_secs_f64() * 1e3, r)
    };
    let (base, _) = time(&TableConfig {
        text_longseg: false,
        text_dense: false,
        gutter_prose: 0,
        ..TableConfig::default()
    });
    let (ms, r) = time(&TableConfig::default());
    eprintln!(
        "two-column prose 20000 words: without T15 {base:.2} ms, default {ms:.2} ms, tables {}, rejected {:?}",
        r.tables.len(),
        r.rejected.iter().map(|x| x.reason).collect::<Vec<_>>()
    );
    assert!(r.skipped.is_none());
    assert!(r.tables.is_empty(), "prose is not a table");
    // Measured ~51 ms (~48 ms without T15; the guard itself ~3 ms: `gutter_scan_is_fast_on_20k_words`).
    if !cfg!(debug_assertions) {
        assert!(ms <= 1000.0, "detector took {ms} ms");
    }
}

#[test]
#[ignore = "prints timings of the hostile pages (release)"]
fn measure_hostile() {
    for (name, p) in hostile_pages() {
        for (cn, cfg) in [
            ("default", TableConfig::default()),
            ("reference", TableConfig::reference()),
            (
                "split",
                TableConfig {
                    split_glued_words: true,
                    ..TableConfig::default()
                },
            ),
        ] {
            let t = std::time::Instant::now();
            let r = detect_tables(&p, &cfg);
            eprintln!(
                "{name:42} {cn:10} {:9.2} ms  tables {} skipped {:?}",
                t.elapsed().as_secs_f64() * 1e3,
                r.tables.len(),
                r.skipped
            );
        }
    }
}

fn skipped(p: &PageInput) -> LimitExceeded {
    let r = detect_tables(p, &TableConfig::default());
    assert!(r.tables.is_empty() && r.rejected.is_empty());
    r.skipped.expect("page is skipped")
}

#[test]
fn too_many_words_skip_the_page() {
    // 400 × 400 ruled grid with a word per cell: 160 000 words (20 s per page without the limit).
    let mut p = page(5000.0, 5000.0);
    for k in 0..160_000 {
        let (i, j) = ((k % 400) as f64, (k / 400) as f64);
        p.words
            .push(input_word("12", 50.0 + 12.0 * i, 50.0 + 12.0 * j, 10.0));
    }
    for k in 0..=400 {
        let v = 50.0 + 12.0 * k as f64;
        p.paths.push(hline(50.0, 4850.0, v));
        p.paths.push(vline(v, 50.0, 4850.0));
    }
    let l = skipped(&p);
    assert_eq!(
        (l.what, l.count, l.limit),
        ("words on the page", 160_000, MAX_WORDS)
    );
}

#[test]
fn too_many_rulings_skip_the_page() {
    // 32 000 full-width rulings: the region sweep is quadratic (1 s without the limit).
    let mut p = page(600.0, 800.0);
    p.paths
        .extend((0..32_000).map(|k| hline(10.0, 590.0, 10.0 + k as f64 * 780.0 / 32_000.0)));
    p.words.push(input_word("x", 20.0, 20.0, 10.0));
    assert_eq!(skipped(&p).count, 32_000);
}

#[test]
fn far_off_primitives_are_capped_before_clipping() {
    // Rulings far below the page: under the raw cap clipping drops them, over it nothing is copied.
    let far = |n: usize| {
        let mut p = page(600.0, 800.0);
        p.paths
            .extend((0..n).map(|k| hline(10.0, 590.0, 5_000.0 + k as f64 * 0.01)));
        p.words.push(input_word("x", 20.0, 20.0, 10.0));
        p
    };
    assert!(
        detect_tables(&far(MAX_PRIMS + 1), &TableConfig::default())
            .skipped
            .is_none()
    );
    let l = skipped(&far(ADAPTER_MAX_PRIMS + 1));
    assert_eq!(
        (l.what, l.count, l.limit),
        (
            "paths and segments before clipping",
            ADAPTER_MAX_PRIMS + 1,
            ADAPTER_MAX_PRIMS
        )
    );
}

#[test]
fn huge_mesh_is_skipped_before_allocating_cells() {
    // 4000 × 4000 mesh far beyond a Letter page, 6 words: 426 MB of cells without the limit. Clipping
    // keeps ~530 × 410 rulings near the page, still over MAX_CELLS.
    let mut p = page(612.0, 792.0);
    for k in 0..=4000 {
        let v = 3.0 * k as f64;
        p.paths.push(hline(0.0, 12_000.0, v));
        p.paths.push(vline(v, 0.0, 12_000.0));
    }
    for k in 0..6 {
        p.words
            .push(input_word("77", 100.0 + 40.0 * k as f64, 100.0, 2.0));
    }
    let l = skipped(&p);
    assert_eq!((l.what, l.limit), ("cells of a grid", MAX_CELLS));
}

#[test]
fn stroked_boxes_are_bounded_by_the_primitives_derived_from_them() {
    // 6 000 stroked boxes pass the input count, but are 30 000 primitives for the region sweep.
    let mut p = page(600.0, 800.0);
    p.paths.extend((0..6_000).map(|k| InputPath {
        rect: Rect::new(10.0, 10.0 + k as f64 * 0.03, 550.0, 5.0),
        stroke: true,
        fill: false,
    }));
    p.words.push(input_word("x", 20.0, 12.0, 8.0));
    assert!(p.paths.len() < MAX_PRIMS);
    let l = skipped(&p);
    assert_eq!(
        (l.what, l.count, l.limit),
        ("primitives derived from the page", 30_000, MAX_PRIMS)
    );
}

#[test]
fn content_far_outside_the_page_is_clipped() {
    let mut p = testkit::grid_page(
        &[50.0, 150.0, 250.0, 350.0],
        &[100.0, 120.0, 140.0, 160.0],
        &[],
    );
    for (r, y) in [102.0, 122.0, 142.0].into_iter().enumerate() {
        for (c, x) in [55.0, 155.0, 255.0].into_iter().enumerate() {
            p.words.push(input_word(&format!("{r}{c}"), x, y, 10.0));
        }
    }
    let inside = detect_tables(&p, &TableConfig::default());
    assert_eq!(inside.tables.len(), 1);
    // 6 000 segments two pages below the page change nothing and cost nothing.
    p.segments.extend((0..6_000).map(|k| InputSegment {
        x1: 0.0,
        y1: 2400.0 + k as f64,
        x2: 500.0,
        y2: 2400.0 + k as f64,
        raster: false,
    }));
    assert!(clip_to_page(&p, true).is_some_and(|c| c.segments.is_empty()));
    assert_eq!(detect_tables(&p, &TableConfig::default()), inside);
}
