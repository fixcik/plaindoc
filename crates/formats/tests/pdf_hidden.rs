//! Hidden text golden set: 34 attacks (`h*`, payload `PWNED<n>`) and 24 look-alike controls
//! (`c*`, text `KEEP<n>` must stay); provenance in `tests/fixtures/pdf/hidden/README.md`.
use plaindoc_core::*;
use plaindoc_formats::pdf::{PdfConfig, PdfParser};
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdf/hidden")
}

struct Case {
    file: String,
    absent: Vec<String>,
    present: Vec<String>,
}

fn manifest() -> Vec<Case> {
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir().join("manifest.json")).unwrap()).unwrap();
    let strs = |x: &serde_json::Value| -> Vec<String> {
        x.as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect()
    };
    v.as_array()
        .unwrap()
        .iter()
        .map(|c| Case {
            file: c["file"].as_str().unwrap().to_string(),
            absent: strs(&c["absent"]),
            present: strs(&c["present"]),
        })
        .collect()
}

fn parse(file: &str, mode: HiddenText) -> Document {
    parse_with(file, mode, false)
}

/// `facts`: `PdfConfig::visibility_facts` (phase 2 rules).
fn parse_with(file: &str, mode: HiddenText, facts: bool) -> Document {
    let mut o = ParseOptions::default();
    o.hidden_text = mode;
    let mut c = PdfConfig::default();
    c.visibility_facts = facts;
    PdfParser::with_config(c)
        .parse(Input::Path(dir().join(file)), Format::Pdf, &o)
        .unwrap()
}

/// Markdown without format characters, so a payload split by ZWSP or reversed by RLO still counts.
fn visible(d: &Document) -> String {
    d.to_markdown()
        .chars()
        .filter(|&c| {
            !matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}' | '\u{E0000}'..='\u{E007F}')
        })
        .collect()
}

/// `(leaking attacks, controls with lost text)` by file prefix (`h06`, `c13`).
fn run(mode: HiddenText) -> (Vec<String>, Vec<String>) {
    run_with(mode, false)
}

fn run_with(mode: HiddenText, facts: bool) -> (Vec<String>, Vec<String>) {
    let (mut leak, mut lost) = (Vec::new(), Vec::new());
    for c in manifest() {
        let t = visible(&parse_with(&c.file, mode, facts));
        let id = c.file[..3].to_string();
        if c.absent
            .iter()
            .any(|a| t.contains(a.as_str()) || t.contains(&a.chars().rev().collect::<String>()))
        {
            leak.push(id.clone());
        }
        if c.present.iter().any(|p| !t.contains(p.as_str())) {
            lost.push(id);
        }
    }
    (leak, lost)
}

/// Phase 2 (fork facts) leaves these open (P2-1 closed h11, h12, h33). h27 stays open here: these
/// runs have no OCR engine, and the layer is checked against OCR only on pages OCRed anyway, which
/// a scan with a layer is not under `OcrMode::Auto` (`pdf_hidden_ocr.rs` closes it in `Always`).
const OPEN_AFTER_PHASE1: [&str; 17] = [
    "h01", "h02", "h03", "h04", "h05", "h13", "h14", "h15", "h16", "h17", "h18", "h27", "h29",
    "h30", "h31", "h32", "h34",
];
/// Lost before SP8 as well: h27 (fake OCR layer replaces the visible words; without OCR, as in
/// `OPEN_AFTER_PHASE1`), c23 (form field values are not extracted). c13 is kept since its Noto regeneration: one font per script gives separate
/// spans, and the fork no longer reverses the Latin line next to RTL (it did with one Arial Unicode
/// span).
const LOST_BEFORE_SP8: [&str; 2] = ["h27", "c23"];

#[test]
fn strip_closes_phase1_channels_and_keeps_controls() {
    let (leak, lost) = run(HiddenText::Strip);
    assert_eq!(leak, OPEN_AFTER_PHASE1, "{leak:?}");
    assert_eq!(lost, LOST_BEFORE_SP8, "{lost:?}");
}

#[test]
fn off_is_the_main_baseline() {
    let (leak, lost) = run(HiddenText::Off);
    assert_eq!(leak.len(), 26, "{leak:?}");
    assert_eq!(lost, LOST_BEFORE_SP8, "{lost:?}");
}

/// Files with a `HiddenContent`/`Warn` warning.
fn warned(mode: HiddenText) -> Vec<String> {
    manifest()
        .iter()
        .filter(|c| {
            let d = parse(&c.file, mode);
            d.sections
                .iter()
                .flat_map(|s| &s.warnings)
                .any(|w| w.kind == WarningKind::HiddenContent && w.level == Level::Warn)
        })
        .map(|c| c.file[..3].to_string())
        .collect()
}

#[test]
fn flag_keeps_text_and_warns_on_the_same_pages() {
    // c08: the alternate-language layer is off by default, by design of the file; h12: the fork
    // filters `/OC` on XObjects since 0299ec9
    let w = warned(HiddenText::Flag);
    let want = [
        "h06", "h07", "h08", "h11", "h12", "h24", "h25", "h26", "h33", "c08",
    ];
    assert_eq!(w, want, "{w:?}");
    assert_eq!(run(HiddenText::Flag).0.len(), 26);
}

#[test]
fn strip_removes_default_off_layers() {
    assert_eq!(warned(HiddenText::Strip), Vec::<String>::new());
    let ids = ["h11", "h12", "h33", "c08"];
    for c in manifest().iter().filter(|c| ids.contains(&&c.file[..3])) {
        let w = hidden_warnings(&c.file, HiddenText::Strip, false);
        assert_eq!(w, [("ocg_off", Level::Info)], "{}", c.file);
    }
}

#[test]
fn strip_reports_removed_text_as_info() {
    let d = parse("h06_tiny_0_5pt.pdf", HiddenText::Strip);
    let w: Vec<&Warning> = d.sections[0]
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::HiddenContent)
        .collect();
    assert_eq!(
        (w.len(), w[0].level, w[0].code),
        (1, Level::Info, Some("tiny"))
    );
    assert!(
        w[0].message.starts_with("hidden text removed: tiny="),
        "{}",
        w[0].message
    );
}

/// With visibility facts on (phase 2): what `Strip` still leaks. Each phase-2 rule closes its own
/// files (`alpha`: h17, h18; `clip`: h15, h16); `invisible` (h13, h14), `overlap` (h04, h05, h32,
/// h34) and `contrast` (h01-h03, h29, h31) only flag, also in `Strip`. The controls stay as without
/// facts.
const OPEN_WITH_FACTS: [&str; 13] = [
    "h01", "h02", "h03", "h04", "h05", "h13", "h14", "h27", "h29", "h30", "h31", "h32", "h34",
];

#[test]
fn strip_with_facts_closes_phase2_channels_and_keeps_controls() {
    let (leak, lost) = run_with(HiddenText::Strip, true);
    assert_eq!(leak, OPEN_WITH_FACTS, "{leak:?}");
    assert_eq!(lost, LOST_BEFORE_SP8, "{lost:?}");
}

#[test]
fn off_with_facts_config_is_the_main_baseline() {
    // `visibility_facts` takes effect only with hidden-text rules on: files whose `Strip` output
    // changes with facts give the same `Off` document with and without the setting
    for f in [
        "h09_off_mediabox.pdf",
        "h15_clip_zero.pdf",
        "h17_opacity_0.pdf",
    ] {
        assert_ne!(
            parse_with(f, HiddenText::Strip, true),
            parse_with(f, HiddenText::Strip, false),
            "{f}"
        );
        assert_eq!(
            parse_with(f, HiddenText::Off, true),
            parse_with(f, HiddenText::Off, false),
            "{f}"
        );
    }
    let (leak, lost) = run_with(HiddenText::Off, true);
    assert_eq!(leak.len(), 26, "{leak:?}");
    assert_eq!(lost, LOST_BEFORE_SP8, "{lost:?}");
}

/// `HiddenContent` warnings `(code, level)` of the first section.
fn hidden_warnings(file: &str, mode: HiddenText, facts: bool) -> Vec<(&'static str, Level)> {
    parse_with(file, mode, facts).sections[0]
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::HiddenContent)
        .map(|w| (w.code.unwrap(), w.level))
        .collect()
}

#[test]
fn offpage_text_dropped_by_the_fork_is_reported_with_facts() {
    // h09/h10: payload outside MediaBox / CropBox; c07: slug line outside CropBox (by design)
    for f in [
        "h09_off_mediabox.pdf",
        "h10_off_cropbox.pdf",
        "c07_bleed_marks.pdf",
    ] {
        assert_eq!(hidden_warnings(f, HiddenText::Strip, false), [], "{f}");
        for mode in [HiddenText::Strip, HiddenText::Flag] {
            // already gone in every mode: `Info` "removed", never "kept"
            assert_eq!(
                hidden_warnings(f, mode, true),
                [("offpage", Level::Info)],
                "{f} {mode:?}"
            );
        }
    }
}

#[test]
fn ocr_layers_are_not_invisible_text() {
    // Tr 3 over a scan image (c01), in GlyphLessFont (c02), partly outside an inset scan (c19):
    // neither removed nor flagged
    for f in [
        "c01_ocr_scan_tr3.pdf",
        "c02_ocr_scan_glyphless.pdf",
        "c19_ocr_scan_jitter.pdf",
    ] {
        for mode in [HiddenText::Strip, HiddenText::Flag] {
            assert_eq!(hidden_warnings(f, mode, true), [], "{f} {mode:?}");
        }
    }
    // only flagged, also in `Strip`: search layers over vector glyphs look the same
    for f in ["h13_tr3_no_image.pdf", "h14_tr7_clip.pdf"] {
        assert_eq!(
            hidden_warnings(f, HiddenText::Strip, true),
            [("invisible", Level::Warn)],
            "{f}"
        );
        assert_eq!(
            hidden_warnings(f, HiddenText::Flag, true),
            [("invisible", Level::Warn)],
            "{f}"
        );
    }
}

#[test]
fn transparent_text_is_stripped_and_half_opacity_kept() {
    for f in ["h17_opacity_0.pdf", "h18_opacity_0_02.pdf"] {
        assert_eq!(
            hidden_warnings(f, HiddenText::Strip, true),
            [("alpha", Level::Info)],
            "{f}"
        );
        assert_eq!(
            hidden_warnings(f, HiddenText::Flag, true),
            [("alpha", Level::Warn)],
            "{f}"
        );
    }
    // c12: `ca 0.5`; c20: stroke-only and fill+stroke render modes
    for f in ["c12_opacity_half.pdf", "c20_stroke_modes.pdf"] {
        for mode in [HiddenText::Strip, HiddenText::Flag] {
            assert_eq!(hidden_warnings(f, mode, true), [], "{f} {mode:?}");
        }
    }
}

#[test]
fn clipped_out_text_is_stripped_and_a_page_clip_kept() {
    for f in ["h15_clip_zero.pdf", "h16_clip_elsewhere.pdf"] {
        assert_eq!(
            hidden_warnings(f, HiddenText::Strip, true),
            [("clip", Level::Info)],
            "{f}"
        );
        assert_eq!(
            hidden_warnings(f, HiddenText::Flag, true),
            [("clip", Level::Warn)],
            "{f}"
        );
    }
    for mode in [HiddenText::Strip, HiddenText::Flag] {
        assert_eq!(
            hidden_warnings("c11_clip_normal.pdf", mode, true),
            [],
            "{mode:?}"
        );
    }
}

#[test]
fn controls_get_no_new_hidden_warnings_with_facts() {
    // known: c07's slug line outside CropBox (dropped by the fork, `Info`), c08's default-off
    // alternate-language layer (removed in `Strip`, flagged in `Flag`)
    for mode in [HiddenText::Strip, HiddenText::Flag] {
        let got: Vec<(String, Vec<(&'static str, Level)>)> = manifest()
            .iter()
            .filter(|c| c.file.starts_with('c'))
            .map(|c| {
                (
                    c.file[..3].to_string(),
                    hidden_warnings(&c.file, mode, true),
                )
            })
            .filter(|(_, w)| !w.is_empty())
            .collect();
        let c08 = [Level::Info, Level::Warn][usize::from(mode == HiddenText::Flag)];
        let want = vec![
            ("c07".to_string(), vec![("offpage", Level::Info)]),
            ("c08".to_string(), vec![("ocg_off", c08)]),
        ];
        assert_eq!(got, want, "{mode:?}");
    }
}

#[test]
fn text_under_a_later_paint_is_flagged() {
    // h04/h32: an image over the payload (h32 covers it only partly), h05: a white rectangle,
    // h34: a dark rectangle; only flagged, also in `Strip` (c10, text over an image, is a control)
    for f in [
        "h04_under_image.pdf",
        "h05_under_rect.pdf",
        "h32_under_image_partial.pdf",
        "h34_under_dark_rect.pdf",
    ] {
        assert_eq!(hidden_warnings(f, HiddenText::Strip, false), [], "{f}");
        for mode in [HiddenText::Strip, HiddenText::Flag] {
            assert_eq!(
                hidden_warnings(f, mode, true),
                [("overlap", Level::Warn)],
                "{f} {mode:?}"
            );
        }
    }
}

#[test]
fn text_in_the_colour_of_its_background_is_flagged() {
    // h01: white on the page, h02: 0.97 grey, h03: white on a white rectangle, h29: black on a
    // black rectangle, h31: CMYK white; only flagged, also in `Strip` (c03/c22 white on dark,
    // c09 light grey and c14 coloured on coloured are controls)
    for f in [
        "h01_white_on_white.pdf",
        "h02_near_white.pdf",
        "h03_white_on_white_rect.pdf",
        "h29_black_on_black.pdf",
        "h31_white_cmyk.pdf",
    ] {
        assert_eq!(hidden_warnings(f, HiddenText::Strip, false), [], "{f}");
        for mode in [HiddenText::Strip, HiddenText::Flag] {
            assert_eq!(
                hidden_warnings(f, mode, true),
                [("contrast", Level::Warn)],
                "{f} {mode:?}"
            );
        }
    }
}
