//! Detector time per page on the golden fixtures (no PDF parsing): `cargo bench --features tables-eval --bench tables`.

use criterion::{Criterion, criterion_group, criterion_main};
use plaindoc_formats::tables::dump::page_from_dump;
use plaindoc_formats::tables::{PageInput, TableConfig, detect_tables};

fn pages() -> Vec<PageInput> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/tables/golden.json"
    );
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .map(|p| page_from_dump(&p["input"]))
        .collect()
}

fn bench(c: &mut Criterion) {
    let pages = pages();
    let cfg = TableConfig::default();
    c.bench_function(
        &format!("detect_tables/{}_golden_pages", pages.len()),
        |b| {
            b.iter(|| {
                pages
                    .iter()
                    .map(|p| detect_tables(p, &cfg).tables.len())
                    .sum::<usize>()
            })
        },
    );
}

criterion_group!(benches, bench);
criterion_main!(benches);
