//! Hidden text in the PDF text layer (a prompt-injection channel for LLM consumers), phase 1:
//! rules that need no change of the `pdf_oxide` fork.
//!
//! Reliable rules (removed in [`HiddenText::Strip`]): `offpage` (span outside the page box; the fork
//! already drops most of these), `tiny` (bbox height <= 1 pt and advance <= 1 pt per char; dvips
//! Type3 fonts use `Tf 0.24` with normal glyph advance), `uni_tags` (Unicode tag characters are
//! cut), `uni_bidi` (a span with a bidi override LRO/RLO; isolates LRI/RLI/FSI/PDI are legitimate
//! in RTL text and kept), `uni_zw` (a span with >= 4 single ASCII letters joined by zero-width
//! chars), `ocg_off` (optional content off by default: `Strip` reads the page without it,
//! [`ocg`](super::ocg)). Only flagged, also in `Strip`: `tiny` on scanned pages (OCR specks).
//! Phase 2, only with the fork's visibility facts (`PdfConfig::visibility_facts`, off by default):
//! removed in `Strip`: `alpha` (opacity <= 0.05), `clip` (<= 1 % of the span inside the clip);
//! only flagged, also in `Strip`: `invisible` (render mode 3/7 outside a recognised OCR layer;
//! vector-outlined glyphs with a Tr 3 search layer and per-word image patches with Tr 3 OCR text
//! are legitimate and not told apart), `overlap` (painted text under a later opaque rectangle or
//! image, except an image over >= 90 % of the page: a scan placed over its own OCR text, and a
//! paint under a clip that is not a single rectangle: the fork tracks a clip as the box of its
//! points, so a cell or banner frame filled through an even-odd clip ring would look like a
//! cover), `contrast` (fill-only text whose colour is within
//! [`MAX_CONTRAST_DIFF`] of the fill right under it, or of a white page; never removed: both
//! colours are the fork's resolved ones (Separation, DeviceN and Indexed through their tint
//! transform or lookup), an unknown one (a Pattern space, an unsupported tint function) gives no
//! judgement; a span merged from several runs carries its first run's colour, the stroke colour
//! of Tr 1/2 is not resolved, and the fork keeps the first copy of a duplicated run); the
//! `offpage` count then includes the spans the fork dropped.
//! Known false hits of the occluder and background model: a `Path` judged by the box of its points, a fill under a soft
//! mask (the fork marks fills unmasked) and a Multiply highlight (no blend mode on paints);
//! strokes, annotations and glyph boxes are not recorded as paints.
//! Phase 3 (the text layer against our OCR on pages OCRed whole): `hidden_ocr`.

use pdf_oxide::geometry::Rect;
use pdf_oxide::{StructuredPage, StructuredRegion};
use plaindoc_core::{HiddenText, Level, Warning, WarningKind};

use crate::tables::InputWord;

mod content;
mod facts;
mod words;

use content::is_tag;
#[cfg(test)]
use content::zw_interleave_run;
pub(crate) use content::{content_rule, non_ws};
pub(crate) use facts::{PageFacts, page_facts, placement_rule};
pub(crate) use words::glyph_box;
use words::{cut_removed, keep_word};

const TINY_MAX_HEIGHT: f32 = 1.0;
/// The fork derives span height from the font size, so dvips Type3 text (`Tf 0.24`, normal glyphs)
/// has a 0.24 pt box: tiny text must also be narrow (measured: attacks 0.22–0.45 pt per char,
/// dvips 3.6–7.3). Known open: `Tc`/`Tz` spacing widens a 0.5 pt span past this bound.
const TINY_MAX_ADVANCE: f32 = 1.0;
const OFFPAGE_TOL: f32 = 2.0;
/// Single ASCII letters joined by zero-width characters that make a span an interleave payload.
const MIN_ZW_INTERLEAVE: usize = 4;
/// A page whose images cover at least this share counts as a scan ([`PageFacts::scan`]).
pub(crate) const SCAN_IMAGE_SHARE: f32 = 0.5;
/// An image covering at least this share of the page may carry an OCR layer (`invisible`).
const OCR_IMAGE_SHARE: f32 = 0.25;
/// Text painted with at most this opacity (`ca`/`CA`) is not visible (`alpha`).
const MAX_HIDDEN_ALPHA: f32 = 0.05;
/// Text with at most this share of its box inside the clip is not visible (`clip`).
const MAX_HIDDEN_CLIP: f32 = 0.01;
/// A later paint covering at least this share of a span's box hides it (`overlap`).
const MIN_COVERED_SHARE: f32 = 0.9;
/// A paint with at least this opacity (`ca`) hides what is under it.
const OPAQUE_ALPHA: f32 = 0.95;
/// An image over at least this share of the page painted over text is a scan over its OCR text
/// (ABBYY and similar), not an `overlap`.
const FULL_PAGE_IMAGE_SHARE: f32 = 0.9;
/// Text whose colour differs from its background by at most this in every RGB channel is not
/// visible (`contrast`; attacks use 0.97 grey on white, light grey labels 0.85).
const MAX_CONTRAST_DIFF: f32 = 0.1;

/// Rule codes, in the order they are reported (also `Warning::code`).
pub(crate) const RULES: [&str; 11] = [
    "offpage",
    "tiny",
    "ocg_off",
    "uni_tags",
    "uni_bidi",
    "uni_zw",
    "invisible",
    "alpha",
    "clip",
    "overlap",
    "contrast",
];
const OFFPAGE: usize = 0;
const TINY: usize = 1;
pub(super) const OCG_OFF: usize = 2;
const UNI_TAGS: usize = 3;
const UNI_BIDI: usize = 4;
const UNI_ZW: usize = 5;
/// Phase 2, visibility facts: render mode 3/7 outside an OCR layer. Only flagged, also in `Strip`:
/// legitimate search layers over vector glyphs or small image patches look the same.
const INVISIBLE: usize = 6;
/// Phase 2: fill (and for stroke modes stroke) opacity <= [`MAX_HIDDEN_ALPHA`].
const ALPHA: usize = 7;
/// Phase 2: the span lies outside the clip (Tr 0–2; the fork does not narrow text clips, Tr 4–7).
const CLIP: usize = 8;
/// Phase 2: painted text (Tr 0–2, 4–6) under a later opaque `Rect`/`Image` covering
/// >= [`MIN_COVERED_SHARE`] of its box. Only flagged, also in `Strip`, until gated on corpora.
const OVERLAP: usize = 9;
/// Phase 2: fill-only text (Tr 0/4) in about the colour of the fill under it (or a white page).
/// Only flagged, also in `Strip`: the colour of a merged span is its first run's, and text or a
/// fill whose colour the fork cannot resolve (`None`) is not judged.
const CONTRAST: usize = 10;

/// Per-page counts of hidden characters by rule (index into [`RULES`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HiddenCounts {
    pub chars: [usize; RULES.len()],
    pub spans: [usize; RULES.len()],
    /// The rule's text was removed on this page (`Strip`); otherwise it was only flagged.
    pub removed: [bool; RULES.len()],
    /// `Strip` on a scan page: `tiny` text kept there is OCR-layer specks, reported as `Info`.
    pub scan_ocr: bool,
    /// The page was read with or without the default-off layers only (`ocg`): not checked.
    pub layers_unchecked: bool,
}

impl HiddenCounts {
    fn add(&mut self, rule: usize, chars: usize, removed: bool) {
        self.chars[rule] += chars;
        self.spans[rule] += 1;
        self.removed[rule] = removed;
    }

    /// Combines the counts of two extractions of the same page (spans and table words): per rule
    /// the larger count, removed if either removed it.
    pub(crate) fn merge_max(&mut self, other: &HiddenCounts) {
        for i in 0..RULES.len() {
            self.chars[i] = self.chars[i].max(other.chars[i]);
            self.spans[i] = self.spans[i].max(other.spans[i]);
            self.removed[i] |= other.removed[i];
        }
        self.scan_ocr |= other.scan_ocr;
        self.layers_unchecked |= other.layers_unchecked;
    }

    /// One warning per (page, rule) with `code` = the rule: `Info` for removed text and for `tiny`
    /// kept on a scan in `Strip` (no quality penalty), `Warn` for text that was only flagged.
    pub(crate) fn warnings(&self) -> Vec<Warning> {
        let unchecked = self.layers_unchecked.then(|| {
            let what = "default-off layers not checked: a page extraction failed";
            let mut w = Warning::new(WarningKind::HiddenContent, Level::Warn, what);
            w.code = Some(RULES[OCG_OFF]);
            w
        });
        (0..RULES.len())
            .filter(|&i| self.spans[i] > 0)
            .map(|i| {
                let (level, what) = if self.removed[i] {
                    (Level::Info, "hidden text removed")
                } else if i == TINY && self.scan_ocr {
                    (Level::Info, "hidden text kept on a scan")
                } else {
                    (Level::Warn, "possibly hidden text kept")
                };
                let mut w = Warning::new(
                    WarningKind::HiddenContent,
                    level,
                    format!(
                        "{what}: {}={} chars ({} spans)",
                        RULES[i], self.chars[i], self.spans[i]
                    ),
                );
                w.code = Some(RULES[i]);
                w
            })
            .chain(unchecked)
            .collect()
    }
}

/// [`filter_page_boxes`] without the removed boxes.
#[cfg(test)]
pub(crate) fn filter_page(
    page: &mut StructuredPage,
    mode: HiddenText,
    facts: &PageFacts,
) -> HiddenCounts {
    filter_page_boxes(page, mode, facts, &mut Vec::new())
}

/// Removes (`Strip`) or only counts (`Flag`) hidden text on `page` in place (`ocg_off` is read by
/// `ocg`, not here). `invisible` (search layers
/// that are not recognised as OCR), `overlap` (not gated for removal yet), `contrast` (a merged
/// span's colour is its first run's; unresolved colours are not judged) and `tiny` on scans are
/// only counted in `Strip` as well. The boxes (PDF coordinates) and texts of the removed spans
/// are pushed to `removed`, for [`filter_words`].
pub(crate) fn filter_page_boxes(
    page: &mut StructuredPage,
    mode: HiddenText,
    facts: &PageFacts,
    removed: &mut Vec<(Rect, String)>,
) -> HiddenCounts {
    let mut counts = HiddenCounts::default();
    if mode == HiddenText::Off {
        return counts;
    }
    let strip = mode == HiddenText::Strip;
    counts.scan_ocr = strip && facts.scan;
    let removes = |rule: usize| removes(rule, strip, facts);
    let mut changed = false;
    for r in &mut page.regions {
        let before = r.spans.len();
        let mut touched = false;
        r.spans.retain_mut(|s| {
            if s.text.trim().is_empty() {
                return true;
            }
            if let Some(rule) = placement_rule(s, facts) {
                counts.add(rule, non_ws(&s.text), removes(rule));
                if removes(rule) {
                    removed.push((glyph_box(s), s.text.clone()));
                    return false;
                }
            }
            if let Some(rule) = content_rule(&s.text) {
                counts.add(rule, non_ws(&s.text), strip);
                if strip {
                    removed.push((glyph_box(s), s.text.clone()));
                    return false;
                }
            }
            let tags = s.text.chars().filter(|&c| is_tag(c)).count();
            if tags > 0 {
                counts.add(UNI_TAGS, tags, strip);
                if strip {
                    s.text = s.text.chars().filter(|&c| !is_tag(c)).collect();
                    touched = true;
                }
            }
            true
        });
        if touched || r.spans.len() != before {
            rebuild(r);
            changed = true;
        }
    }
    // The fork drops off-page spans in every mode: they are reported as removed, also in `Flag`.
    if facts.offpage.0 > 0 {
        counts.chars[OFFPAGE] += facts.offpage.1;
        counts.spans[OFFPAGE] += facts.offpage.0;
        counts.removed[OFFPAGE] = true;
    }
    if changed {
        page.regions
            .retain(|r| !r.spans.is_empty() && !r.text.trim().is_empty());
    }
    counts
}

/// `Strip` removes the text of `rule`; `invisible`, `overlap`, `contrast` and `tiny` on a scan are
/// only counted.
fn removes(rule: usize, strip: bool, facts: &PageFacts) -> bool {
    strip && !matches!(rule, INVISIBLE | OVERLAP | CONTRAST) && !(rule == TINY && facts.scan)
}

/// [`filter_page_boxes`] for the table detector's words (`extract_words`, page-local top-left
/// coordinates; `origin` = `table_input::page_origin`). First the glyphs of the spans the span
/// pass removed (`facts.removed`, any rule, also the phase-2 `alpha`/`clip` with visibility
/// facts on, and the default-off layer spans of `ocg::ocg_off`: `extract_words` has no layer
/// filter), matched by position and text ([`cut_removed`]): a payload glued to a visible word,
/// the other words of a removed multi-word span, a payload in a cell. These glyphs are already
/// counted by the span pass and are not counted again. Then, as a fallback, the per-word rules: `tiny` by the word box (only
/// counted on a scan) and `uni_tags`/`uni_bidi`/`uni_zw` by its text, with the same removal
/// rules. Not applied to words (open):
/// - the flag-only phase-2 rules (`invisible`, `overlap`, `contrast`): words carry no
///   visibility facts, and the span pass flags the page.
///
/// The words are another extraction of the same glyphs as the spans, so the page reports the
/// larger of the two counts per rule ([`HiddenCounts::merge_max`]); the counts here are in words.
pub(crate) fn filter_words(
    words: &mut Vec<InputWord>,
    mode: HiddenText,
    facts: &PageFacts,
    origin: (f64, f64),
) -> HiddenCounts {
    let mut counts = HiddenCounts::default();
    if mode == HiddenText::Off {
        return counts;
    }
    let strip = mode == HiddenText::Strip;
    counts.scan_ocr = strip && facts.scan;
    cut_removed(words, &facts.removed, origin);
    words.retain_mut(|w| keep_word(w, strip, facts, &mut counts));
    counts
}

/// Region text and box from its spans, joined like the fork (trimmed span texts, single spaces)
/// less the spaces between touching spans at punctuation ([`super::spacing::tighten`]).
fn rebuild(r: &mut StructuredRegion) {
    r.text = r
        .spans
        .iter()
        .map(|s| s.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    super::spacing::tighten(r);
    if let Some(first) = r.spans.first() {
        r.bbox = r
            .spans
            .iter()
            .skip(1)
            .fold(first.bbox, |b, s| b.union(&s.bbox));
    }
}

#[cfg(test)]
#[path = "../hidden_test.rs"]
mod tests;
