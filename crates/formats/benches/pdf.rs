//! Pages-per-second benchmark of the PDF parser.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use pdf_oxide::PdfDocument;
use pdf_oxide::writer::DocumentBuilder;
use plaindoc_core::{Format, Input, ParseOptions, Parser};
use plaindoc_formats::pdf::{OrderBox, PdfParser, xy_cut};
use std::path::PathBuf;

/// Copy of `pdf::tests::builder_pdf_bytes` (benches cannot see `#[cfg(test)]`).
fn builder_pdf_bytes(pages: usize) -> Vec<u8> {
    let mut b = DocumentBuilder::new();
    for i in 0..pages {
        b.letter_page()
            .font("Helvetica", 12.0)
            .at(72.0, 720.0)
            .heading(1, &format!("Chapter {i}"))
            .font("Helvetica", 12.0)
            .at(72.0, 690.0)
            .paragraph("First paragraph of body text on this page.")
            .at(72.0, 660.0)
            .paragraph("Second paragraph with a few more words in it.")
            .done();
    }
    b.build().unwrap()
}

fn page_count(bytes: &[u8]) -> pdf_oxide::Result<u64> {
    PdfDocument::from_bytes(bytes.to_vec())
        .and_then(|d| d.page_count())
        .map(|n| n as u64)
}

fn bench_pdf(c: &mut Criterion) {
    let parser = PdfParser::new();
    let mut g = c.benchmark_group("pdf");

    let notes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/pdf/notes.pdf"
    ))
    .unwrap();
    g.throughput(Throughput::Elements(
        page_count(&notes).expect("notes.pdf page count"),
    ));
    g.bench_function("notes_3p", |b| {
        b.iter(|| {
            parser
                .parse(
                    Input::Bytes(notes.clone()),
                    Format::Pdf,
                    &ParseOptions::default(),
                )
                .unwrap()
        })
    });

    let built = builder_pdf_bytes(48);
    g.throughput(Throughput::Elements(48));
    for (name, parallel) in [("builder_48p/seq", false), ("builder_48p/par", true)] {
        let mut opts = ParseOptions::default();
        opts.page_parallel = parallel;
        g.bench_function(name, |b| {
            b.iter(|| {
                parser
                    .parse(Input::Bytes(built.clone()), Format::Pdf, &opts)
                    .unwrap()
            })
        });
    }

    match std::env::var_os("ODL_BENCH_PDFS") {
        Some(dir) => {
            let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.extension().is_some_and(|e| e == "pdf"))
                .collect();
            paths.sort();
            let files: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
            let opts = ParseOptions::default();
            // Setup only: count pages of openable files and report rejected ones instead of hiding them.
            let (mut pages, mut rejected) = (0u64, 0usize);
            for f in &files {
                let n = page_count(f).ok();
                pages += n.unwrap_or(0);
                let parsed = parser.parse(Input::Bytes(f.clone()), Format::Pdf, &opts);
                if n.is_none() || parsed.is_err() {
                    rejected += 1;
                }
            }
            eprintln!(
                "odl_200: {} files, {pages} pages, {rejected} fail to open or parse",
                files.len()
            );
            g.sample_size(10);
            g.throughput(Throughput::Elements(pages));
            g.bench_function("odl_200", |b| {
                b.iter(|| {
                    for f in &files {
                        // Some corpus files may be rejected; only throughput matters here.
                        let _ = parser.parse(Input::Bytes(f.clone()), Format::Pdf, &opts);
                    }
                })
            });
        }
        None => eprintln!("odl_200 skipped: ODL_BENCH_PDFS not set"),
    }
    g.finish();
}

fn bx(x0: f32, y0: f32, x1: f32, y1: f32) -> OrderBox {
    OrderBox {
        x0,
        y0,
        x1,
        y1,
        font_size: 10.0,
    }
}

/// The adversarial 2000-block pages of `pdf::order::tests` (copies: benches cannot see
/// `#[cfg(test)]`); the service budget is 50 ms per page in release.
fn adversarial() -> Vec<(&'static str, Vec<OrderBox>)> {
    const N: usize = 2_000;
    let rows = |cols: usize| -> Vec<OrderBox> {
        let w = 500.0 / cols as f32;
        (0..N)
            .map(|i| {
                let (r, c) = ((i / cols) as f32, (i % cols) as f32);
                let y = 20_000.0 - 16.0 * r;
                bx(72.0 + w * c, y, 72.0 + w * c + w - 20.0, y + 10.0)
            })
            .collect()
    };
    let edge = (0..N)
        .map(|i| {
            let f = i as f32;
            bx(6.0 * f, 0.0, 6.0 * f + 0.5, 10.0 + (i % 7) as f32)
        })
        .collect();
    let stairs = (0..N)
        .map(|i| {
            let f = i as f32;
            bx(
                10.0 * f,
                10_000.0 - 10.0 * f,
                10.0 * f + 4.0,
                10_004.0 - 10.0 * f,
            )
        })
        .collect();
    // 20 narrow gutter-crossing blocks 5 pt apart at the top and the bottom (no gutter cut), 980
    // rows of two column paragraphs with 30 pt row gaps between them
    let (mut crossing, mut y) = (Vec::new(), 1e6f32);
    let narrow = |b: &mut Vec<OrderBox>, y: &mut f32| {
        for _ in 0..20 {
            b.push(bx(250.0, *y - 10.0, 350.0, *y));
            *y -= 15.0;
        }
    };
    narrow(&mut crossing, &mut y);
    for _ in 0..980 {
        crossing.push(bx(72.0, y - 70.0, 290.0, y));
        crossing.push(bx(320.0, y - 70.0, 540.0, y));
        y -= 100.0;
    }
    y += 25.0;
    narrow(&mut crossing, &mut y);
    vec![
        ("crossing_stack", crossing),
        ("rows_2_columns", rows(2)),
        ("rows_4_columns", rows(4)),
        ("one_bottom_edge", edge),
        ("staircase", stairs),
        ("peeled_stack_fs_1000", peel(1000.0, false)),
        ("peeled_stack_fs_1000_shuffled", peel(1000.0, true)),
    ]
}

/// Copy of `pdf::order::tests::peel`: a gutter crosser over two column blocks at the top, 1997
/// one-pt blocks alternating left and right below (5 pt gaps near the top, 6 pt further down); hits
/// the XY-cut work limit (`None`).
fn peel(fs: f32, shuffle: bool) -> Vec<OrderBox> {
    let b = |x0, y0, x1, y1| OrderBox {
        x0,
        y0,
        x1,
        y1,
        font_size: fs,
    };
    let mut v = vec![
        b(250.0, 0.0, 350.0, 100.0),
        b(72.0, 0.0, 290.0, 100.0),
        b(320.0, 0.0, 540.0, 100.0),
    ];
    let (mut y, mut k) = (0.0f32, 0);
    while v.len() < 2_000 {
        y -= if -y < 6.0 * fs + 10.0 { 5.0 } else { 6.0 };
        let (x0, x1) = if k % 2 == 0 {
            (72.0, 290.0)
        } else {
            (320.0, 540.0)
        };
        v.push(b(x0, y - 1.0, x1, y));
        y -= 1.0;
        k += 1;
    }
    if shuffle {
        let mut r = 0x9E37_79B9_7F4A_7C15u64;
        for i in (1..v.len()).rev() {
            r ^= r << 13;
            r ^= r >> 7;
            r ^= r << 17;
            v.swap(i, (r % (i as u64 + 1)) as usize);
        }
    }
    v
}

fn bench_xy_cut(c: &mut Criterion) {
    let mut g = c.benchmark_group("xy_cut");
    for (name, blocks) in adversarial() {
        let t = std::time::Instant::now();
        let n = xy_cut(&blocks).map_or(0, |o| o.len());
        eprintln!(
            "xy_cut {name}: {n} blocks, one run {:?} (budget 50 ms)",
            t.elapsed()
        );
        g.bench_function(name, |b| b.iter(|| xy_cut(&blocks)));
    }
    g.finish();
}

criterion_group!(benches, bench_pdf, bench_xy_cut);
criterion_main!(benches);
