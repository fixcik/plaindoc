//! Optional content off by default (`ocg_off`, SP8 P2-1): the page read with and without the
//! layers the document's default configuration hides.
//!
//! The fork decides groups by reference ([`LayerFilter::document_default`]: duplicate or missing
//! `/Name`, `/BaseState /OFF` with `/ON`/`/OFF`, OCMDs and `/VE` over groups that are on, a Form
//! XObject drawn hidden then visible), and on a document with optional content it reads large
//! streams with one parser in both extractions, so visible glyphs keep the same boxes. Span
//! assembly still differs: in the whole page a hidden run can be merged into the visible span
//! next to it, or win a duplicate-overlay dedup against it. So the spans of the whole page missing
//! from the filtered one are not purely hidden text: [`ocg_off`] guards the table-word cuts.
//!
//! Policy by [`HiddenText`] mode: `Strip` uses the page without the layers where it differs from
//! the whole page (`Info`), `Flag` the whole page and reports the difference (`Warn`), `Off`
//! reads the whole page and computes nothing. Messages carry counts, never the text.
//!
//! Extraction errors fail open with a `Warn` (SP8 P2-1 review I-3): when the filtered or the
//! whole extraction fails on a document with optional content, the page that was read is used
//! (an empty page would drop visible text) and the page reports that its layers were not checked.

use std::collections::{HashMap, HashSet};

use pdf_oxide::geometry::Rect;
use pdf_oxide::layout::TextSpan;
use pdf_oxide::optional_content::LayerFilter;
use pdf_oxide::{ColumnMode, PdfDocument, StructuredPage};
use plaindoc_core::HiddenText;

use super::columns::false_gutter;
use super::hidden::{HiddenCounts, OCG_OFF, glyph_box, non_ws};

/// The layer filter for stage A, once per document: `None` in `Off` and without optional content
/// (almost every file), then stage A reads pages exactly as without layers.
pub(super) fn layer_filter(doc: &PdfDocument, mode: HiddenText) -> Option<LayerFilter> {
    (mode != HiddenText::Off)
        .then(|| LayerFilter::document_default(doc))
        .filter(|f| !f.is_empty())
}

/// `ocg_off` of one page, before the span rules: counts and, in `Strip`, the boxes and texts of
/// the default-off spans for the table words (`hidden::filter_words`).
#[derive(Debug, Default)]
pub(super) struct OcgOff {
    pub counts: HiddenCounts,
    pub removed: Vec<(Rect, String)>,
}

impl OcgOff {
    fn unchecked() -> Self {
        let mut o = Self::default();
        o.counts.layers_unchecked = true;
        o
    }
}

/// The page of stage A: `extract_structured`, read as one column when the fork's gutter is false
/// ([`false_gutter`]). With a layer filter the page is also read without the layers (first, so
/// the used page is the last extraction: the fork's `page_visibility` is that of the last one).
/// Where the two raw pages hold the same spans (text and box), the whole page is used as without
/// layers. Otherwise `Strip` reads the filtered page again with the same column gate and uses it;
/// `Flag` uses the whole page.
pub(super) fn extract_page(
    doc: &PdfDocument,
    p: usize,
    layers: Option<&LayerFilter>,
    mode: HiddenText,
) -> (pdf_oxide::Result<StructuredPage>, OcgOff) {
    // a gutter inside a two-column table: read the page as one column
    let column = |s: &StructuredPage| {
        if false_gutter(s) {
            ColumnMode::Single
        } else {
            ColumnMode::Auto
        }
    };
    let gate = |s: StructuredPage| match column(&s) {
        ColumnMode::Auto => Ok(s),
        m => doc.extract_structured_with_column_mode(p, m),
    };
    let Some(f) = layers else {
        return (doc.extract_structured(p).and_then(gate), OcgOff::default());
    };
    let shown = doc.extract_structured_with_layer_filter(p, ColumnMode::Auto, f);
    let full = doc.extract_structured(p);
    let filtered = |s: &StructuredPage| doc.extract_structured_with_layer_filter(p, column(s), f);
    choose(full, shown, mode == HiddenText::Strip, gate, filtered)
}

/// [`extract_page`] once both raw pages are read: `gate` reads the whole page with the column
/// gate, `filtered` the page without the layers again with the gate of `shown`.
fn choose(
    full: pdf_oxide::Result<StructuredPage>,
    shown: pdf_oxide::Result<StructuredPage>,
    strip: bool,
    gate: impl FnOnce(StructuredPage) -> pdf_oxide::Result<StructuredPage>,
    filtered: impl FnOnce(&StructuredPage) -> pdf_oxide::Result<StructuredPage>,
) -> (pdf_oxide::Result<StructuredPage>, OcgOff) {
    match (full, shown) {
        (Ok(full), Ok(shown)) if same_spans(&full, &shown) => (gate(full), OcgOff::default()),
        (Ok(full), Ok(shown)) if strip => (filtered(&shown), ocg_off(&full, &shown, true)),
        (Ok(full), Ok(shown)) => {
            let off = ocg_off(&full, &shown, false);
            (gate(full), off)
        }
        (Ok(full), Err(_)) => (gate(full), OcgOff::unchecked()),
        (Err(_), Ok(shown)) if strip => (filtered(&shown), OcgOff::unchecked()),
        (Err(e), _) => (Err(e), OcgOff::default()),
    }
}

fn spans(p: &StructuredPage) -> impl Iterator<Item = &TextSpan> {
    p.regions.iter().flat_map(|r| r.spans.iter())
}

type SpanKey = (String, [u32; 4]);

fn key(s: &TextSpan) -> SpanKey {
    let b = s.bbox;
    (
        s.text.clone(),
        [b.x, b.y, b.width, b.height].map(f32::to_bits),
    )
}

/// The spans of `full` missing from `shown` (same text and box bits, as a multiset), and the
/// number of spans of `shown` missing from `full`.
fn unmatched<'a>(full: &'a StructuredPage, shown: &StructuredPage) -> (Vec<&'a TextSpan>, usize) {
    let mut left: HashMap<SpanKey, usize> = HashMap::new();
    for s in spans(shown) {
        *left.entry(key(s)).or_default() += 1;
    }
    let off = spans(full)
        .filter(|s| match left.get_mut(&key(s)) {
            Some(k) if *k > 0 => {
                *k -= 1;
                false
            }
            _ => true,
        })
        .collect();
    (off, left.values().sum())
}

fn same_spans(full: &StructuredPage, shown: &StructuredPage) -> bool {
    let (off, extra) = unmatched(full, shown);
    off.is_empty() && extra == 0
}

/// Shown spans taller than this are checked against every default-off span; the others are found
/// by their bottom, at most this far below the span's.
const TALL_SPAN: f32 = 24.0;

/// `ocg_off` from two raw pages that differ: `full` (`extract_structured`) and `shown` (without
/// the default-off layers). Counted by page: the non-whitespace chars of `full` minus those of
/// `shown`, or, when that is 0 (a same-length substitution: the whole page kept a hidden
/// duplicate of a visible run), the chars of the `full` spans missing from `shown`; at least one
/// span. `Strip` reports it removed (`Info`), `Flag` kept (`Warn`).
///
/// In `Strip` the spans of `full` missing from `shown` go to `removed` for the table words
/// (`extract_words` reads without a layer filter), less the tokens of `shown` spans whose boxes
/// overlap them (a visible run the fork merged with a default-off one). Consistency check: when
/// the remaining chars exceed the page count, the extractions disagree beyond the hidden text (a
/// payload glued to a visible token, dedup and a payload together): no word is cut, so no visible
/// cell text is lost, and the page reports `Warn` (hidden text may stay in a table cell).
pub(crate) fn ocg_off(full: &StructuredPage, shown: &StructuredPage, strip: bool) -> OcgOff {
    let (off, _) = unmatched(full, shown);
    let off: Vec<&TextSpan> = off
        .into_iter()
        .filter(|s| !s.text.trim().is_empty())
        .collect();
    let chars = |p| spans(p).map(|s| non_ws(&s.text)).sum::<usize>();
    let n = match chars(full).saturating_sub(chars(shown)) {
        0 => off.iter().map(|s| non_ws(&s.text)).sum(),
        n => n,
    };
    let mut o = OcgOff::default();
    o.counts.chars[OCG_OFF] = n;
    o.counts.spans[OCG_OFF] = off.len().max(1);
    o.counts.removed[OCG_OFF] = strip;
    if !strip {
        return o;
    }
    o.removed = guarded(&off, shown).0;
    if o.removed.iter().map(|(_, t)| non_ws(t)).sum::<usize>() > n {
        o.removed.clear();
        o.counts.removed[OCG_OFF] = false;
    }
    o
}

/// The boxes and texts of the default-off spans `off`, less the tokens of the `shown` spans whose
/// boxes intersect them, and the number of (default-off span, shown span) pairs looked at (a
/// y-band index: work <= off spans x (spans of their line + tall spans)).
fn guarded(off: &[&TextSpan], shown: &StructuredPage) -> (Vec<(Rect, String)>, usize) {
    // shown spans by the bottom of their box; the tall ones apart
    let y_range = |b: &Rect| (b.y.min(b.y + b.height), b.y.max(b.y + b.height));
    type ByBottom<'a> = Vec<(f32, &'a TextSpan)>;
    let (mut short, tall): (ByBottom, ByBottom) = spans(shown)
        .filter(|v| {
            [v.bbox.x, v.bbox.y, v.bbox.width, v.bbox.height]
                .iter()
                .all(|x| x.is_finite())
        })
        .map(|v| (y_range(&v.bbox).0, v))
        .partition(|(_, v)| v.bbox.height.abs() <= TALL_SPAN);
    short.sort_by(|a, b| a.0.total_cmp(&b.0));
    let reach = short
        .iter()
        .map(|(_, v)| v.bbox.height.abs())
        .fold(0.0, f32::max);
    let (mut out, mut work) = (Vec::new(), 0);
    for s in off {
        let b = glyph_box(s);
        let (lo, hi) = y_range(&b);
        let from = short.partition_point(|(y, _)| *y < lo - reach);
        let to = short.partition_point(|(y, _)| *y <= hi).max(from);
        let mut near: HashSet<&str> = HashSet::new();
        for (_, v) in short[from..to].iter().chain(&tall) {
            work += 1;
            if v.bbox.intersects(&b) {
                near.extend(v.text.split_whitespace());
            }
        }
        let text: Vec<&str> = s
            .text
            .split_whitespace()
            .filter(|t| !near.contains(t))
            .collect();
        if !text.is_empty() {
            out.push((b, text.join(" ")));
        }
    }
    (out, work)
}

#[cfg(test)]
#[path = "ocg_test.rs"]
mod tests;
