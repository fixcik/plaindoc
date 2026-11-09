//! PDF pictures and the interim OCR policy (`pdf::pictures`), on hand-written PDFs: a page with
//! a text layer links its images instead of OCRing them, scans stay on OCR, running repeats and
//! decoration are left out, one raster is stored once.

use std::sync::Arc;

use plaindoc_core::{
    Block, Document, ImageInfo, Input, Level, OcrMode, ParseOptions, ParseStats, TextSource,
    WarningKind,
};
use plaindoc_formats::Registry;
use plaindoc_ocr::{FakeEngine, TextBox};

/// A raster image object of the test PDF (8-bit samples, stored uncompressed).
#[derive(Clone)]
struct Img {
    w: u32,
    h: u32,
    /// DeviceGray (`w * h` bytes) or DeviceRGB (`w * h * 3`).
    rgb: bool,
    data: Vec<u8>,
    /// DeviceGray soft mask of the same size.
    smask: Option<Vec<u8>>,
}

/// Dark 2x2 dots every 6 px on white: glyph-like ink for the need-OCR classifier.
fn dots(w: u32, h: u32) -> Img {
    let mut data = vec![255u8; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            if x % 6 < 2 && y % 6 < 2 {
                data[(y * w + x) as usize] = 0;
            }
        }
    }
    Img {
        w,
        h,
        rgb: false,
        data,
        smask: None,
    }
}

/// Smooth colour picture without glyph-like ink (a photo stand-in), `seed` makes it distinct.
fn photo(w: u32, h: u32, seed: u8) -> Img {
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            data.extend([
                (x * 255 / w) as u8,
                (y * 255 / h) as u8,
                seed.wrapping_add((x + y) as u8),
            ]);
        }
    }
    Img {
        w,
        h,
        rgb: true,
        data,
        smask: None,
    }
}

/// One page: text lines `(x, y, text)` in Helvetica 10 pt and images `(object index, x, y, w, h)`.
#[derive(Default, Clone)]
struct Page {
    lines: Vec<(f32, f32, String)>,
    images: Vec<(usize, f32, f32, f32, f32)>,
}

/// English prose of about `chars` non-space chars, as lines from the top of a letter page.
fn prose(chars: usize, top: f32) -> Vec<(f32, f32, String)> {
    const WORDS: &str = "the committee reviewed the annual report and agreed that the project \
        budget should cover training staff equipment and travel during the next financial year \
        while every department prepares a short summary of results";
    let words: Vec<&str> = WORDS.split_whitespace().collect();
    let (mut lines, mut line, mut n, mut i) = (Vec::new(), String::new(), 0, 0);
    while n < chars {
        let w = words[i % words.len()];
        i += 1;
        n += w.len();
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(w);
        if line.len() > 80 {
            lines.push(std::mem::take(&mut line));
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
        .into_iter()
        .enumerate()
        .map(|(k, t)| (72.0, top - 14.0 * k as f32, t))
        .collect()
}

/// A letter-size PDF of `pages` drawing `imgs` (one object each; masks are extra objects).
fn pdf(imgs: &[Img], pages: &[Page]) -> Vec<u8> {
    let mut objs: Vec<Vec<u8>> = Vec::new();
    let n_pages = pages.len();
    // 1 catalog, 2 pages, 3 font, then per page: page + content, then images (+ masks)
    let img0 = 4 + 2 * n_pages;
    let mut img_ids = Vec::new();
    let mut next = img0;
    for im in imgs {
        img_ids.push(next);
        next += 1 + usize::from(im.smask.is_some());
    }
    let kids: Vec<String> = (0..n_pages).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    objs.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objs.push(
        format!(
            "<< /Type /Pages /Kids [{}] /Count {n_pages} >>",
            kids.join(" ")
        )
        .into_bytes(),
    );
    objs.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    for (i, pg) in pages.iter().enumerate() {
        let xo: String = pg
            .images
            .iter()
            .map(|&(k, ..)| format!("/Im{k} {} 0 R ", img_ids[k]))
            .collect();
        objs.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << \
                 /F1 3 0 R >> /XObject << {xo}>> >> /Contents {} 0 R >>",
                5 + 2 * i
            )
            .into_bytes(),
        );
        let mut c = String::new();
        for &(k, x, y, w, h) in &pg.images {
            c.push_str(&format!("q {w} 0 0 {h} {x} {y} cm /Im{k} Do Q\n"));
        }
        for (x, y, t) in &pg.lines {
            c.push_str(&format!("BT /F1 10 Tf {x} {y} Td ({t}) Tj ET\n"));
        }
        objs.push(format!("<< /Length {} >>\nstream\n{c}\nendstream", c.len()).into_bytes());
    }
    for (k, im) in imgs.iter().enumerate() {
        let cs = if im.rgb { "/DeviceRGB" } else { "/DeviceGray" };
        let sm = if im.smask.is_some() {
            format!(" /SMask {} 0 R", img_ids[k] + 1)
        } else {
            String::new()
        };
        objs.push(stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {cs} \
                 /BitsPerComponent 8{sm}",
                im.w, im.h
            ),
            &im.data,
        ));
        if let Some(m) = &im.smask {
            objs.push(stream(
                &format!(
                    "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray \
                     /BitsPerComponent 8",
                    im.w, im.h
                ),
                m,
            ));
        }
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend(o);
        out.extend(b"\nendobj\n");
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

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut s = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    s.extend_from_slice(data);
    s.extend_from_slice(b"\nendstream");
    s
}

/// Parses with a fake OCR engine that reads `"seen by ocr"` anywhere; returns the document, the
/// stats and the number of detector calls.
fn parse_with(bytes: Vec<u8>, mode: OcrMode, images: bool) -> (Document, ParseStats, usize) {
    let e = Arc::new(FakeEngine::new(vec![(
        TextBox::from_aabb(10.0, 10.0, 200.0, 40.0, 0.9),
        "seen by ocr".to_string(),
        0.9,
    )]));
    let r = Registry::with_ocr_engine(e.clone());
    let mut o = ParseOptions::default();
    o.ocr = mode;
    o.images.enabled = images;
    let (d, s) = r.parse_with_stats(Input::Bytes(bytes), &o);
    (d.unwrap(), s, e.detect_calls())
}

fn parse(bytes: Vec<u8>, mode: OcrMode) -> (Document, ParseStats, usize) {
    parse_with(bytes, mode, true)
}

/// The pictures of section `s` in block order.
fn pictures(d: &Document, s: usize) -> Vec<(Option<&ImageInfo>, f32)> {
    d.sections[s]
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Image { image, bbox, .. } => Some((image.as_deref(), bbox.map_or(0.0, |b| b.y))),
            _ => None,
        })
        .collect()
}

fn ocr_text(d: &Document) -> bool {
    d.to_markdown().contains("seen by ocr")
}

fn skipped_warning(d: &Document, s: usize) -> Option<String> {
    d.sections[s]
        .warnings
        .iter()
        .find(|w| w.message.contains("not recognised"))
        .map(|w| {
            assert_eq!((w.kind, w.level), (WarningKind::ImageTextPage, Level::Info));
            w.message.clone()
        })
}

/// Text page with a dotted (glyph-like) picture of `w x h` pt: the classifier's ink rule wants
/// the merge OCR of its image.
fn mixed(chars: usize, w: f32, h: f32) -> Vec<u8> {
    let pg = Page {
        lines: prose(chars, 740.0),
        images: vec![(0, 72.0, 60.0, w, h)],
    };
    pdf(&[dots(240, 160)], &[pg])
}

#[test]
fn text_page_links_its_image_instead_of_ocr() {
    let (d, s, calls) = parse(mixed(600, 300.0, 200.0), OcrMode::Auto);
    assert_eq!(calls, 0, "no OCR on a page with a text layer");
    assert!(!ocr_text(&d));
    assert_eq!(d.sections[0].source, TextSource::TextLayer);
    let p = pictures(&d, 0);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].0.and_then(|i| i.file.as_deref()), Some("image1.png"));
    assert_eq!(d.images.len(), 1);
    let w = skipped_warning(&d, 0).expect("page warning");
    assert!(w.starts_with("1 image(s) not recognised"), "{w}");
    // the classifier's hint is not left at Warn
    assert!(
        d.sections[0]
            .warnings
            .iter()
            .all(|w| w.level == Level::Info),
        "{:?}",
        d.sections[0].warnings
    );
    assert_eq!((s.images.linked, s.images.ocr_skipped), (1, 1));
    assert_eq!(s.pages.ocr_wanted, 0);
    let md = d.to_markdown();
    assert!(md.contains("![изображение 1](image1.png)"), "{md}");
    // below the text: after the last paragraph
    assert!(matches!(
        d.sections[0].blocks.last(),
        Some(Block::Image { .. })
    ));
    let json = d.to_json();
    assert!(json.contains("\"file\": \"image1.png\""), "{json}");
}

#[test]
fn images_mode_keeps_the_ocr_and_adds_the_link() {
    let (d, s, calls) = parse(mixed(600, 300.0, 200.0), OcrMode::Images);
    assert!(calls > 0);
    assert!(ocr_text(&d));
    assert_eq!(d.sections[0].source, TextSource::Mixed);
    assert_eq!(pictures(&d, 0).len(), 1);
    assert_eq!(skipped_warning(&d, 0), None);
    assert_eq!((s.images.linked, s.images.ocr_skipped), (1, 0));
}

#[test]
fn always_and_never_link_too() {
    let (d, _, calls) = parse(mixed(600, 300.0, 200.0), OcrMode::Always);
    assert!(calls > 0 && ocr_text(&d));
    assert_eq!(pictures(&d, 0).len(), 1);
    let (d, _, calls) = parse(mixed(600, 300.0, 200.0), OcrMode::Never);
    assert_eq!(calls, 0);
    assert_eq!(pictures(&d, 0).len(), 1);
    assert_eq!(skipped_warning(&d, 0), None);
}

#[test]
fn half_page_picture_next_to_short_text_is_a_mixed_page() {
    // cover 0.5 < SERVICE_COVER: mixed at any text volume (here 150 chars)
    let (d, _, calls) = parse(mixed(150, 612.0, 396.0), OcrMode::Auto);
    assert_eq!(calls, 0);
    assert_eq!(pictures(&d, 0).len(), 1);
}

#[test]
fn scan_with_service_text_stays_on_ocr() {
    // cover 0.7 with a two-line footer (under SERVICE_TEXT): a scan, merge OCR, no picture
    let pg = Page {
        lines: prose(100, 40.0),
        images: vec![(0, 0.0, 200.0, 612.0, 554.0)],
    };
    let (d, s, calls) = parse(pdf(&[dots(400, 360)], &[pg]), OcrMode::Auto);
    assert!(calls > 0, "{:?}", d.sections[0].warnings);
    assert_eq!(d.sections[0].source, TextSource::Mixed, "merge OCR kept");
    assert!(pictures(&d, 0).is_empty());
    assert_eq!(skipped_warning(&d, 0), None);
    assert_eq!(s.images.ocr_skipped, 0);
    // the same page with a full text layer is a mixed page
    let pg = Page {
        lines: prose(400, 180.0),
        images: vec![(0, 0.0, 200.0, 612.0, 554.0)],
    };
    let (d, _, calls) = parse(pdf(&[dots(400, 360)], &[pg]), OcrMode::Auto);
    assert_eq!(calls, 0, "{:?}", d.sections[0].warnings);
    assert_eq!(pictures(&d, 0).len(), 1);
}

#[test]
fn full_page_scan_with_a_page_number_stays_on_ocr() {
    let pg = Page {
        lines: vec![(300.0, 20.0, "7".into())],
        images: vec![(0, 0.0, 0.0, 612.0, 792.0)],
    };
    let (d, _, calls) = parse(pdf(&[dots(400, 500)], &[pg]), OcrMode::Auto);
    assert!(calls > 0);
    assert_eq!(d.sections[0].source, TextSource::Ocr);
    assert!(pictures(&d, 0).is_empty());
    assert!(d.images.is_empty());
}

#[test]
fn small_illustration_on_a_text_page_is_linked_without_ocr() {
    // two paragraphs with the picture in the gap between them
    let mut lines = prose(300, 740.0);
    lines.extend(prose(300, 400.0));
    let pg = Page {
        lines,
        images: vec![(0, 72.0, 470.0, 120.0, 90.0)],
    };
    let (d, _, calls) = parse(pdf(&[photo(80, 60, 1)], &[pg]), OcrMode::Auto);
    assert_eq!(calls, 0);
    assert_eq!(pictures(&d, 0).len(), 1);
    // the picture sits by its top edge, among the paragraphs
    let pos = d.sections[0]
        .blocks
        .iter()
        .position(|b| matches!(b, Block::Image { .. }))
        .unwrap();
    assert!(
        pos > 0 && pos + 1 < d.sections[0].blocks.len(),
        "{:?}",
        d.sections[0].blocks
    );
}

#[test]
fn one_raster_on_several_pages_is_stored_once() {
    // same picture mid-page on two pages: two links, one file; a copy in a second object too
    let pg = Page {
        lines: prose(300, 740.0),
        images: vec![(0, 72.0, 300.0, 200.0, 150.0)],
    };
    let pg2 = Page {
        images: vec![(1, 72.0, 300.0, 200.0, 150.0)],
        ..pg.clone()
    };
    let im = photo(100, 75, 7);
    let (d, s, _) = parse(pdf(&[im.clone(), im], &[pg.clone(), pg2]), OcrMode::Auto);
    let files: Vec<_> = (0..2)
        .map(|i| pictures(&d, i)[0].0.and_then(|x| x.file.clone()))
        .collect();
    assert_eq!(
        files,
        [Some("image1.png".into()), Some("image1.png".into())]
    );
    assert_eq!(d.images.len(), 1);
    assert_eq!(s.images.linked, 2);
}

/// `n` text pages with a logo `logo` in the top margin band and a photo mid-page on the first.
fn logo_doc(n: usize, logo_h: f32) -> Vec<u8> {
    let pages: Vec<Page> = (0..n)
        .map(|i| {
            let mut images = vec![(0, 420.0, 740.0, 120.0, logo_h)];
            if i == 0 {
                images.push((1, 72.0, 300.0, 250.0, 180.0));
            }
            Page {
                lines: prose(300, 700.0),
                images,
            }
        })
        .collect();
    pdf(&[photo(120, 40, 3), photo(100, 72, 9)], &pages)
}

#[test]
fn running_logo_is_left_out_from_three_pages_on() {
    let (d, s, _) = parse(logo_doc(3, 40.0), OcrMode::Auto);
    let all: usize = (0..3).map(|i| pictures(&d, i).len()).sum();
    assert_eq!(all, 1, "only the photo");
    assert_eq!(d.images.len(), 1);
    assert_eq!(s.images.dropped, 3);
    // two pages: not enough to call it a running repeat
    let (d, _, _) = parse(logo_doc(2, 40.0), OcrMode::Auto);
    let all: usize = (0..2).map(|i| pictures(&d, i).len()).sum();
    assert_eq!(all, 3);
    assert_eq!(d.images.len(), 2);
}

#[test]
fn small_images_are_decoration() {
    // 31 pt high: under MIN_SIDE_PT; 32 pt: a picture
    for (h, want) in [(31.0, 0), (32.0, 1)] {
        let pg = Page {
            lines: prose(300, 700.0),
            images: vec![(0, 72.0, 300.0, 200.0, h)],
        };
        let (d, s, _) = parse(pdf(&[photo(100, 40, 5)], &[pg]), OcrMode::Auto);
        assert_eq!(pictures(&d, 0).len(), want, "{h} pt");
        assert_eq!(s.images.dropped, 1 - want as u64);
    }
    // a 10x10 px fill stretched over a large box
    let pg = Page {
        lines: prose(300, 700.0),
        images: vec![(0, 72.0, 300.0, 200.0, 100.0)],
    };
    let (d, _, _) = parse(pdf(&[photo(10, 10, 5)], &[pg]), OcrMode::Auto);
    assert!(pictures(&d, 0).is_empty());
}

#[test]
fn plates_are_decoration() {
    let plate = |lo: u8, hi: u8| Img {
        w: 64,
        h: 32,
        rgb: false,
        data: (0..64 * 32)
            .map(|i| if i % 2 == 0 { lo } else { hi })
            .collect(),
        smask: None,
    };
    for (hi, want) in [(232u8, 0), (233, 1)] {
        let pg = Page {
            lines: prose(300, 700.0),
            images: vec![(0, 72.0, 300.0, 200.0, 100.0)],
        };
        let (d, _, _) = parse(pdf(&[plate(200, hi)], &[pg]), OcrMode::Auto);
        assert_eq!(pictures(&d, 0).len(), want, "range {}", hi - 200);
    }
}

#[test]
fn soft_mask_gives_png_with_alpha() {
    let mut im = photo(40, 40, 2);
    im.smask = Some((0..40 * 40).map(|i| (i % 256) as u8).collect());
    let pg = Page {
        lines: prose(300, 700.0),
        images: vec![(0, 72.0, 300.0, 100.0, 100.0)],
    };
    let (d, _, _) = parse(pdf(&[im], &[pg]), OcrMode::Auto);
    let png = &d.images[0].data;
    // IHDR colour type 6: RGBA
    assert_eq!(&png[1..4], b"PNG");
    assert_eq!(png[25], 6);
}

#[test]
fn no_images_gives_bare_marks_and_still_skips_ocr() {
    let (d, s, calls) = parse_with(mixed(600, 300.0, 200.0), OcrMode::Auto, false);
    assert_eq!(calls, 0);
    let p = pictures(&d, 0);
    assert_eq!(p.len(), 1);
    assert!(p[0].0.is_none());
    assert!(d.images.is_empty());
    assert!(d.to_markdown().contains("<!-- image -->"));
    assert!(skipped_warning(&d, 0).is_some());
    assert_eq!((s.images.linked, s.images.ocr_skipped), (0, 1));
}

#[test]
fn text_page_without_images_is_untouched() {
    let pg = Page {
        lines: prose(600, 740.0),
        images: vec![],
    };
    let (d, s, _) = parse(pdf(&[], &[pg]), OcrMode::Auto);
    assert!(pictures(&d, 0).is_empty());
    assert_eq!(s.images, Default::default());
    assert!(!s.stages_us.contains_key(&plaindoc_core::Stage::Images));
}
