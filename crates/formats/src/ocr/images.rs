//! Standalone images (png, jpeg, tiff incl. multi-page, bmp, webp): decode, apply EXIF
//! orientation, OCR. Frames are decoded, recognised and freed one at a time (a 6 MB multi-page
//! TIFF can hold gigabytes of pixels), within explicit pixel and frame limits.

use std::io::Cursor;
use std::time::Instant;

use image::ImageDecoder;
use plaindoc_core::{
    Block, Document, Format, Input, Level, OcrMode, ParseError, ParseOptions, Parser, Result,
    Section, SectionKind, TextSource, Warning, WarningKind,
};
use plaindoc_ocr::{Image, OcrError, ocr_image};

use super::{OcrConfig, PageGeom, ocr_blocks, ocr_warnings, rgba_to_rgb};

/// Decoding limits for hostile images (service use: one file must not exhaust memory).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageLimits {
    /// Maximum width or height of one frame, px.
    pub max_side: u32,
    /// Maximum decoder allocation for one frame, bytes.
    pub max_alloc: u64,
    /// Frames of a multi-page TIFF looked at; the rest give `PartialContent`.
    pub max_frames: usize,
    /// Frames are downscaled to this long side before OCR (det works at 960 anyway).
    pub ocr_max_side: u32,
}

impl Default for ImageLimits {
    fn default() -> Self {
        Self {
            max_side: 12_000,
            max_alloc: 256 << 20,
            max_frames: 1000,
            ocr_max_side: 4000,
        }
    }
}

fn corrupt(e: impl std::fmt::Display) -> ParseError {
    ParseError::Corrupt(e.to_string())
}

/// The RGB8 frame (3 bytes per pixel, what OCR works on) must fit `max_alloc`: decoder limits only
/// bound the decoder's own buffer, the RGB expansion comes after it (x3 from gray, x24 from 1 bit).
fn check_pixels(w: u32, h: u32, l: &ImageLimits) -> Result<()> {
    if u64::from(w) * u64::from(h) * 3 > l.max_alloc {
        return Err(ParseError::Limit {
            kind: "image pixels".into(),
            limit: l.max_alloc,
        });
    }
    Ok(())
}

fn image_err(e: image::ImageError, l: &ImageLimits) -> ParseError {
    match e {
        image::ImageError::Limits(_) => ParseError::Limit {
            kind: "image size".into(),
            limit: u64::from(l.max_side),
        },
        e => corrupt(e),
    }
}

/// Decodes up to `take` frames, calling `f(index, frame)` for each before decoding the next.
/// Returns the number of frames in the file (counted up to `max_frames + 1` without decoding).
pub(crate) fn for_each_page(
    bytes: &[u8],
    format: Format,
    l: &ImageLimits,
    take: usize,
    mut f: impl FnMut(usize, Image) -> Result<()>,
) -> Result<usize> {
    if format == Format::Tiff {
        return tiff_pages(bytes, l, take, f);
    }
    let img = no_panic(|| {
        let mut reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(corrupt)?;
        let mut lim = image::Limits::default();
        lim.max_image_width = Some(l.max_side);
        lim.max_image_height = Some(l.max_side);
        lim.max_alloc = Some(l.max_alloc);
        reader.limits(lim);
        let mut dec = reader.into_decoder().map_err(|e| image_err(e, l))?;
        let orientation = dec.orientation().map_err(corrupt)?;
        let (w, h) = dec.dimensions();
        check_pixels(w, h, l)?;
        if take == 0 {
            return Ok(None);
        }
        let mut img = image::DynamicImage::from_decoder(dec).map_err(|e| image_err(e, l))?;
        img.apply_orientation(orientation);
        Ok(Some(img.into_rgb8()))
    })?;
    if let Some(img) = img {
        f(0, img)?;
    }
    Ok(1)
}

/// Decoders run on hostile bytes: a panic inside one becomes `Internal` (В-16) instead of
/// unwinding into the caller (the OCR callback `f` runs outside, its panics are not masked).
fn no_panic<T>(decode: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(decode))
        .unwrap_or_else(|_| Err(ParseError::Internal("image decoder panicked".into())))
}

/// All frames, for tests and small inputs.
#[cfg(test)]
pub(crate) fn decode_pages(bytes: &[u8], format: Format) -> Result<Vec<Image>> {
    let mut out = Vec::new();
    for_each_page(
        bytes,
        format,
        &ImageLimits::default(),
        usize::MAX,
        |_, img| {
            out.push(img);
            Ok(())
        },
    )?;
    Ok(out)
}

fn tiff_pages(
    bytes: &[u8],
    l: &ImageLimits,
    take: usize,
    mut f: impl FnMut(usize, Image) -> Result<()>,
) -> Result<usize> {
    use tiff::ColorType as C;
    use tiff::decoder::{Decoder, DecodingResult, Limits};
    let mut tl = Limits::default();
    tl.decoding_buffer_size = l.max_alloc as usize;
    let mut d = no_panic(|| {
        Ok(Decoder::new(Cursor::new(bytes))
            .map_err(corrupt)?
            .with_limits(tl))
    })?;
    let mut i = 0;
    loop {
        if i < take {
            let img = no_panic(|| {
                let (w, h) = d.dimensions().map_err(corrupt)?;
                if w.max(h) > l.max_side {
                    return Err(ParseError::Limit {
                        kind: "image size".into(),
                        limit: u64::from(l.max_side),
                    });
                }
                check_pixels(w, h, l)?;
                let ct = d.colortype().map_err(corrupt)?;
                let unsupported =
                    |what: String| ParseError::Unsupported(Format::Tiff).to_string() + &what;
                let DecodingResult::U8(buf) = d.read_image().map_err(|e| match e {
                    tiff::TiffError::LimitsExceeded => ParseError::Limit {
                        kind: "image bytes".into(),
                        limit: l.max_alloc,
                    },
                    e => corrupt(e),
                })?
                else {
                    return Err(corrupt(unsupported(format!(": sample type of {ct:?}"))));
                };
                let rgb: Vec<u8> = match ct {
                    C::Gray(8) => buf.iter().flat_map(|&v| [v, v, v]).collect(),
                    C::RGB(8) => buf,
                    C::RGBA(8) => rgba_to_rgb(&buf),
                    C::Gray(1) => {
                        let row = (w as usize).div_ceil(8);
                        if buf.len() < row * h as usize {
                            return Err(corrupt("TIFF bilevel data too short"));
                        }
                        (0..h as usize)
                            .flat_map(|y| (0..w as usize).map(move |x| (y, x)))
                            .flat_map(|(y, x)| {
                                let v = if buf[y * row + x / 8] >> (7 - x % 8) & 1 == 1 {
                                    255
                                } else {
                                    0
                                };
                                [v, v, v]
                            })
                            .collect()
                    }
                    other => return Err(corrupt(unsupported(format!(": color type {other:?}")))),
                };
                Image::from_raw(w, h, rgb).ok_or_else(|| corrupt("TIFF size mismatch"))
            })?;
            f(i, img)?;
        }
        i += 1;
        if !d.more_images() || i > l.max_frames {
            return Ok(i);
        }
        no_panic(|| d.next_image().map_err(corrupt))?;
    }
}

/// Downscales a frame for OCR; returns the image and the factor (OCR px per source px).
/// `thumbnail` (box filter) allocates only the u8 output; `resize` would add an f32 RGBA
/// intermediate of `width * new_height * 16` bytes and break the per-frame memory budget.
pub(crate) fn fit_for_ocr(img: Image, max_side: u32) -> (Image, f32) {
    let long = img.width().max(img.height());
    if long <= max_side {
        return (img, 1.0);
    }
    let s = max_side as f32 / long as f32;
    let (w, h) = (
        ((img.width() as f32 * s).round() as u32).max(1),
        ((img.height() as f32 * s).round() as u32).max(1),
    );
    let small = image::imageops::thumbnail(&img, w, h);
    (small, w as f32 / img.width() as f32)
}

/// Image parser; every image goes to OCR (need-OCR research: images always OCR, det decides).
pub struct ImageParser {
    config: OcrConfig,
}

impl ImageParser {
    pub fn new(config: OcrConfig) -> Self {
        Self { config }
    }
}

impl Parser for ImageParser {
    fn formats(&self) -> &[Format] {
        &[
            Format::Png,
            Format::Jpeg,
            Format::Tiff,
            Format::Bmp,
            Format::Webp,
        ]
    }

    fn parse(&self, input: Input, format: Format, opts: &ParseOptions) -> Result<Document> {
        if !self.formats().contains(&format) {
            return Err(ParseError::Unsupported(format));
        }
        let bytes = input.into_bytes()?;
        let l = self.config.limits;
        let take = opts.max_pages.unwrap_or(usize::MAX).min(l.max_frames);
        let engine = self
            .config
            .engine
            .as_deref()
            .filter(|_| opts.ocr != OcrMode::Never);
        let ocr_limit = opts.ocr_max_pages.unwrap_or(usize::MAX);
        // Job timeout: `run_parser` detaches this thread on expiry; stop OCRing frames then.
        let job_deadline = opts.timeout.and_then(|t| Instant::now().checked_add(t));
        let mut sections = Vec::new();
        let mut unavailable: Option<String> = None;
        let frames = for_each_page(&bytes, format, &l, take, |i, img| {
            if let (Some(t), Some(d)) = (opts.timeout, job_deadline)
                && Instant::now() >= d
            {
                return Err(ParseError::Timeout(t));
            }
            let mut s = Section::new(SectionKind::Page, i);
            s.source = TextSource::Ocr;
            s.quality = 0.0;
            let image_block = Block::Image {
                alt: None,
                ocr_text: None,
                bbox: None,
                image: None,
            };
            let Some(engine) = engine.filter(|_| unavailable.is_none() && i < ocr_limit) else {
                s.blocks.push(image_block);
                sections.push(s);
                return Ok(());
            };
            let (w, h) = img.dimensions();
            let (img, k) = fit_for_ocr(img, l.ocr_max_side);
            let mut params = self.config.params;
            let page_deadline = opts
                .ocr_page_timeout
                .and_then(|t| Instant::now().checked_add(t));
            params.deadline = match (page_deadline, job_deadline) {
                (Some(p), Some(j)) => Some(p.min(j)),
                (p, j) => p.or(j),
            };
            match ocr_image(engine, &img, &params) {
                Ok(page) => {
                    let mut geom = PageGeom::image(w, h);
                    geom.scale = k;
                    s.blocks = ocr_blocks(&page, &geom, (0.0, 0.0), &|_| true);
                    if s.blocks.is_empty() {
                        s.blocks.push(image_block);
                    }
                    let (w, q) = ocr_warnings(&[&page], "image", self.config.low_confidence);
                    s.warnings = w;
                    s.quality = q;
                    if page.timed_out {
                        let after = match opts.ocr_page_timeout {
                            Some(t) => format!(" after {t:?}"),
                            None => " (job timeout)".into(),
                        };
                        s.warnings.push(Warning::new(
                            WarningKind::PartialContent,
                            Level::Warn,
                            format!(
                                "OCR timed out{after}: {} line(s) recognised",
                                page.lines.len()
                            ),
                        ));
                    }
                }
                Err(OcrError::ModelsUnavailable(m)) => {
                    unavailable = Some(m); // reported once per document
                    s.blocks.push(image_block);
                }
                Err(e) => {
                    s.blocks.push(image_block);
                    s.warnings.push(Warning::new(
                        WarningKind::PartialContent,
                        Level::Loss,
                        format!("OCR failed: {e}"),
                    ));
                }
            }
            sections.push(s);
            Ok(())
        })?;
        let n = sections.len();
        let mut doc = Document::new(sections);
        doc.metadata.page_count = Some(frames);
        let why = match (opts.ocr, &self.config.engine, unavailable) {
            (OcrMode::Never, _, _) => Some("ocr = never".to_string()),
            (_, None, _) => Some("no OCR engine in this build".to_string()),
            (_, _, m) => m,
        };
        if let Some(why) = why {
            doc.warnings.push(Warning::new(
                WarningKind::OcrUnavailable,
                Level::Warn,
                format!("image text not extracted: {why}"),
            ));
        }
        if engine.is_some() && n > ocr_limit {
            doc.warnings.push(Warning::new(
                WarningKind::PartialContent,
                Level::Warn,
                format!("OCR limited to {ocr_limit} of {n} pages that need it"),
            ));
        }
        if n < frames {
            let more = if frames > l.max_frames { "+" } else { "" };
            let why = if opts.max_pages.is_some_and(|m| m <= l.max_frames) {
                "max_pages"
            } else {
                "frame limit"
            };
            doc.warnings.push(Warning::new(
                WarningKind::PartialContent,
                Level::Warn,
                format!("processed {n} of {frames}{more} pages ({why})"),
            ));
        }
        Ok(doc)
    }
}

#[cfg(test)]
#[path = "images_test.rs"]
mod tests;
