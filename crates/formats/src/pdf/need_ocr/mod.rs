//! "Does this page need OCR?" classifier (ocr-need research, rule cascade v2 `decide2`).
//!
//! All thresholds are named fields of [`NeedOcrConfig`] with provenance; the two presets are the
//! operating points tuned in round 2 (`work/final_params_v2_{0.98,0.95}.json`).
//!
//! Caveats of the presets:
//! - the round-2 test half is small (196 pages, 3–26 per stratum) and drawn from the same
//!   sources the thresholds were tuned on;
//! - the synthetic `d_poor` stratum (letter noise in the layer) is not caught;
//! - a third, independent sample is still needed, so every threshold can be overridden.
//!
//! Task 14a miss analysis (Rust signals from `classifier_text`, `classify_page` and the 50-dpi
//! render, against the PyMuPDF features of the same pages). Decisions agree with the reference on
//! 62/63 cross-check pages with the bigram table and 63/63 without it (reference also without it).
//! - `rb_lezgi` p3, p7 (`e_garbage_layer`, were `TextOk`): pdf_oxide's structured text drops glyphs
//!   that decode to U+FFFD, so `pua_share` read 0 instead of 0.41 / 0.53. Port fix in
//!   `classifier_text` (unmapped glyphs re-added from `extract_chars`); both are now
//!   `GarbageLayer`.
//! - `001157` p0, p3 and `001494` p0, p11 (`r2e_garbage`): pdf_oxide decodes these fonts to other
//!   characters than PyMuPDF (no U+FFFD and no control chars: PyMuPDF has 84–268 U+FFFD and
//!   94–752 control chars per page), so `nonws`, `bigram_lp` (-1.71 vs -2.06 on p3) and
//!   `novowel_share` differ. Different text from the same PDF, not a port bug; not fixed. Without
//!   the bigram table the reference misses three of these four pages too.
//! - Synthetic `d_scan_layer_poor` 005_ru, 027_en (`ScanLayerOk`): `image_cover` 0.9999 on both
//!   sides (the summed vs largest image area is not the cause); the reference misses them as well
//!   (letter noise, caveat above). Without the table 002_ru flips in the reference too.
//! - Other differences, all on negatives where Rust matches the label: `r2cm_33` p1 ink tie
//!   (`glyph_cc_outside` 102 vs 128 at the 127.5 limit); `002202` p2, where the reference counts
//!   NUL padding as PUA (Rust counts control chars apart, research REPORT §5.4).

pub(crate) mod bigrams;
#[cfg(feature = "ocr")]
pub(crate) mod ink;
pub(crate) mod text;

pub use bigrams::Bigrams;
#[cfg(feature = "ocr")]
pub(crate) use ink::ink_features;
pub(crate) use text::TextFeatures;

use pdf_oxide::{PdfDocument, RegionRole, StructuredPage};

/// Raster ink features of a page, computed from the 50-dpi render by `ink::ink_features`
/// (feature `ocr`). Without `ocr` nothing fills it and pages carry `ink = None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct InkFeatures {
    /// Glyph-like components of ink outside text-layer boxes.
    pub glyph_cc_outside: u32,
    /// All components of ink outside text-layer boxes.
    pub cc_outside: u32,
    /// Bands of rows with ink outside text boxes.
    pub ink_rows_outside: u32,
    /// Glyph-like components of all ink.
    pub glyph_cc_all: u32,
}

/// Thresholds of the cascade. Field comments give the `final_params_v2_*.json` key.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NeedOcrConfig {
    /// `min_chars`: fewer non-space chars means "no text layer".
    pub min_chars: usize,
    /// `g_nolayer` (G0): glyph-like components needed to OCR a page without a layer.
    pub g_nolayer: u32,
    /// `images_always_ocr`: standalone images always go to OCR (det decides).
    pub images_always_ocr: bool,
    /// `g_img_rows`: ink row bands that send an image file to OCR when `images_always_ocr` is off.
    pub g_img_rows: u32,
    /// `g_img_cc`: glyph-like components that send an image file to OCR when
    /// `images_always_ocr` is off.
    pub g_img_cc: u32,
    /// `pua`: PUA/U+FFFD share that marks a garbage layer.
    pub pua: f32,
    /// `letter_scan`: a scan's layer is bad below this letter share.
    pub letter_scan: f32,
    /// `punct_scan`: a scan's layer is bad above this punctuation share.
    pub punct_scan: f32,
    /// `nw`: words of >= 4 letters for the text to count as prose.
    pub nw: usize,
    /// `bigram`: prose garbage below this mean log10 bigram probability.
    pub bigram: f32,
    /// `bigram_lowletter`: as `bigram`, when `letter_share < prose_letter_share`.
    pub bigram_lowletter: f32,
    /// `novowel`: prose garbage above this share of vowel-less words.
    pub novowel: f32,
    /// `g_abs`: components outside the layer needed for "text exists only as pixels".
    pub g_abs: u32,
    /// `g_ratio`: and those components must also reach `g_ratio * nonws`.
    pub g_ratio: f32,
    /// Image cover at which a page counts as a scan (`img_max_cover >= 0.8` in the reference).
    pub scan_cover: f32,
    /// Letter share separating prose from tables/formulas (0.5 in the reference).
    pub prose_letter_share: f32,
    /// Render gate: skip the 50-dpi render when the page has a layer, no image and fewer
    /// painted paths than this (flips 2 of 1672 research pages, both towards the label).
    pub render_min_paths: usize,
    /// Render gate, images: a page with a layer whose images cover at most this share of it
    /// (summed, `classify_page`) counts as a page without images, so a small running logo or an
    /// icon does not render every page (on a Chromium brochure the 50-dpi render of eight pages
    /// was a third of the parse, all for a header logo of 0.6 %). 0.01 is a square of about
    /// 70 pt (2.5 cm) on A4: little room for the `g_abs` glyph-like components of text in an image.
    /// The pipeline applies it under `OcrMode::Auto` only (0 in the other modes).
    pub render_min_image_cover: f32,
}

impl NeedOcrConfig {
    /// Recall >= 0.98 point (the default `DiagnosticsConfig::need_ocr`): R1 test acc 0.949 /
    /// recall 0.976, R2 test-half acc 0.985 / recall 1.000, born-digital false triggers 2.9 %.
    ///
    /// Measured on PyMuPDF features (Python, with the bigram table).
    ///
    /// Task 14a gate, Rust port on pdf_oxide features (Wilson 95 % in brackets):
    /// - shipped (default config, bundled public-domain bigram table): R2 test-half recall
    ///   71/73 = 0.973 [0.91, 0.99], born-digital false triggers 1/104 = 1.0 %; 63-page
    ///   cross-check recall 26/29 = 0.90 [0.74, 0.96]; page for page the same decisions as with
    ///   the research's Russian Wikipedia table. Gate (user's ruling: not worse than the first
    ///   table): R2 >= 71/73, FTR <= 1/104, cross-check >= 26/29; the plan's 27/29 is missed by
    ///   one page (`001157` p3, pdf_oxide decodes its font differently from PyMuPDF).
    /// - built without the table: R2 test-half 69/73 = 0.945 [0.87, 0.98], FTR 1/104;
    ///   cross-check 22/29 = 0.76 [0.58, 0.88].
    /// - Python reference on the same pages: with the research table 73/73 and 27/29; without it
    ///   70/73 = 0.959 and 22/29, so without the table the cross-check gate is out of reach of
    ///   the reference itself.
    pub const RECALL: Self = Self {
        min_chars: 20,
        g_nolayer: 5,
        images_always_ocr: true,
        g_img_rows: 1,
        g_img_cc: 50,
        pua: 0.15,
        letter_scan: 0.3,
        punct_scan: 0.4,
        nw: 15,
        bigram: -1.35,
        bigram_lowletter: -1.9,
        novowel: 0.3,
        g_abs: 60,
        g_ratio: 0.5,
        scan_cover: 0.8,
        prose_letter_share: 0.5,
        render_min_paths: 200,
        render_min_image_cover: 0.01,
    };

    /// Balanced point: R1 test acc 0.940 / recall 0.856, R2 test-half acc 0.980 / recall 0.973.
    pub const BALANCED: Self = Self {
        g_nolayer: 50,
        bigram: -1.45,
        g_ratio: 2.0,
        ..Self::RECALL
    };
}

impl Default for NeedOcrConfig {
    fn default() -> Self {
        Self::RECALL
    }
}

/// Why the classifier decided as it did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NeedReason {
    /// No layer, glyph-like ink present (or engine says scanned without layer, no render).
    NoTextLayer,
    /// No layer, no ink.
    Blank,
    /// Layer of PUA / mojibake / vowel-less prose.
    GarbageLayer,
    /// Scan whose layer is mostly digits and punctuation.
    ScanLayerBad,
    /// Layer present, but much more text exists only as pixels.
    InkOutsideLayer,
    /// Scan with a usable layer.
    ScanLayerOk,
    /// Usable text layer.
    TextOk,
}

impl NeedReason {
    /// Used by the gate harness (feature `ocr`) and tests.
    #[cfg_attr(not(feature = "ocr"), allow(dead_code))]
    pub(crate) fn needs_ocr(self) -> bool {
        matches!(
            self,
            Self::NoTextLayer | Self::GarbageLayer | Self::ScanLayerBad | Self::InkOutsideLayer
        )
    }
}

/// Engine signals used when no render is available (report: no-render variant).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct EngineHints {
    pub scanned_no_layer: bool,
    pub image_area_ratio: f32,
    pub vector_path_count: usize,
}

/// Everything the cascade reads about one page.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PageSignals {
    pub standalone_image: bool,
    pub text: TextFeatures,
    /// Share of the page covered by images (the reference uses the largest image).
    pub image_cover: f32,
    /// `None` when the page was not rendered (gate or build without `ocr`).
    pub ink: Option<InkFeatures>,
    pub hints: EngineHints,
}

/// Render gate: whether the 50-dpi ink features are worth a render. A page with a text layer,
/// images over at most `render_min_image_cover` of it and fewer than `render_min_paths` painted
/// paths is decided on text alone.
#[cfg(feature = "ocr")]
pub(crate) fn needs_render(t: &TextFeatures, h: &EngineHints, c: &NeedOcrConfig) -> bool {
    t.nonws < c.min_chars
        || h.image_area_ratio > c.render_min_image_cover
        || h.vector_path_count >= c.render_min_paths
}

/// Rule cascade v2 (port of `phase2_v2.decide2`); the first matching rule wins.
///
/// `text.bigram_lp = None` (no bigram table loaded, or no letter pairs) behaves exactly as the
/// reference's `bigram_lp is None`: only the bigram clause of the prose-garbage rule is skipped;
/// its vowel-less-words clause and all other rules still apply.
pub(crate) fn decide(s: &PageSignals, c: &NeedOcrConfig) -> NeedReason {
    use NeedReason::*;
    if s.standalone_image {
        let ink = s.ink.unwrap_or_default();
        return if c.images_always_ocr
            || ink.ink_rows_outside >= c.g_img_rows
            || ink.glyph_cc_all >= c.g_img_cc
        {
            NoTextLayer
        } else {
            Blank
        };
    }
    let t = &s.text;
    if t.nonws < c.min_chars {
        let ink_says = match s.ink {
            Some(i) => i.glyph_cc_outside >= c.g_nolayer,
            None => {
                s.hints.scanned_no_layer
                    || s.hints.image_area_ratio > 0.3
                    || s.hints.vector_path_count >= 500
            }
        };
        return if ink_says { NoTextLayer } else { Blank };
    }
    let scan = s.image_cover >= c.scan_cover;
    if t.pua_share > c.pua {
        return GarbageLayer;
    }
    if scan && (t.letter_share < c.letter_scan || t.punct_share > c.punct_scan) {
        return ScanLayerBad;
    }
    let lettered = t.letter_share >= c.prose_letter_share;
    if t.n_alpha_words4 >= c.nw {
        let limit = if lettered {
            c.bigram
        } else {
            c.bigram_lowletter
        };
        if t.bigram_lp.is_some_and(|b| b < limit) || (lettered && t.novowel_share > c.novowel) {
            return GarbageLayer;
        }
    }
    if let Some(i) = s.ink
        && i.glyph_cc_outside >= c.g_abs
        && i.glyph_cc_outside as f32 >= c.g_ratio * t.nonws as f32
    {
        return InkOutsideLayer;
    }
    if scan { ScanLayerOk } else { TextOk }
}

/// Text the classifier reads: non-empty non-artifact regions of `page` joined by `\n` (SP2's text),
/// plus one U+FFFD per unmapped glyph the regions lost.
///
/// pdf_oxide's span/structured text drops glyphs that decode to U+FFFD (its global
/// `preserve_unmapped_glyphs` is off), while `extract_chars` and the PyMuPDF reference keep them.
/// Without them `pua_share` never sees U+FFFD and a layer of unmapped glyphs looks like clean prose
/// (task 14a, `rb_lezgi` p7 / p3: nonws 469 / 273 and `pua_share` 0 without them; with them nonws
/// 1006 / 466 and 537 / 193 U+FFFD, the PyMuPDF counts exactly).
///
/// Limits: the count covers the whole page, artifact regions included, and the glyphs are appended
/// as one trailing run, so letters on both sides of a dropped glyph still form one word and one
/// bigram chain (PyMuPDF splits them). `nonws` and the char shares are exact.
pub(crate) fn classifier_text(doc: &PdfDocument, p: usize, page: &StructuredPage) -> String {
    let mut text = page
        .regions
        .iter()
        .filter(|r| r.kind != RegionRole::Artifact && !r.text.trim().is_empty())
        .map(|r| r.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    const R: char = '\u{FFFD}';
    let in_chars = doc
        .extract_chars_shared(p)
        .map(|v| v.iter().filter(|c| c.char == R).count())
        .unwrap_or(0);
    let lost = in_chars.saturating_sub(text.chars().filter(|c| *c == R).count());
    if lost > 0 {
        text.push('\n');
        text.extend(std::iter::repeat_n(R, lost));
    }
    text
}

/// Classifier signals of page `p`, shared by the pipeline (stage A, classifier switched on) and
/// the gate harness [`eval_page`]: text = [`classifier_text`], hints and image cover from the
/// `classify_page` facts (`None` when classification failed), the 50-dpi ink only under feature
/// `ocr` and when [`needs_render`] says so.
pub(crate) fn page_signals(
    doc: &PdfDocument,
    p: usize,
    page: &StructuredPage,
    facts: Option<&super::diagnose::PageFacts>,
    big: Option<&Bigrams>,
    c: &NeedOcrConfig,
) -> PageSignals {
    let hints = facts.map(|f| f.hints()).unwrap_or_default();
    let text = TextFeatures::from_text(&classifier_text(doc, p, page), big);
    #[cfg(feature = "ocr")]
    let ink = needs_render(&text, &hints, c)
        .then(|| super::ocr_stage::ink_signals(doc, p))
        .flatten();
    #[cfg(not(feature = "ocr"))]
    let (ink, _) = (None, c);
    PageSignals {
        standalone_image: false,
        text,
        image_cover: hints.image_area_ratio.min(1.0),
        ink,
        hints,
    }
}

/// One page as the gate harness sees it.
#[cfg(feature = "ocr")]
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct EvalPage {
    pub needs_ocr: bool,
    /// `NeedReason` variant name.
    pub reason: String,
    /// `PageSignals` debug dump (miss analysis).
    pub signals: String,
}

/// Gate harness (task 14a): the need-OCR decision for page `p` of the PDF at `path`, from the
/// signals product code computes: opened as `PdfParser` opens it, text = [`classifier_text`],
/// `classify_page` hints and summed image area, the 50-dpi ink only when `needs_render` says so.
/// A raster image file (format sniffed from its bytes) is a standalone image with no ink, which
/// `RECALL` sends to OCR. Any other file without `%PDF` in its first 1 KiB is the error
/// `"unsupported file"`.
#[cfg(feature = "ocr")]
#[doc(hidden)]
pub fn eval_page(
    path: &std::path::Path,
    p: usize,
    big: Option<&Bigrams>,
    c: &NeedOcrConfig,
) -> Result<EvalPage, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let s = if matches!(
        plaindoc_core::detect(&bytes, None),
        Ok(plaindoc_core::Format::Pdf)
    ) {
        let o = super::open::open_input(
            plaindoc_core::Input::Bytes(bytes),
            &plaindoc_core::ParseOptions::default(),
        )
        .map_err(|e| e.to_string())?;
        // As `parse_opened` with the shipped `PdfConfig`.
        o.doc
            .set_char_x_offset_refinement(super::PdfConfig::default().char_x_refinement);
        let page = o.doc.extract_structured(p).map_err(|e| e.to_string())?;
        let cl = o.doc.classify_page(p).map_err(|e| e.to_string())?;
        let facts = super::diagnose::PageFacts::from(&cl);
        page_signals(&o.doc, p, &page, Some(&facts), big, c)
    } else if image::guess_format(&bytes).is_ok() {
        PageSignals {
            standalone_image: true,
            ..Default::default()
        }
    } else {
        return Err("unsupported file".into());
    };
    let d = decide(&s, c);
    Ok(EvalPage {
        needs_ocr: d.needs_ocr(),
        reason: format!("{d:?}"),
        signals: format!("{s:?}"),
    })
}

#[cfg(test)]
mod golden;

/// Hand-written cases for `GarbageLayer`, `InkOutsideLayer`, `ScanLayerOk` and `TextOk`, next to
/// the golden rows. Literals are synthetic.
#[cfg(test)]
mod tests {
    use super::*;

    /// Born-digital Russian prose page; `bigram_lp` is a made-up literal.
    fn prose(bigram_lp: Option<f32>) -> PageSignals {
        PageSignals {
            text: TextFeatures {
                nonws: 2000,
                n_alpha_words4: 200,
                letter_share: 0.9,
                punct_share: 0.05,
                bigram_lp,
                ..Default::default()
            },
            ink: Some(InkFeatures::default()),
            ..Default::default()
        }
    }

    fn both(s: &PageSignals) -> [NeedReason; 2] {
        [
            decide(s, &NeedOcrConfig::RECALL),
            decide(s, &NeedOcrConfig::BALANCED),
        ]
    }

    #[test]
    fn clean_prose_is_text_ok() {
        assert_eq!(both(&prose(Some(-1.0))), [NeedReason::TextOk; 2]);
    }

    #[test]
    fn missing_bigram_skips_only_the_bigram_clause() {
        assert_eq!(both(&prose(Some(-2.0))), [NeedReason::GarbageLayer; 2]);
        assert_eq!(both(&prose(None)), [NeedReason::TextOk; 2]);
        let mut s = prose(None);
        s.text.novowel_share = 0.4;
        assert_eq!(both(&s), [NeedReason::GarbageLayer; 2]);
    }

    #[test]
    fn bigram_limits_depend_on_letter_share_and_preset() {
        let mut s = prose(Some(-1.4));
        assert_eq!(
            both(&s),
            [NeedReason::GarbageLayer, NeedReason::TextOk],
            "-1.4 is below RECALL -1.35 but not BALANCED -1.45"
        );
        s.text.letter_share = 0.4;
        assert_eq!(both(&s), [NeedReason::TextOk; 2], "low-letter limit -1.9");
        s.text.bigram_lp = Some(-2.0);
        assert_eq!(both(&s), [NeedReason::GarbageLayer; 2]);
        s.text.n_alpha_words4 = 14;
        assert_eq!(both(&s), [NeedReason::TextOk; 2], "not prose below nw");
    }

    #[test]
    fn pua_layer_is_garbage_without_bigrams() {
        let mut s = prose(None);
        s.text.pua_share = 0.2;
        assert_eq!(both(&s), [NeedReason::GarbageLayer; 2]);
    }

    #[test]
    fn ink_outside_layer_and_scan_ok() {
        let mut s = prose(None);
        s.text.nonws = 100;
        s.ink = Some(InkFeatures {
            glyph_cc_outside: 100,
            ..Default::default()
        });
        assert_eq!(
            both(&s),
            [NeedReason::InkOutsideLayer, NeedReason::TextOk],
            "100 >= 0.5 * 100 but < 2.0 * 100"
        );
        s.ink = None;
        assert_eq!(both(&s), [NeedReason::TextOk; 2], "no render, no ink rule");
        s.image_cover = 0.9;
        assert_eq!(both(&s), [NeedReason::ScanLayerOk; 2]);
    }

    #[test]
    fn no_render_uses_engine_hints() {
        let mut s = PageSignals::default();
        assert_eq!(both(&s), [NeedReason::Blank; 2]);
        s.hints.scanned_no_layer = true;
        assert_eq!(both(&s), [NeedReason::NoTextLayer; 2]);
        s.standalone_image = true;
        assert_eq!(both(&s), [NeedReason::NoTextLayer; 2], "images_always_ocr");
    }

    #[cfg(feature = "ocr")]
    #[test]
    fn render_gate_skips_plain_text_pages() {
        let c = NeedOcrConfig::RECALL;
        let t = prose(None).text;
        let mut h = EngineHints::default();
        assert!(!needs_render(&t, &h, &c), "layer, no image, few paths");
        assert!(needs_render(&TextFeatures::default(), &h, &c), "no layer");
        h.vector_path_count = 200;
        assert!(needs_render(&t, &h, &c), "many paths");
        h = EngineHints {
            image_area_ratio: 0.01,
            ..Default::default()
        };
        assert!(!needs_render(&t, &h, &c), "small images only");
        h.image_area_ratio = 0.0101;
        assert!(needs_render(&t, &h, &c), "larger images");
    }

    /// One page, Helvetica with a /ToUnicode that maps "A" to U+FFFD, showing "AAAA BBBB".
    fn unmapped_glyph_pdf() -> Vec<u8> {
        let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
                    1 begincodespacerange <00> <FF> endcodespacerange \
                    1 beginbfchar <41> <FFFD> endbfchar endcmap CMapName currentdict /CMap \
                    defineresource pop end end";
        let content = "BT /F1 12 Tf 72 720 Td (AAAA BBBB) Tj ET";
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
                .to_string(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>".to_string(),
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ),
            format!("<< /Length {} >>\nstream\n{cmap}\nendstream", cmap.len()),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offs = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offs.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in offs {
            out.extend(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objs.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    #[test]
    fn unmapped_glyphs_reach_the_classifier_text() {
        use plaindoc_core::{Input, ParseOptions};
        let o = crate::pdf::open::open_input(
            Input::Bytes(unmapped_glyph_pdf()),
            &ParseOptions::default(),
        )
        .unwrap();
        let page = o.doc.extract_structured(0).unwrap();
        let t = classifier_text(&o.doc, 0, &page);
        assert!(t.contains("BBBB"), "{t:?}");
        assert_eq!(t.chars().filter(|c| *c == '\u{FFFD}').count(), 4, "{t:?}");
        let f = TextFeatures::from_text(&t, None);
        assert_eq!((f.nonws, f.pua_share), (8, 0.5));
    }

    #[test]
    fn default_is_recall() {
        assert_eq!(NeedOcrConfig::default(), NeedOcrConfig::RECALL);
    }
}
