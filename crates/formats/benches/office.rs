//! Criterion benchmark for the office parser (fixture corpus).

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use plaindoc_core::{Input, ParseOptions};
use plaindoc_formats::Registry;
use std::hint::black_box;
use std::path::{Path, PathBuf};

/// Files of `generated/` and `poi/` except `NOTICE` and `encrypted.docx`, sorted.
fn fixtures() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/office");
    let mut out = Vec::new();
    for dir in ["generated", "poi"] {
        for e in std::fs::read_dir(root.join(dir)).expect("fixture dir") {
            let path = e.unwrap().path();
            let name = path.file_name().unwrap();
            if path.is_file() && name != "NOTICE" && name != "encrypted.docx" {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn bench_files(c: &mut Criterion) {
    let registry = Registry::with_defaults();
    let opts = ParseOptions::default();
    let mut group = c.benchmark_group("office/parse");
    group.throughput(Throughput::Elements(1));
    for path in fixtures() {
        let bytes = std::fs::read(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        registry
            .parse(Input::Bytes(bytes.clone()), &opts)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        group.bench_function(name, |b| {
            b.iter_batched(
                || Input::Bytes(bytes.clone()),
                |input| black_box(registry.parse(input, &opts)),
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

fn bench_corpus(c: &mut Criterion) {
    let registry = Registry::with_defaults();
    let opts = ParseOptions::default();
    let files: Vec<Vec<u8>> = fixtures()
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();
    c.bench_function("office/corpus_rayon", |b| {
        b.iter_batched(
            || files.iter().cloned().map(Input::Bytes).collect::<Vec<_>>(),
            |inputs| black_box(registry.parse_many(inputs, &opts)),
            BatchSize::LargeInput,
        )
    });
}

criterion_group!(benches, bench_files, bench_corpus);
criterion_main!(benches);
