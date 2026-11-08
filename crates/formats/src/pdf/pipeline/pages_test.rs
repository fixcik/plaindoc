use super::*;
use plaindoc_core::ParseError;
use std::time::Duration;

fn pool(n: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(n)
        .build()
        .unwrap()
}

#[test]
fn page_workers_bounds() {
    assert_eq!(page_workers(None, 100), DEFAULT_PAGE_THREADS);
    assert_eq!(page_workers(Some(0), 100), 1);
    assert_eq!(page_workers(Some(1), 100), 1);
    assert_eq!(page_workers(Some(3), 100), 3);
    assert_eq!(page_workers(Some(64), 5), 5, "at most the pages");
    assert_eq!(page_workers(None, 0), 1);
}

#[test]
fn results_in_page_order_for_any_worker_count() {
    let want: Vec<usize> = (3..40).map(|p| p * 10).collect();
    for w in [1, 2, 5, 64] {
        let got = pool(4)
            .install(|| par_pages(3..40, w, |p| Ok(p * 10)))
            .unwrap();
        assert_eq!(got, want, "workers {w}");
    }
    assert!(par_pages(7..7, 4, Ok).unwrap().is_empty());
}

#[test]
fn first_error_in_page_order() {
    for w in [1, 3, 8] {
        let r = pool(4).install(|| {
            par_pages(0..50, w, |p| {
                if p == 9 {
                    // Fails later in time than page 30: the order must come from the page number.
                    std::thread::sleep(Duration::from_millis(30));
                    Err(ParseError::Internal(format!("page {p}")))
                } else if p == 30 {
                    Err(ParseError::Internal(format!("page {p}")))
                } else {
                    std::thread::sleep(Duration::from_micros(200));
                    Ok(p)
                }
            })
        });
        assert!(
            matches!(&r, Err(ParseError::Internal(m)) if m == "page 9"),
            "workers {w}"
        );
    }
}

#[test]
fn at_most_workers_pages_in_flight() {
    let (now, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
    pool(8)
        .install(|| {
            par_pages(0..60, 3, |p| {
                let n = now.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(2));
                now.fetch_sub(1, Ordering::SeqCst);
                Ok(p)
            })
        })
        .unwrap();
    let peak = peak.into_inner();
    assert!((2..=3).contains(&peak), "peak {peak}");
}
