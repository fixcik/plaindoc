#![cfg(feature = "office")]
mod common;
use common::*;
use plaindoc_core::*;
use rayon::prelude::*;

fn inputs() -> Vec<std::path::PathBuf> {
    all_fixtures()
        .into_iter()
        .filter(|f| !f.ends_with("encrypted.docx"))
        .collect()
}
fn json(p: &std::path::Path) -> String {
    registry()
        .parse(Input::Path(p.to_path_buf()), &ParseOptions::default())
        .unwrap()
        .to_json()
}

#[test]
fn parallel_over_office_thread_cap_matches_sequential() {
    let files: Vec<_> = inputs()
        .into_iter()
        .cycle()
        .take(inputs().len() * 4)
        .collect();
    let seq: Vec<String> = files.iter().map(|f| json(f)).collect();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(72)
        .build()
        .unwrap();
    let par: Vec<String> = pool.install(|| files.par_iter().map(|f| json(f)).collect());
    assert_eq!(seq, par);
}

#[test]
fn registry_parse_many_is_ordered_and_deterministic() {
    let files = inputs();
    let a: Vec<String> = registry()
        .parse_many(
            files.iter().cloned().map(Input::Path).collect(),
            &ParseOptions::default(),
        )
        .into_iter()
        .map(|r| r.unwrap().to_json())
        .collect();
    let b: Vec<String> = files.iter().map(|f| json(f)).collect();
    assert_eq!(a, b);
}

#[test]
fn repeated_parse_is_byte_identical() {
    for f in inputs() {
        assert_eq!(json(&f), json(&f), "{}", f.display());
    }
}

#[test]
fn deep_nesting_on_small_stack_rayon_worker() {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .stack_size(2 << 20)
        .build()
        .unwrap();
    let d = pool
        .install(|| parse_fixture("generated/deep.docx"))
        .unwrap();
    fn depth(b: &Block) -> usize {
        match b {
            Block::Table { rows, .. } => {
                1 + rows
                    .iter()
                    .flatten()
                    .flat_map(|c| &c.blocks)
                    .map(depth)
                    .max()
                    .unwrap_or(0)
            }
            _ => 0,
        }
    }
    assert!(d.sections[0].blocks.iter().map(depth).max().unwrap() >= 60);
    assert!(all_text(&d).contains("deep"));
}

#[test]
#[ignore = "measurement, run manually with --release"]
fn measure_throughput_by_pool_size() {
    let files: Vec<Vec<u8>> = inputs().iter().map(|f| std::fs::read(f).unwrap()).collect();
    let reg = registry();
    for n in [1usize, 4, 8, 18, 36, 72] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap();
        let work: Vec<usize> = (0..files.len() * 20).map(|i| i % files.len()).collect();
        let t = std::time::Instant::now();
        pool.install(|| {
            work.par_iter().for_each(|&i| {
                reg.parse(Input::Bytes(files[i].clone()), &ParseOptions::default())
                    .unwrap();
            })
        });
        eprintln!(
            "threads={n:2} files/s={:.0}",
            work.len() as f64 / t.elapsed().as_secs_f64()
        );
    }
}
