use super::*;

fn feat() -> Features {
    Features {
        nrows: 5.0,
        ncols: 3.0,
        fill: 1.0,
        nw: 30.0,
        mean_words: 1.0,
        line_w_mean: 0.8,
        ..Default::default()
    }
}

#[test]
fn rules_in_filter_order() {
    assert_eq!(rule_reject(&feat(), CandidateKind::Lined), None);
    assert_eq!(
        rule_reject(
            &Features {
                n_diag: 8.0,
                ..feat()
            },
            CandidateKind::Lined
        ),
        Some(RejectReason::Chart)
    );
    assert_eq!(
        rule_reject(
            &Features {
                nrows: 2.0,
                ncols: 2.0,
                ..feat()
            },
            CandidateKind::Lined
        ),
        Some(RejectReason::Tiny)
    );
    assert_eq!(
        rule_reject(
            &Features {
                sparse_rows: 0.5,
                ..feat()
            },
            CandidateKind::Lined
        ),
        Some(RejectReason::Sparse)
    );
    let prose = Features {
        ncols: 2.0,
        col_w_cv: 0.0,
        line_w_mean: 0.9,
        mean_words: 4.0,
        ..feat()
    };
    assert_eq!(
        rule_reject(&prose, CandidateKind::Text),
        Some(RejectReason::TwoColumnProse)
    );
    assert_eq!(
        rule_reject(&prose, CandidateKind::Lined),
        None,
        "twocol is for text candidates only"
    );
    assert_eq!(
        rule_reject(
            &Features {
                numcols: 1.0,
                ..prose
            },
            CandidateKind::Text
        ),
        None
    );
}

#[test]
fn probability_matches_python_on_a_reference_vector() {
    // syn_000_plain page 0 candidate 0: features and fsyn.py `prob` from the Python prototype.
    let f = Features {
        nrows: 13.0,
        ncols: 2.0,
        fill: 1.0,
        mean_words: 1.5,
        num_frac: 0.3077,
        numcols: 1.0,
        numcol_frac: 0.5,
        sparse_rows: 0.0,
        line_w_mean: 0.8368,
        nlines: 13.0,
        n_h: 14.0,
        n_v: 3.0,
        h_len: 0.9992,
        v_len: 0.9989,
        n_paths: 68.0,
        n_segs: 0.0,
        n_diag: 0.0,
        fill_area: 1.0,
        paths_per_word: 1.7436,
        row_h_cv: 0.0,
        col_w_cv: 0.2676,
        align_cv: 0.0,
        w_frac: 0.27,
        h_frac: 0.2675,
        fs_ratio: 0.9091,
        first_col_words: 1.0,
        last_col_num: 0.0,
        header_like: 1.0,
        word_share: 0.2203,
        nw: 39.0,
        short_frac: 1.0,
        row_h_fs: 1.725,
    };
    let p = probability(&f, CandidateKind::Lined);
    assert!((p - 0.972_350_713_801_345_4).abs() < 1e-9, "{p}");
}

#[test]
fn hybrid_guard_is_two_sided() {
    let pl = |nrows: usize| Planar {
        rows: vec![],
        nonrect: 0,
        ncols: 3,
        nrows,
        ncells: 12,
        edge_cov: 1.0,
    };
    let cand = |nrows: usize| Candidate {
        kind: CandidateKind::Lined,
        bbox: [0.0; 4],
        cells: [0.0; 4],
        rows: vec![vec![String::new(); 3]; 10],
        features: Features::default(),
        planar: Some(pl(nrows)),
        rotated: false,
        head: 0,
        gutter_prose: false,
        box_flow: false,
        text_block: false,
    };
    assert!(use_planar(&cand(10)));
    assert!(use_planar(&cand(6)) && use_planar(&cand(14)));
    assert!(!use_planar(&cand(5)), "loses rows (ODL-45)");
    assert!(!use_planar(&cand(15)), "inflates rows (rounded_cells)");
}

#[test]
fn header_rows_from_spans_and_text() {
    let c = |t: &str, cs: u32, rs: u32| GridCell::new(t, cs, rs);
    let multi = vec![
        vec![c("Region", 1, 2), c("2019", 2, 1)],
        vec![c("Q1", 1, 1), c("Q2", 1, 1)],
        vec![c("A", 1, 1), c("1", 1, 1), c("2", 1, 1)],
    ];
    assert_eq!(header_rows(&multi), 2);
    let plain = vec![
        vec![c("Name", 1, 1), c("Qty", 1, 1)],
        vec![c("a", 1, 1), c("3", 1, 1)],
    ];
    assert_eq!(header_rows(&plain), 1);
    let years = vec![
        vec![c("2019", 1, 1), c("2020", 1, 1)],
        vec![c("1", 1, 1), c("2", 1, 1)],
    ];
    assert_eq!(header_rows(&years), 1, "numeric headers are headers too");
    assert_eq!(header_rows(&plain[..1]), 0, "a header needs a body row");
}

fn text_block(rows: &[&[&str]]) -> Candidate {
    Candidate {
        kind: CandidateKind::Text,
        bbox: [0.0; 4],
        cells: [0.0; 4],
        rows: rows
            .iter()
            .map(|r| r.iter().map(|t| t.to_string()).collect())
            .collect(),
        features: feat(),
        planar: None,
        rotated: false,
        head: 0,
        gutter_prose: false,
        box_flow: false,
        text_block: true,
    }
}

/// SP5 22.3: OCR keeps pieces of dot leaders and joins equation numbers to the formula line.
#[test]
fn scan_guards_reject_toc_and_equation_numbers_only() {
    let toc = text_block(&[
        &["1 Введение....", "9"],
        &["1.1 Цели и задачи. .", ".9"],
        &["1.2 Выводы...", ".9"],
        &["2 Общие положения.", "11"],
    ]);
    let eq = text_block(&[
        &["S = P (1 + rn)nt", "(1)"],
        &["где N — объём выборки", ""],
        &["HDC = 20/ 120", "(2)"],
    ]);
    let table = text_block(&[
        &["Товар", "Кол-во", "Цена"],
        &["Яблоки", "12", "3.50"],
        &["Груши (сорт 2)", "7", "(4)"],
        &["Сливы", "..", "11"],
    ]);
    let apa = text_block(&[
        &["Variable", "r", "p"],
        &["Age", ".45", ".001"],
        &["Income", ".31", ".05"],
        &["Tenure", ".12", ".20"],
    ]);
    let apa2 = text_block(&[&["Age.", ".45"], &["Income", ".05"], &["Tenure", ".12"]]);
    let stats = text_block(&[
        &["Country", "2019", "2020", "2021"],
        &["Albania", "..", "..", "12"],
        &["Belarus", "4.1", "..", "15"],
        &["Chad", "..", "..", "9"],
    ]);
    let stats3 = text_block(&[
        &["Country", "2020", "2021"],
        &["Albania", "..", "12"],
        &["Chad", "..", "9"],
    ]);
    assert!(scan_decoy(&toc) && scan_decoy(&eq));
    for t in [&apa, &apa2, &stats, &stats3] {
        assert!(!scan_decoy(t), "{:?}", t.rows);
    }
    assert!(
        !scan_decoy(&table),
        "one leader-like row and one bracketed value"
    );
    let lined = Candidate {
        text_block: false,
        ..toc.clone()
    };
    assert!(
        !scan_decoy(&lined),
        "rule groups and lined tables are not touched"
    );
    let off = TableConfig::default();
    let on = TableConfig {
        scan_text_guards: true,
        ..TableConfig::default()
    };
    assert_eq!(filter_reason(&toc, 1.0, &off, LOGIT_THRESHOLD), None);
    assert_eq!(
        filter_reason(&toc, 1.0, &on, LOGIT_THRESHOLD),
        Some(RejectReason::ScanDecoy)
    );
}
