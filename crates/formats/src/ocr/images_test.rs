use super::*;
use plaindoc_ocr::{FakeEngine, TextBox};
use std::sync::Arc;

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut img = Image::from_pixel(w, h, image::Rgb([255, 255, 255]));
    for y in 20..40.min(h) {
        for x in 20..120.min(w) {
            img.put_pixel(x, y, image::Rgb([0, 0, 0]));
        }
    }
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn fake() -> OcrConfig {
    OcrConfig::with_engine(Arc::new(FakeEngine::new(vec![(
        TextBox::from_aabb(15.0, 15.0, 125.0, 45.0, 0.9),
        "Счёт 1250".into(),
        0.93,
    )])))
}

#[test]
fn png_is_ocred_into_one_page_with_bbox_in_points() {
    let d = ImageParser::new(fake())
        .parse(
            Input::Bytes(png(200, 100)),
            Format::Png,
            &ParseOptions::default(),
        )
        .unwrap();
    assert_eq!(d.sections.len(), 1);
    let s = &d.sections[0];
    assert_eq!(s.source, TextSource::Ocr);
    let Block::Paragraph { text, bbox, .. } = &s.blocks[0] else {
        panic!("{:?}", s.blocks)
    };
    let b = bbox.expect("bbox");
    assert_eq!(text, "Счёт 1250");
    // pixels 20..120 x 20..40 of a 100 px high image -> y from the bottom
    assert_eq!((b.x, b.y, b.w, b.h), (20.0, 60.0, 100.0, 20.0));
    assert!(s.warnings.iter().any(|w| w.kind == WarningKind::OcrUsed));
    assert!((s.quality - 0.93).abs() < 1e-6);
}

#[test]
fn never_and_missing_engine_report_unavailable() {
    let mut o = ParseOptions::default();
    o.ocr = OcrMode::Never;
    let d = ImageParser::new(fake())
        .parse(Input::Bytes(png(50, 50)), Format::Png, &o)
        .unwrap();
    assert!(
        d.warnings
            .iter()
            .any(|w| w.kind == WarningKind::OcrUnavailable)
    );
    let d = ImageParser::new(OcrConfig::default())
        .parse(
            Input::Bytes(png(50, 50)),
            Format::Png,
            &ParseOptions::default(),
        )
        .unwrap();
    assert!(
        d.warnings
            .iter()
            .any(|w| w.kind == WarningKind::OcrUnavailable && w.level == Level::Warn)
    );
    assert!(matches!(d.sections[0].blocks[0], Block::Image { .. }));
}

#[test]
fn exif_rotation_is_applied_and_garbage_is_corrupt() {
    assert!(matches!(
        decode_pages(b"\x89PNG\r\n\x1a\nxx", Format::Png),
        Err(ParseError::Corrupt(_))
    ));
    assert_eq!(
        decode_pages(&png(30, 10), Format::Png).unwrap()[0].dimensions(),
        (30, 10)
    );
}

#[test]
fn section_without_text_keeps_the_image_block() {
    // png(10, 10) is all white: the bar starts at y = 20.
    let c = OcrConfig::with_engine(Arc::new(FakeEngine::new(vec![])));
    let d = ImageParser::new(c)
        .parse(
            Input::Bytes(png(10, 10)),
            Format::Png,
            &ParseOptions::default(),
        )
        .unwrap();
    let s = &d.sections[0];
    assert!(
        matches!(s.blocks[..], [Block::Image { .. }]),
        "{:?}",
        s.blocks
    );
    assert!(s.warnings.iter().any(|w| w.kind == WarningKind::OcrEmpty));
}

#[test]
fn bmp_and_webp_are_decoded() {
    for (f, fmt) in [
        (image::ImageFormat::Bmp, Format::Bmp),
        (image::ImageFormat::WebP, Format::Webp),
    ] {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(Image::new(30, 10))
            .write_to(&mut Cursor::new(&mut bytes), f)
            .unwrap();
        let d = ImageParser::new(OcrConfig::default())
            .parse(Input::Bytes(bytes), fmt, &ParseOptions::default())
            .unwrap();
        assert_eq!((d.sections.len(), d.metadata.page_count), (1, Some(1)));
    }
}

#[test]
fn decoder_panic_becomes_internal() {
    let r: Result<()> = no_panic(|| panic!("hostile input"));
    assert!(matches!(r, Err(ParseError::Internal(m)) if m.contains("panicked")));
}

#[test]
fn multipage_tiff_gives_one_section_per_page() {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut enc = tiff::encoder::TiffEncoder::new(&mut buf).unwrap();
        for v in [0u8, 255] {
            enc.write_image::<tiff::encoder::colortype::Gray8>(8, 4, &[v; 32])
                .unwrap();
        }
    }
    let pages = decode_pages(buf.get_ref(), Format::Tiff).unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[1].get_pixel(0, 0)[0], 255);
}

/// JPEG 30x10 with an APP1 Exif segment (Orientation = `o`) spliced right after SOI.
fn jpeg_with_orientation(o: u8) -> Vec<u8> {
    let mut jpg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut jpg)
        .encode_image(&Image::new(30, 10))
        .unwrap();
    let mut app1 = vec![0xFF, 0xE1, 0x00, 0x22];
    app1.extend_from_slice(b"Exif\0\0II*\0");
    app1.extend_from_slice(&[
        0x08, 0, 0, 0, 0x01, 0x00, 0x12, 0x01, 0x03, 0x00, 0x01, 0, 0, 0, o, 0, 0, 0, 0, 0, 0, 0,
    ]);
    assert_eq!(app1.len(), 36);
    [&jpg[..2], &app1[..], &jpg[2..]].concat()
}

#[test]
fn exif_orientation_6_rotates_jpeg() {
    let d = |o| decode_pages(&jpeg_with_orientation(o), Format::Jpeg).unwrap()[0].dimensions();
    assert_eq!((d(1), d(6)), ((30, 10), (10, 30)));
}

fn tiff_frames(n: usize, w: u32, h: u32) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut enc = tiff::encoder::TiffEncoder::new(&mut buf).unwrap();
        let data = vec![255u8; (w * h) as usize];
        for _ in 0..n {
            enc.write_image::<tiff::encoder::colortype::Gray8>(w, h, &data)
                .unwrap();
        }
    }
    buf.into_inner()
}

#[test]
fn frames_are_streamed_and_capped() {
    let bytes = tiff_frames(5, 16, 8);
    let mut seen = Vec::new();
    let frames = for_each_page(&bytes, Format::Tiff, &ImageLimits::default(), 2, |i, _| {
        seen.push(i);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        (seen, frames),
        (vec![0, 1], 5),
        "only `take` frames decoded"
    );
    let mut c = OcrConfig::default();
    c.limits.max_frames = 3;
    let d = ImageParser::new(c)
        .parse(Input::Bytes(bytes), Format::Tiff, &ParseOptions::default())
        .unwrap();
    assert_eq!(d.sections.len(), 3);
    assert!(
        d.warnings
            .iter()
            .any(|w| w.kind == WarningKind::PartialContent
                && w.message.contains("processed 3 of 4+ pages (frame limit)"))
    );
}

#[test]
fn ocr_max_pages_and_page_timeout_apply_to_images() {
    // Three 200x100 frames with the bar of `png()`.
    let mut buf = Cursor::new(Vec::new());
    {
        let mut enc = tiff::encoder::TiffEncoder::new(&mut buf).unwrap();
        let mut data = vec![255u8; 200 * 100];
        for y in 20..40 {
            data[y * 200 + 20..y * 200 + 120].fill(0);
        }
        for _ in 0..3 {
            enc.write_image::<tiff::encoder::colortype::Gray8>(200, 100, &data)
                .unwrap();
        }
    }
    let e = Arc::new(FakeEngine::new(vec![(
        TextBox::from_aabb(15.0, 15.0, 125.0, 45.0, 0.9),
        "Счёт 1250".into(),
        0.93,
    )]));
    let mut o = ParseOptions::default();
    o.ocr_max_pages = Some(1);
    let d = ImageParser::new(OcrConfig::with_engine(e.clone()))
        .parse(Input::Bytes(buf.into_inner()), Format::Tiff, &o)
        .unwrap();
    assert_eq!((d.sections.len(), e.detect_calls()), (3, 1));
    assert!(matches!(d.sections[2].blocks[..], [Block::Image { .. }]));
    assert!(
        d.warnings
            .iter()
            .any(|w| w.kind == WarningKind::PartialContent
                && w.message.contains("OCR limited to 1 of 3 pages"))
    );
    let mut o = ParseOptions::default();
    o.ocr_page_timeout = Some(std::time::Duration::ZERO);
    let d = ImageParser::new(fake())
        .parse(Input::Bytes(png(200, 100)), Format::Png, &o)
        .unwrap();
    assert!(
        d.sections[0]
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::PartialContent && w.message.contains("OCR timed out")),
        "{:?}",
        d.sections[0].warnings
    );
}

#[test]
fn job_timeout_stops_the_frame_loop() {
    // Detection takes 40 ms a frame; the job has 60 ms: OCR must stop, not run all 6 frames.
    struct Slow(FakeEngine);
    impl plaindoc_ocr::OcrEngine for Slow {
        fn detect(&self, img: &Image) -> std::result::Result<Vec<TextBox>, OcrError> {
            std::thread::sleep(std::time::Duration::from_millis(40));
            self.0.detect(img)
        }
        fn recognize(
            &self,
            img: &Image,
            b: &[TextBox],
        ) -> std::result::Result<Vec<plaindoc_ocr::Recognized>, OcrError> {
            self.0.recognize(img, b)
        }
    }
    let mut buf = Cursor::new(Vec::new());
    {
        let mut enc = tiff::encoder::TiffEncoder::new(&mut buf).unwrap();
        let mut data = vec![255u8; 200 * 100];
        for y in 20..40 {
            data[y * 200 + 20..y * 200 + 120].fill(0);
        }
        for _ in 0..6 {
            enc.write_image::<tiff::encoder::colortype::Gray8>(200, 100, &data)
                .unwrap();
        }
    }
    let e = Arc::new(Slow(FakeEngine::new(vec![(
        TextBox::from_aabb(15.0, 15.0, 125.0, 45.0, 0.9),
        "Счёт 1250".into(),
        0.93,
    )])));
    let mut c = OcrConfig::with_engine(e.clone());
    c.params.orientation = false;
    let mut o = ParseOptions::default();
    o.timeout = Some(std::time::Duration::from_millis(60));
    let r = ImageParser::new(c).parse(Input::Bytes(buf.into_inner()), Format::Tiff, &o);
    assert!(matches!(r, Err(ParseError::Timeout(_))), "{:?}", r.err());
    assert!(
        e.0.detect_calls() < 6,
        "{} frames OCRed",
        e.0.detect_calls()
    );
}

#[test]
fn huge_page_timeout_means_no_deadline_not_a_panic() {
    let mut o = ParseOptions::default();
    o.ocr_page_timeout = Some(std::time::Duration::MAX);
    let d = ImageParser::new(fake())
        .parse(Input::Bytes(png(200, 100)), Format::Png, &o)
        .unwrap();
    assert!(
        d.sections[0]
            .warnings
            .iter()
            .all(|w| !w.message.contains("timed out"))
    );
}

#[test]
fn oversized_images_are_a_limit_not_corrupt() {
    // Within max_side but the RGB frame (300 * 300 * 3 = 270 000 B) exceeds max_alloc.
    let mut c = OcrConfig::default();
    c.limits.max_alloc = 200_000;
    for (bytes, f) in [
        (png(300, 300), Format::Png),
        (tiff_frames(1, 300, 300), Format::Tiff),
    ] {
        let r = ImageParser::new(c.clone()).parse(Input::Bytes(bytes), f, &ParseOptions::default());
        assert!(
            matches!(&r, Err(ParseError::Limit { kind, limit: 200_000 }) if kind == "image pixels"),
            "{f:?}: {:?}",
            r.err()
        );
    }
    let mut c = OcrConfig::default();
    c.limits.max_side = 100;
    for (bytes, f) in [
        (png(200, 50), Format::Png),
        (tiff_frames(1, 200, 50), Format::Tiff),
    ] {
        let r = ImageParser::new(c.clone()).parse(Input::Bytes(bytes), f, &ParseOptions::default());
        assert!(
            matches!(r, Err(ParseError::Limit { limit: 100, .. })),
            "{f:?}: {:?}",
            r.err()
        );
    }
}

#[test]
fn large_frames_are_downscaled_for_ocr_and_boxes_stay_in_source_pixels() {
    // 200x100 -> 100x50 for OCR; the scripted box is in OCR pixels around the half-size bar.
    let mut c = OcrConfig::with_engine(Arc::new(FakeEngine::new(vec![(
        TextBox::from_aabb(7.0, 7.0, 63.0, 23.0, 0.9),
        "Счёт".into(),
        0.93,
    )])));
    c.limits.ocr_max_side = 100;
    // Deskew (task 8) reads -0.5° off the 50 px blurred bar of the 100x50 raster and would tilt
    // the box; this test is about the downscale mapping only.
    c.params.deskew = false;
    let d = ImageParser::new(c)
        .parse(
            Input::Bytes(png(200, 100)),
            Format::Png,
            &ParseOptions::default(),
        )
        .unwrap();
    let Block::Paragraph { bbox: Some(b), .. } = &d.sections[0].blocks[0] else {
        panic!("{:?}", d.sections[0].blocks)
    };
    let near = |a: f32, e: f32| (a - e).abs() <= 2.0;
    assert!(
        near(b.x, 20.0) && near(b.w, 100.0) && near(b.y, 60.0) && near(b.h, 20.0),
        "{b:?}"
    );
    let (img, k) = fit_for_ocr(Image::new(8000, 2000), 4000);
    assert_eq!((img.dimensions(), k), ((4000, 1000), 0.5));
}
