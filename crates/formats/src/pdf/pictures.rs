//! Pictures of PDF pages: the interim policy until an image classifier exists.
//!
//! A page that keeps its text layer does not send its images to OCR (`OcrMode::Auto`); its raster
//! images become `Block::Image` pictures (files and links, as in office documents). Scans stay on
//! OCR and give no pictures: pages without a usable layer (the need-OCR classifier, unchanged),
//! pages mostly covered by images ([`SCAN_COVER`]) and "scans with service text" ([`service_scan`]).
//! `OcrMode::Images` keeps the OCR of images on such pages (the behaviour before the policy) and
//! links the pictures as well.
//!
//! Left out as page furniture: running repeats (the same raster on [`REPEAT_MIN_PAGES`] or more
//! pages in the top or bottom [`MARGIN_BAND`], such as a header logo), small images
//! ([`MIN_SIDE_PT`], [`MIN_SIDE_PX`]: rules, bullets, icons) and plates (one colour or a faint
//! gradient, [`PLATE_RANGE`]; not checked on JPEGs, which are passed through undecoded).
//!
//! Stage A lists the images of each page from their XObject dictionaries ([`collect`], nothing is
//! decoded), [`decode_pass`] decodes each kept raster once (by content hash) and stage B places the
//! pictures among the page's blocks by their top edge ([`place`]).

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use pdf_oxide::extractors::PdfImage;
use pdf_oxide::{PdfDocument, PdfFilter, PdfImageHandle};
use plaindoc_core::{BBox, Block, ImageInfo, Level, Warning, WarningKind};
use rayon::prelude::*;

use super::diagnose::{PageDecision, PageDiagnosis};
use super::table_place::place_by_top;

/// Image cover of a page (summed image area over the page area, `classify_page`) from which the
/// page is a scan whatever its text layer: the need-OCR classifier's `scan_cover`. Such a page
/// keeps the OCR it was given and gives no pictures (the image is the page or its background; a
/// searchable scan has a full text layer over it).
pub(crate) const SCAN_COVER: f32 = 0.8;

/// "Scan with service text", lower cover: a page whose images cover at least this share and
/// whose text layer is under [`SERVICE_TEXT`] is a scan with a page number, header or footer as
/// its only text. 0.6 is `ocr_stage::MAX_REGION_SHARE`, above which the merge OCR already reads
/// such a page as a whole. A half-page picture (cover 0.5) next to text is a mixed page at any
/// text volume.
pub(crate) const SERVICE_COVER: f32 = 0.6;

/// "Scan with service text", text volume: fewer non-space chars of the text layer than this.
/// About three lines (page number, running header, footer, a stamp line); the mixed pages of the
/// test brochure carry 320 and 540 chars at cover 0.34 and 0.41.
pub(crate) const SERVICE_TEXT: usize = 200;

/// Smaller displayed side than this (points, 32 pt = 11 mm) is decoration: rules, bullets,
/// icons, thin plates.
pub(crate) const MIN_SIDE_PT: f32 = 32.0;

/// Smaller stored side than this (pixels) is decoration too (a stretched 1x1 or 10x10 fill).
pub(crate) const MIN_SIDE_PX: u32 = 16;

/// Top and bottom share of the page where a repeated image is a running header or footer.
pub(crate) const MARGIN_BAND: f32 = 0.15;

/// Pages on which the same raster in the margin band makes a running repeat; documents with fewer
/// pages keep their repeats (as the fork's running header/footer scan, which needs 3 pages).
pub(crate) const REPEAT_MIN_PAGES: usize = 3;

/// Largest spread of any channel (alpha included) of a plate: an image whose every channel stays
/// within this many levels (of 255) is a fill or a faint gradient, left out as decoration.
pub(crate) const PLATE_RANGE: u8 = 32;

/// Images larger than this many pixels are not decoded (`skipped: too_large`).
const MAX_DECODE_PX: u64 = 64 << 20;

/// One image painted on a page (stage A, not decoded).
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    /// Hash of the stored stream bytes and size: the same raster gives the same key.
    pub key: u64,
    /// Index among the images painted on the page (`PdfImageHandle::paint_order`).
    pub paint_order: usize,
    /// Displayed box in PDF user space (y up), clipped to the page.
    pub bbox: BBox,
    /// Centre in the top or bottom [`MARGIN_BAND`] of the page.
    pub band: bool,
    /// Under [`MIN_SIDE_PT`] or [`MIN_SIDE_PX`]: decoration. Still counted for running repeats
    /// (a header logo may be small on most pages and larger on the title page).
    pub small: bool,
}

/// Images of one page that may become pictures.
#[derive(Clone, Debug, Default)]
pub(crate) struct PagePictures {
    pub cands: Vec<Candidate>,
    /// Images not sent to OCR by the policy (the page's `cands` when its OCR was skipped).
    pub ocr_skipped: usize,
}

/// Page with a dominant image and almost no text: a scan with service text (page number, header).
pub(crate) fn service_scan(cover: f32, nonws: usize) -> bool {
    cover >= SERVICE_COVER && nonws < SERVICE_TEXT
}

/// Whether the page gives pictures: it keeps its text layer (`UseTextLayer`, or `RunOcrAndMerge`
/// that merges OCR into the layer), has images, and is not a scan ([`SCAN_COVER`]; a merge page
/// that is a [`service_scan`] stays a scan under OCR).
pub(crate) fn takes_pictures(decision: PageDecision, cover: f32, nonws: usize) -> bool {
    match decision {
        PageDecision::UseTextLayer => cover > 0.0 && cover < SCAN_COVER,
        PageDecision::RunOcrAndMerge => {
            cover > 0.0 && cover < SCAN_COVER && !service_scan(cover, nonws)
        }
        PageDecision::RunOcr | PageDecision::Reject => false,
    }
}

/// Lists the images of page `p` (no decoding). `None` when the page's images cannot be read.
pub(crate) fn collect(doc: &PdfDocument, p: usize) -> Option<PagePictures> {
    let info = doc.get_page_info(p).ok()?;
    let handles = doc.page_image_handles(p).ok()?;
    let m = info.media_box;
    let page = [m.x, m.y, m.x + m.width, m.y + m.height];
    let sideways = info.rotation.rem_euclid(180) == 90;
    let mut out = PagePictures::default();
    for h in &handles {
        let b = &h.bbox;
        let r = [
            b.x.max(page[0]),
            b.y.max(page[1]),
            (b.x + b.width).min(page[2]),
            (b.y + b.height).min(page[3]),
        ];
        // off the page or degenerate: not painted where anyone looks
        if !(r[2] > r[0] && r[3] > r[1]) {
            continue;
        }
        let (w, h_pt) = (r[2] - r[0], r[3] - r[1]);
        let Ok(raw) = h.raw_compressed_bytes() else {
            continue;
        };
        let mut hs = DefaultHasher::new();
        (h.width, h.height, &raw).hash(&mut hs);
        // the band runs along the page's visual top and bottom
        let (c, lo, hi) = if sideways {
            ((r[0] + r[2]) / 2.0, page[0], page[2])
        } else {
            ((r[1] + r[3]) / 2.0, page[1], page[3])
        };
        let band = (hi - lo) * MARGIN_BAND;
        out.cands.push(Candidate {
            key: hs.finish(),
            paint_order: h.paint_order,
            bbox: BBox::new(r[0], r[1], w, h_pt),
            band: c < lo + band || c > hi - band,
            small: w.min(h_pt) < MIN_SIDE_PT || h.width.min(h.height) < MIN_SIDE_PX,
        });
    }
    Some(out)
}

/// The interim policy on one page's stage-1 diagnosis: under `skip` (OCR of images on a page
/// with a text layer is off, i.e. `OcrMode::Auto`) a page whose only OCR would be the merge of
/// its images keeps its layer. Its decision becomes `UseTextLayer`, the stage-1 `ImageTextPage`
/// hint drops from `Warn` to `Info` (the page warning of [`place`] says what was not recognised)
/// and `pics.ocr_skipped` counts its images that are not decoration. A page with decoration only
/// (or vector text, no images) keeps its OCR.
pub(crate) fn skip_image_ocr(
    diag: &mut PageDiagnosis,
    wanted: PageDecision,
    pics: &mut PagePictures,
) {
    let n = pics.cands.iter().filter(|c| !c.small).count();
    if wanted != PageDecision::RunOcrAndMerge || n == 0 {
        return;
    }
    diag.decision = PageDecision::UseTextLayer;
    for w in &mut diag.warnings {
        if w.kind == WarningKind::ImageTextPage && w.level == Level::Warn {
            w.level = Level::Info;
        }
    }
    pics.ocr_skipped = n;
}

/// Keys of the running repeats among `pages`: rasters in the margin band on at least
/// [`REPEAT_MIN_PAGES`] pages (none in documents with fewer pages).
pub(crate) fn running_repeats<'a>(
    pages: impl Iterator<Item = &'a PagePictures>,
    page_count: usize,
) -> HashSet<u64> {
    if page_count < REPEAT_MIN_PAGES {
        return HashSet::new();
    }
    let mut on: HashMap<u64, usize> = HashMap::new();
    for pg in pages {
        let keys: HashSet<u64> = pg.cands.iter().filter(|c| c.band).map(|c| c.key).collect();
        for k in keys {
            *on.entry(k).or_default() += 1;
        }
    }
    on.into_iter()
        .filter(|&(_, n)| n >= REPEAT_MIN_PAGES)
        .map(|(k, _)| k)
        .collect()
}

/// A decoded picture.
#[derive(Clone, Debug)]
pub(crate) enum Pic {
    /// JPEG as stored, or PNG (with alpha when the image has a soft mask).
    Data(Arc<[u8]>),
    /// A plate ([`PLATE_RANGE`]), left out as decoration.
    Plate,
    /// Not decodable, or over [`MAX_DECODE_PX`] (the reason of `ImageInfo::skipped`).
    Skipped(&'static str),
}

/// Whether a candidate is left out: decoration or a running repeat.
fn dropped(c: &Candidate, repeats: &HashSet<u64>) -> bool {
    c.small || (c.band && repeats.contains(&c.key))
}

/// Decodes every raster that stays a picture, once per key, on the pool's threads when
/// `parallel`. `pages[p]` are the pictures of page `p`.
pub(crate) fn decode_pass(
    doc: &PdfDocument,
    pages: &[Option<&PagePictures>],
    repeats: &HashSet<u64>,
    parallel: bool,
) -> HashMap<u64, Pic> {
    let mut seen = HashSet::new();
    let mut todo: Vec<(usize, usize, u64)> = Vec::new();
    for (p, pg) in pages.iter().enumerate() {
        for c in pg.iter().flat_map(|g| &g.cands) {
            if !dropped(c, repeats) && seen.insert(c.key) {
                todo.push((p, c.paint_order, c.key));
            }
        }
    }
    let run = |&(p, order, key): &(usize, usize, u64)| (key, decode_one(doc, p, order));
    if parallel && todo.len() > 1 {
        todo.par_iter().map(run).collect()
    } else {
        todo.iter().map(run).collect()
    }
}

/// Decodes the image painted `order`-th on page `p`; decoder panics on hostile data become
/// `bad_data`.
fn decode_one(doc: &PdfDocument, p: usize, order: usize) -> Pic {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let hs = doc.page_image_handles(p).ok()?;
        let h = hs.iter().find(|h| h.paint_order == order)?;
        Some(encode(h))
    }));
    r.ok().flatten().unwrap_or(Pic::Skipped("bad_data"))
}

fn encode(h: &PdfImageHandle<'_>) -> Pic {
    if u64::from(h.width) * u64::from(h.height) > MAX_DECODE_PX {
        return Pic::Skipped("too_large");
    }
    let mask = h.soft_mask().ok().flatten();
    // A plain (not CMYK, unmasked) JPEG goes out as stored.
    if mask.is_none()
        && h.filter_chain == [PdfFilter::DCTDecode]
        && h.color_space.components() != 4
        && let Ok(b) = h.raw_compressed_bytes()
    {
        return Pic::Data(b.into());
    }
    let Some(img) = h.decode().ok().and_then(|i| i.to_dynamic_image().ok()) else {
        return Pic::Skipped("bad_data");
    };
    let img = match mask
        .as_ref()
        .and_then(|m| alpha(m, img.width(), img.height()))
    {
        Some(a) => {
            let mut rgba = img.to_rgba8();
            for (px, v) in rgba.pixels_mut().zip(a.as_raw()) {
                px[3] = *v;
            }
            image::DynamicImage::ImageRgba8(rgba)
        }
        None => img,
    };
    if plate(&img) {
        return Pic::Plate;
    }
    png(&img).map_or(Pic::Skipped("bad_data"), |b| Pic::Data(b.into()))
}

/// The soft mask as an alpha plane of `w` x `h` (resampled when its size differs).
fn alpha(m: &PdfImage, w: u32, h: u32) -> Option<image::GrayImage> {
    let g = m.to_dynamic_image().ok()?.to_luma8();
    if g.width() == 0 || g.height() == 0 {
        return None;
    }
    if g.dimensions() == (w, h) {
        return Some(g);
    }
    Some(image::imageops::resize(
        &g,
        w,
        h,
        image::imageops::FilterType::Triangle,
    ))
}

/// Every channel (alpha included) within [`PLATE_RANGE`] levels. Images of more than 8 bits per
/// channel are never plates.
pub(crate) fn plate(img: &image::DynamicImage) -> bool {
    use image::DynamicImage as D;
    let n = match img {
        D::ImageLuma8(_) => 1,
        D::ImageLumaA8(_) => 2,
        D::ImageRgb8(_) => 3,
        D::ImageRgba8(_) => 4,
        _ => return false,
    };
    let b = img.as_bytes();
    if b.len() < n {
        return true;
    }
    let (mut lo, mut hi) = ([u8::MAX; 4], [0u8; 4]);
    for px in b.chunks_exact(n) {
        for (i, v) in px.iter().enumerate() {
            lo[i] = lo[i].min(*v);
            hi[i] = hi[i].max(*v);
        }
    }
    (0..n).all(|i| hi[i] - lo[i] <= PLATE_RANGE)
}

fn png(img: &image::DynamicImage) -> Option<Vec<u8>> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let mut out = Vec::new();
    let enc =
        PngEncoder::new_with_quality(&mut out, CompressionType::Default, FilterType::Adaptive);
    img.write_with_encoder(enc).ok()?;
    Some(out)
}

/// What [`place`] did on one page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Placed {
    /// Pictures added to the page.
    pub linked: usize,
    /// Left out: running repeats, small images and plates.
    pub dropped: usize,
}

/// Adds the pictures of one page to `blocks` by their top edge (`place_by_top`), in paint order;
/// `decoded` from [`decode_pass`] (`None`: pictures are off, bare marks without decoding). A page
/// whose image OCR was skipped gets an `ImageTextPage`/Info warning with the counts.
pub(crate) fn place(
    blocks: &mut Vec<Block>,
    warnings: &mut Vec<Warning>,
    pg: &PagePictures,
    repeats: &HashSet<u64>,
    decoded: Option<&HashMap<u64, Pic>>,
) -> Placed {
    let mut n = Placed::default();
    for c in &pg.cands {
        if dropped(c, repeats) {
            n.dropped += 1;
            continue;
        }
        let info = match decoded.map(|d| d.get(&c.key)) {
            None => None,
            Some(Some(Pic::Data(b))) => Some(ImageInfo::embedded(Arc::clone(b), None)),
            Some(Some(Pic::Plate)) => {
                n.dropped += 1;
                continue;
            }
            Some(Some(Pic::Skipped(r))) => Some(ImageInfo::skipped(r, None)),
            Some(None) => Some(ImageInfo::skipped("bad_data", None)),
        };
        let mut b = Block::image(None, None, Some(c.bbox));
        if let Block::Image { image, .. } = &mut b {
            *image = info.map(Box::new);
        }
        place_by_top(blocks, b);
        n.linked += 1;
    }
    if pg.ocr_skipped > 0 {
        warnings.push(Warning::new(
            WarningKind::ImageTextPage,
            Level::Info,
            format!(
                "{} image(s) not recognised: OCR is skipped on a page with a text layer ({} kept as pictures)",
                pg.ocr_skipped, n.linked
            ),
        ));
    }
    n
}

#[cfg(test)]
#[path = "pictures_test.rs"]
mod tests;
