//! OCR of images embedded in office sections that have no text (`OfficeParser::with_ocr`).
//! Bytes come from the IR, are decoded within the configured [`ImageLimits`] (first frame only) and
//! recognised one at a time; a bad image degrades its section to the plain `ImageTextPage` warning.

use std::time::{Duration, Instant};

use office_oxide::ir::{DocumentIR, Element, Section as IrSection};
use plaindoc_core::{
    Block, Document, Format, Level, OcrMode, Section, SectionKind, TextSource, Warning, WarningKind,
};
use plaindoc_ocr::{OcrError, OcrPage, ocr_image};

use super::diag::text_chars;
use crate::ocr::images::{fit_for_ocr, for_each_page};
use crate::ocr::{OcrConfig, ocr_warnings};

/// Images of one section sent to OCR; the rest give `PartialContent`.
pub(crate) const MAX_REGIONS: usize = 16;

/// OCR settings of one `parse` call.
#[derive(Clone)]
pub(crate) struct OcrRun {
    pub cfg: OcrConfig,
    pub mode: OcrMode,
    /// Parse start + `ParseOptions::timeout`; checked between images.
    pub deadline: Option<Instant>,
    /// `ParseOptions::ocr_max_pages`: images OCRed per document.
    pub max_images: Option<usize>,
    /// `ParseOptions::ocr_page_timeout`: recognition budget per image.
    pub page_timeout: Option<Duration>,
}

impl OcrRun {
    /// OCRs the images of `doc`'s sections. At most [`MAX_REGIONS`] per section and
    /// `limits.max_frames` (and `max_images`) per document.
    pub fn apply(&self, doc: &mut Document, ir: &DocumentIR) {
        if self.mode == OcrMode::Never {
            return;
        }
        let Some(engine) = self.cfg.engine.as_deref() else {
            let wanted = doc.sections.iter().enumerate().any(|(i, s)| {
                if self.mode == OcrMode::Always {
                    top_images(source(ir, doc, i)).iter().any(Option::is_some)
                } else {
                    wants_ocr_without_engine(s)
                }
            });
            if wanted {
                self.unavailable(doc, "no OCR engine in this build", false);
            }
            return;
        };
        let mut budget = self
            .cfg
            .limits
            .max_frames
            .min(self.max_images.unwrap_or(usize::MAX));
        let mut over_cap = 0usize; // supported images left out by an exhausted budget
        let mut stop: Option<String> = None;
        for i in 0..doc.sections.len() {
            if stop.is_some() {
                break;
            }
            let s = &doc.sections[i];
            if matches!(self.mode, OcrMode::Auto | OcrMode::Images) && !wants_ocr_without_engine(s)
            {
                continue;
            }
            let imgs = top_images(source(ir, doc, i));
            let s = &mut doc.sections[i];
            let slots = s
                .blocks
                .iter()
                .enumerate()
                .filter(|(_, b)| matches!(b, Block::Image { .. }))
                .map(|(i, _)| i)
                .zip(imgs);
            let mut pages: Vec<(usize, OcrPage)> = Vec::new();
            let (mut tried, mut skipped) = (0usize, 0usize);
            for (bi, bytes) in slots {
                let Some(bytes) = bytes else { continue };
                let Ok(format) = plaindoc_core::detect(
                    &bytes[..bytes.len().min(plaindoc_core::DETECT_HEAD_LEN)],
                    None,
                ) else {
                    skipped += 1;
                    continue;
                };
                let supported = matches!(
                    format,
                    Format::Png | Format::Jpeg | Format::Tiff | Format::Bmp | Format::Webp
                );
                if supported && budget == 0 {
                    over_cap += 1;
                }
                if !supported
                    || tried >= MAX_REGIONS
                    || budget == 0
                    || self.deadline.is_some_and(|d| Instant::now() >= d)
                {
                    skipped += 1;
                    continue;
                }
                budget -= 1;
                tried += 1;
                let mut result = None;
                let decoded = for_each_page(bytes, format, &self.cfg.limits, 1, |_, img| {
                    let (img, _) = fit_for_ocr(img, self.cfg.limits.ocr_max_side);
                    let mut params = self.cfg.params;
                    let page = self
                        .page_timeout
                        .and_then(|t| Instant::now().checked_add(t));
                    params.deadline = match (page, self.deadline) {
                        (Some(p), Some(j)) => Some(p.min(j)),
                        (p, j) => p.or(j),
                    };
                    result = Some(ocr_image(engine, &img, &params));
                    Ok(())
                });
                match (decoded, result) {
                    (Ok(_), Some(Ok(page))) => pages.push((bi, page)),
                    (Ok(_), Some(Err(OcrError::ModelsUnavailable(m)))) => {
                        stop = Some(m);
                        break;
                    }
                    // Limit/Corrupt/Internal of one image, or an engine failure: this image only.
                    _ => skipped += 1,
                }
            }
            finish_section(s, pages, skipped, self.cfg.low_confidence);
        }
        if let Some(m) = stop {
            self.unavailable(doc, &m, true);
        }
        if let Some(m) = self.max_images.filter(|_| over_cap > 0) {
            let n = m.min(self.cfg.limits.max_frames);
            doc.warnings.push(Warning::new(
                WarningKind::PartialContent,
                Level::Warn,
                format!(
                    "office: OCR limited to {n} of {} images that need it",
                    n + over_cap
                ),
            ));
        }
    }

    /// `Warn` when the caller demanded OCR (`Always`) or the engine's models are missing (a broken
    /// setup), `Info` when OCR was only opportunistic.
    fn unavailable(&self, doc: &mut Document, why: &str, models_missing: bool) {
        let level = if self.mode == OcrMode::Always || models_missing {
            Level::Warn
        } else {
            Level::Info
        };
        doc.warnings.push(Warning::new(
            WarningKind::OcrUnavailable,
            level,
            format!("office: image text not extracted: {why}"),
        ));
    }
}

fn wants_ocr_without_engine(s: &Section) -> bool {
    s.warnings
        .iter()
        .any(|w| w.kind == WarningKind::ImageTextPage)
}

/// IR section behind `doc.sections[i]`: sections are mapped in order, only the trailing slide
/// master may follow skipped (`max_pages`) ones.
fn source<'a>(ir: &'a DocumentIR, doc: &Document, i: usize) -> &'a IrSection {
    let master = ir.sections.last().filter(|m| {
        doc.sections[i].kind == SectionKind::Slide
            && i + 1 == doc.sections.len()
            && doc.sections.len() < ir.sections.len()
            && m.title.as_deref() == Some(office_oxide::pptx::layout::MASTER_TEXT_SECTION_TITLE)
    });
    master.unwrap_or(&ir.sections[i])
}

/// Bytes of the top-level images of a section (nested in text boxes too), in block order; nothing
/// is copied.
fn top_images(sec: &IrSection) -> Vec<Option<&[u8]>> {
    fn walk<'a>(els: &'a [Element], out: &mut Vec<Option<&'a [u8]>>) {
        for e in els {
            match e {
                Element::Image(i) => out.push(i.data.as_deref()),
                Element::TextBox(tb) => walk(&tb.content, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&sec.elements, &mut out);
    out
}

fn finish_section(s: &mut Section, pages: Vec<(usize, OcrPage)>, skipped: usize, low: f32) {
    let was_textless = text_chars(&s.blocks) == 0;
    let scope = format!("section {} images", s.index + 1);
    let mut found = false;
    for (bi, page) in &pages {
        let text = page.text();
        if let Block::Image { ocr_text, .. } = &mut s.blocks[*bi]
            && !text.trim().is_empty()
        {
            *ocr_text = Some(text);
            found = true;
        }
    }
    if !pages.is_empty() {
        let refs: Vec<&OcrPage> = pages.iter().map(|(_, p)| p).collect();
        let (w, q) = ocr_warnings(&refs, &scope, low);
        s.warnings.extend(w);
        if found && was_textless {
            s.source = TextSource::Ocr;
            s.quality = q;
        }
    }
    if found
        && let Some(w) = s
            .warnings
            .iter_mut()
            .find(|w| w.kind == WarningKind::ImageTextPage)
    {
        w.level = Level::Info;
        w.message = "office: section has images but no text; text recognised by OCR".into();
    }
    let timed_out = pages.iter().filter(|(_, p)| p.timed_out).count();
    if timed_out > 0 {
        s.warnings.push(Warning::new(
            WarningKind::PartialContent,
            Level::Warn,
            format!(
                "office: OCR of {timed_out} image(s) of section {} timed out",
                s.index + 1
            ),
        ));
    }
    if skipped > 0 {
        s.warnings.push(Warning::new(
            WarningKind::PartialContent,
            Level::Warn,
            format!(
                "office: {skipped} image(s) of section {} not OCRed (unsupported, damaged, over limits or past the {MAX_REGIONS}-per-section cap)",
                s.index + 1
            ),
        ));
    }
}
