//! Golden tests against the Python reference (ocr-need `pagefeat.py`, `phase2_v2.decide2`).
//!
//! The text fixture carries reference `bigram_lp` values computed with
//! `data/need_ocr/bigrams.json` (see its NOTICE). A build without
//! the table checks only the text rows whose reference `bigram_lp` is null.

use super::bigrams::Bigrams;
use super::text::{Lang, TextFeatures};
use super::{EngineHints, InkFeatures, NeedOcrConfig, NeedReason, PageSignals, decide};
use serde_json::Value;

const DECIDE: &str = include_str!("../../../tests/fixtures/need_ocr/decide_golden.jsonl");
const TEXT: &str = include_str!("../../../tests/fixtures/need_ocr/text_golden.jsonl");
#[cfg(feature = "ocr")]
const INK: &str = include_str!("../../../tests/fixtures/need_ocr/ink_golden.jsonl");

fn rows(s: &str) -> Vec<Value> {
    s.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}
/// Text rows checkable in this build: all with the bundled table, else those whose reference
/// `bigram_lp` (column 13) is null.
fn checkable_text(s: &str) -> Vec<Value> {
    let table = Bigrams::bundled().is_some();
    rows(s)
        .into_iter()
        .filter(|r| table || r[13].is_null())
        .collect()
}
fn num(v: &Value) -> f32 {
    v.as_f64().unwrap_or(0.0) as f32
}

/// Text row: `[text, nonws, letter, digit, punct, cyr, lat, pua(+ctrl), upper, n_words,
/// n_alpha_words4, novowel, lang, bigram_lp, bigram_n]`.
fn check_text_rows(rs: &[Value], big: Option<&Bigrams>) {
    for r in rs {
        let r = r.as_array().unwrap();
        let t = r[0].as_str().unwrap();
        let f = TextFeatures::from_text(t, big);
        let n = |i: usize| r[i].as_u64().unwrap() as usize;
        assert_eq!(
            (f.nonws, f.n_words, f.n_alpha_words4, f.bigram_n),
            (n(1), n(9), n(10), n(14)),
            "{t:?}"
        );
        for (v, i) in [
            (f.letter_share, 2),
            (f.digit_share, 3),
            (f.punct_share, 4),
            (f.cyr_share, 5),
            (f.lat_share, 6),
            (f.pua_share + f.ctrl_share, 7),
            (f.upper_share, 8),
            (f.novowel_share, 11),
        ] {
            assert!(
                (v - num(&r[i])).abs() <= 1e-4,
                "col {i}: {v} vs {} in {t:?}",
                r[i]
            );
        }
        let lang = if r[12] == "ru" { Lang::Ru } else { Lang::En };
        assert_eq!(f.lang, Some(lang), "{t:?}");
        match (f.bigram_lp, r[13].as_f64()) {
            (None, None) => {}
            (Some(a), Some(b)) => {
                assert!((a as f64 - b).abs() <= 5.1e-4, "bigram {a} vs {b} in {t:?}")
            }
            other => panic!("bigram {other:?} in {t:?}"),
        }
    }
}

#[test]
fn text_features_match_reference() {
    let big = Bigrams::bundled();
    let rs = checkable_text(TEXT);
    assert!(
        rs.len() >= if big.is_some() { 34 } else { 10 },
        "{}",
        rs.len()
    );
    check_text_rows(&rs, big.as_deref());
}

/// Decide row: `[round, stratum, is_image, nonws, glyph_cc_outside, img_max_cover, n_alpha_words4,
/// pua_share, letter_share, punct_share, bigram_lp, novowel_share, ink_rows_outside, glyph_cc_all,
/// reason@RECALL, reason@BALANCED]`; reasons N/B/G/I/S/T as in `reason_code`.
fn signals(r: &[Value]) -> PageSignals {
    let u = |i: usize| r[i].as_f64().unwrap() as u32;
    PageSignals {
        standalone_image: r[2] == 1,
        text: TextFeatures {
            nonws: u(3) as usize,
            n_alpha_words4: u(6) as usize,
            pua_share: num(&r[7]),
            letter_share: num(&r[8]),
            punct_share: num(&r[9]),
            bigram_lp: r[10].as_f64().map(|v| v as f32),
            novowel_share: num(&r[11]),
            ..Default::default()
        },
        image_cover: num(&r[5]),
        ink: Some(InkFeatures {
            glyph_cc_outside: u(4),
            ink_rows_outside: u(12),
            glyph_cc_all: u(13),
            ..Default::default()
        }),
        hints: EngineHints::default(),
    }
}

/// Reference reason (`phase2_v2.decide2`) as its fixture code: `no_text_layer` N,
/// `blank_no_text` B, `garbage_layer` G, `image_page_layer_bad` I (both scan-bad and ink-outside),
/// `image_page_with_layer_ok` S, `text_ok` T.
fn reason_code(r: NeedReason) -> &'static str {
    match r {
        NeedReason::NoTextLayer => "N",
        NeedReason::Blank => "B",
        NeedReason::GarbageLayer => "G",
        NeedReason::ScanLayerBad | NeedReason::InkOutsideLayer => "I",
        NeedReason::ScanLayerOk => "S",
        NeedReason::TextOk => "T",
    }
}

fn check_decide_rows(rs: &[Value]) {
    let rounds: std::collections::BTreeSet<&str> =
        rs.iter().map(|r| r[0].as_str().unwrap()).collect();
    assert_eq!(rounds.len(), 2, "rows from both rounds");
    for (col, cfg) in [(14, NeedOcrConfig::RECALL), (15, NeedOcrConfig::BALANCED)] {
        for r in rs {
            let r = r.as_array().unwrap();
            let got = decide(&signals(r), &cfg);
            assert_eq!(
                reason_code(got),
                r[col].as_str().unwrap(),
                "col {col}: {r:?}"
            );
            assert_eq!(
                got.needs_ocr(),
                matches!(r[col].as_str(), Some("N" | "G" | "I"))
            );
        }
    }
}

#[test]
fn decide_matches_reference_on_both_operating_points() {
    // `bigram_lp` is an input literal here: every row is checkable without the table.
    let rs = rows(DECIDE);
    assert!(rs.len() >= 51, "{}", rs.len());
    check_decide_rows(&rs);
}

/// Synthetic page (same generator as the Python reference run): dark rectangles on a light
/// background, every third one with a text box. Pixels equal points at 72 dpi.
#[cfg(feature = "ocr")]
fn synth_page(seed: u64, w: usize, h: usize, n: usize) -> (Vec<u8>, Vec<[f32; 4]>) {
    let mut s = seed;
    let mut next = || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 33) as usize
    };
    let mut g = vec![250u8; w * h];
    let mut boxes = Vec::new();
    for i in 0..n {
        let (x, y) = (next() % (w - 30), next() % (h - 30));
        let (bw, bh, v) = (2 + next() % 30, 2 + next() % 22, (next() % 120) as u8);
        for yy in y..y + bh {
            g[yy * w + x..yy * w + x + bw].fill(v);
        }
        if i % 3 == 0 {
            boxes.push([x as f32, y as f32, (x + bw) as f32, (y + bh) as f32]);
        }
    }
    (g, boxes)
}

#[cfg(feature = "ocr")]
#[test]
fn ink_features_match_reference_on_synthetic_pages() {
    let rs = rows(INK);
    assert_eq!(rs.len(), 10);
    for r in rs {
        let u = |k: &str| r[k].as_u64().unwrap() as usize;
        let (w, h) = (u("w"), u("h"));
        let (g, boxes) = synth_page(u("seed") as u64, w, h, u("n"));
        let f = super::ink_features(&g, w, h, &boxes, (w as f32, h as f32), 72.0);
        assert_eq!(
            (
                f.glyph_cc_outside,
                f.cc_outside,
                f.ink_rows_outside,
                f.glyph_cc_all
            ),
            (
                u("glyph_cc_outside") as u32,
                u("cc_outside") as u32,
                u("ink_rows_outside") as u32,
                u("glyph_cc_all") as u32
            ),
            "seed {}",
            r["seed"]
        );
    }
}
