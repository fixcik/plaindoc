use super::*;
use crate::raster::tests::lines_page;
use crate::{AlignFrame, FakeEngine, Recognized, TextBox, scripted_recognition};

/// White page with black bars where the scripted words are.
fn page_with(bars: &[[u32; 4]]) -> Image {
    let mut img = Image::from_pixel(400, 200, image::Rgb([255, 255, 255]));
    for b in bars {
        for y in b[1]..b[3] {
            for x in b[0]..b[2] {
                img.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
    }
    img
}

#[test]
fn fake_engine_page_gives_words_lines_and_conf() {
    let img = page_with(&[[20, 20, 60, 40], [80, 20, 140, 40], [20, 120, 100, 140]]);
    let e = FakeEngine::new(vec![
        (
            TextBox::from_aabb(15.0, 15.0, 145.0, 45.0, 0.9),
            "Bыпуск нoмер".into(),
            0.95,
        ),
        (
            TextBox::from_aabb(15.0, 115.0, 105.0, 145.0, 0.9),
            "шум".into(),
            0.3,
        ),
        (
            TextBox::from_aabb(15.0, 160.0, 105.0, 190.0, 0.9),
            "второй".into(),
            0.85,
        ),
    ]);
    let p = ocr_image(&e, &img, &OcrParams::default()).unwrap();
    assert_eq!(e.detect_calls(), 1);
    let texts: Vec<&str> = p.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        texts,
        vec!["Выпуск номер", "второй"],
        "low-conf box dropped, homoglyphs fixed"
    );
    assert_eq!(p.words[0].text, "Выпуск");
    assert_eq!(p.words[0].bbox, [20.0, 20.0, 60.0, 40.0]);
    assert_eq!(p.words[0].char_x.len(), 6, "char_x filled from WordBox");
    assert_eq!(p.words[1].bbox, [80.0, 20.0, 140.0, 40.0]);
    assert!((p.mean_conf - (0.95 * 11.0 + 0.85 * 6.0) / 17.0).abs() < 1e-5);
    assert!(p.has_text(5, 0.8));
}

#[test]
fn blank_page_skips_detector_and_failures_propagate() {
    let e = FakeEngine::new(vec![]);
    let p = ocr_image(
        &e,
        &Image::from_pixel(300, 300, image::Rgb([255; 3])),
        &OcrParams::default(),
    )
    .unwrap();
    assert!(p.blank && p.words.is_empty());
    assert_eq!(e.detect_calls(), 0);
    let img = page_with(&[[20, 20, 300, 40]]);
    assert!(matches!(
        ocr_image(&FakeEngine::failing(), &img, &OcrParams::default()),
        Err(OcrError::Engine(_))
    ));
}

#[test]
fn deadline_and_cancel_stop_recognition_between_chunks() {
    // 40 boxes = 3 chunks; the engine moves the clock past the deadline during chunk 0.
    struct Slow(FakeEngine);
    impl OcrEngine for Slow {
        fn detect(&self, img: &Image) -> Result<Vec<TextBox>, OcrError> {
            self.0.detect(img)
        }
        fn recognize(
            &self,
            img: &Image,
            b: &[TextBox],
        ) -> Result<Vec<crate::Recognized>, OcrError> {
            std::thread::sleep(std::time::Duration::from_millis(300));
            self.0.recognize(img, b)
        }
    }
    let img = page_with(&[[20, 20, 300, 40]]);
    let items = || {
        (0..40)
            .map(|i| {
                let y = i as f32 * 4.0;
                (
                    TextBox::from_aabb(0.0, y, 50.0, y + 3.0, 0.9),
                    "xyz".to_string(),
                    0.9,
                )
            })
            .collect()
    };
    let mut p = OcrParams {
        deadline: Some(Instant::now()),
        ..Default::default()
    };
    let page = ocr_image(&FakeEngine::new(items()), &img, &p).unwrap();
    assert!(
        page.timed_out && page.words.is_empty(),
        "expired before chunk 0"
    );
    // keep the clock for recognition only (no blank rule, skew estimate or orientation probes)
    p.blank_check = false;
    p.deskew = false;
    p.orientation = false;
    p.deadline = Some(Instant::now() + std::time::Duration::from_millis(150));
    let page = ocr_image(&Slow(FakeEngine::new(items())), &img, &p).unwrap();
    assert!(page.timed_out);
    assert_eq!(
        page.lines
            .iter()
            .map(|l| l.text.split(' ').count())
            .sum::<usize>(),
        REC_CHUNK
    );
    let page = ocr_image_cancellable(
        &FakeEngine::new(items()),
        &img,
        &OcrParams::default(),
        &AtomicBool::new(true),
    )
    .unwrap();
    assert!(page.timed_out && page.words.is_empty());
    let p = OcrParams {
        max_boxes: 5,
        ..Default::default()
    };
    let page = ocr_image(&FakeEngine::new(items()), &img, &p).unwrap();
    assert_eq!((page.boxes_dropped, page.timed_out), (35, false));
}

#[test]
fn short_recognition_result_is_an_engine_error() {
    // A third-party engine returning fewer results than boxes must not shift texts.
    struct Short(FakeEngine);
    impl OcrEngine for Short {
        fn detect(&self, img: &Image) -> Result<Vec<TextBox>, OcrError> {
            self.0.detect(img)
        }
        fn recognize(
            &self,
            img: &Image,
            b: &[TextBox],
        ) -> Result<Vec<crate::Recognized>, OcrError> {
            let mut r = self.0.recognize(img, b)?;
            r.pop();
            Ok(r)
        }
    }
    let img = page_with(&[[20, 20, 300, 40], [20, 60, 300, 80]]);
    let e = Short(FakeEngine::new(vec![
        (
            TextBox::from_aabb(15.0, 15.0, 305.0, 45.0, 0.9),
            "a".into(),
            0.9,
        ),
        (
            TextBox::from_aabb(15.0, 55.0, 305.0, 85.0, 0.9),
            "b".into(),
            0.9,
        ),
    ]));
    let r = ocr_image(&e, &img, &OcrParams::default());
    assert!(matches!(r, Err(OcrError::Engine(_))), "{r:?}");
}

#[test]
fn same_input_same_output() {
    let img = page_with(&[[20, 20, 60, 40]]);
    let e = FakeEngine::new(vec![(
        TextBox::from_aabb(15.0, 15.0, 65.0, 45.0, 0.9),
        "x".into(),
        0.9,
    )]);
    let a = ocr_image(&e, &img, &OcrParams::default()).unwrap();
    assert_eq!(a, ocr_image(&e, &img, &OcrParams::default()).unwrap());
}

/// Orientation tests run without the blank rule and deskew (almost empty pages). A literal compiles
/// inside the crate despite `#[non_exhaustive]` (and clippy rejects field reassignment after default).
fn no_blank_no_deskew() -> OcrParams {
    OcrParams {
        blank_check: false,
        deskew: false,
        ..Default::default()
    }
}

// pipeline.rs: engine whose reading confidence depends on page orientation
struct Upright; // a black marker block at top-left means "upright"
impl OcrEngine for Upright {
    fn detect(&self, _img: &Image) -> Result<Vec<TextBox>, OcrError> {
        Ok((0..5)
            .map(|i| {
                TextBox::from_aabb(
                    50.0,
                    100.0 + 60.0 * i as f32,
                    800.0,
                    140.0 + 60.0 * i as f32,
                    0.9,
                )
            })
            .collect())
    }
    fn recognize(&self, img: &Image, b: &[TextBox]) -> Result<Vec<Recognized>, OcrError> {
        let up = img.get_pixel(5, 5)[0] < 128;
        Ok(b.iter()
            .map(|_| scripted_recognition("строка", if up { 0.95 } else { 0.4 }))
            .collect())
    }
}

#[test]
fn upside_down_page_is_turned_and_boxes_map_back() {
    let mut img = Image::from_pixel(900, 500, image::Rgb([255; 3]));
    for y in 490..500 {
        for x in 890..900 {
            img.put_pixel(x, y, image::Rgb([0; 3]));
        }
    } // marker bottom-right = page upside down
    // no blank/deskew noise on an almost empty page (std 3.67 vs blank threshold 3; skew undefined)
    let params = no_blank_no_deskew();
    let p = ocr_image(&Upright, &img, &params).unwrap();
    assert_eq!(p.rotation, 180);
    assert!(p.mean_conf > 0.9);
    assert!(
        p.words
            .iter()
            .all(|w| w.bbox[0] >= 0.0 && w.bbox[2] <= 900.0 && w.bbox[3] <= 500.0)
    );
    assert_eq!(
        p.frame.to_source([0.0, 0.0, 10.0, 10.0]),
        [890.0, 490.0, 900.0, 500.0]
    );
}

/// Engine that "sees" text only on a deskewed page: its 40 line boxes are in deskewed coordinates
/// (rows of `lines_page`), and it reads line i as "строка i" only if the raster under the box is
/// level (row profile of the box has one dark band); on a skewed raster it reads nothing.
struct Level40;
impl OcrEngine for Level40 {
    fn detect(&self, _img: &Image) -> Result<Vec<TextBox>, OcrError> {
        Ok((0..40)
            .map(|r| {
                TextBox::from_aabb(
                    95.0,
                    116.0 + 34.0 * r as f32,
                    1105.0,
                    134.0 + 34.0 * r as f32,
                    0.9,
                )
            })
            .collect())
    }
    fn recognize(&self, img: &Image, b: &[TextBox]) -> Result<Vec<Recognized>, OcrError> {
        Ok(b.iter()
            .map(|t| {
                let [x0, y0, x1, y1] = t.aabb().map(|v| v as u32);
                // level iff the left and right ends of the band are dark on the same rows
                let dark = |x: u32| (y0..y1).filter(|&y| img.get_pixel(x, y)[0] < 128).count();
                let row = ((y0 as f32 - 116.0) / 34.0).round() as u32;
                if dark(x0 + 10) > 4 && dark(x1 - 20) > 4 {
                    scripted_recognition(&format!("строка {row}"), 0.95)
                } else {
                    Recognized::default()
                }
            })
            .collect())
    }
}

/// Early-out of the alignment: below `MIN_SKEW_DEG` the page is not rotated (budget ruling: the
/// 30 ms `ocr_image` budget is for such pages), the aligned raster is the source one.
#[test]
fn level_page_is_not_rotated() {
    let img = lines_page(0.0);
    let p = OcrParams {
        keep_aligned: true,
        ..OcrParams::default()
    };
    let page = ocr_image(&Level40, &img, &p).unwrap();
    assert_eq!(page.skew_deg, 0.0);
    assert!(page.aligned_gray == Some(raster::gray(&img)), "no rotation");
    assert_eq!(page.lines.len(), 40);
}

#[test]
fn skewed_page_keeps_line_order() {
    let p = ocr_image(&Level40, &lines_page(1.5), &OcrParams::default()).unwrap();
    assert!((p.skew_deg + 1.5).abs() <= 0.15, "{}", p.skew_deg);
    let got: Vec<&str> = p.lines.iter().map(|l| l.text.as_str()).collect();
    let want: Vec<String> = (0..40).map(|r| format!("строка {r}")).collect();
    assert_eq!(
        got,
        want.iter().map(String::as_str).collect::<Vec<_>>(),
        "all 40 lines, in page order"
    );
    // boxes come back in the source raster: line 0 is near the top, line 39 near the bottom
    let src = |l: &OcrLine| p.frame.to_source(l.bbox);
    assert!(src(&p.lines[0])[1] < src(&p.lines[39])[1]);
    assert!(
        (p.words[0].angle - p.skew_deg).abs() < 1e-6,
        "word angle = skew + box angle"
    );
}

/// Box whose first edge runs bottom-to-top (text along y): `is_vertical()` (`from_aabb` is always horizontal).
fn vbox(x0: f32, y0: f32, x1: f32, y1: f32) -> TextBox {
    TextBox {
        quad: [[x0, y1], [x0, y0], [x1, y0], [x1, y1]],
        score: 0.9,
    }
}

/// Five vertical boxes; reading confidence `hit` when the marker is at the top-left of the variant, else `miss`.
struct Spines {
    hit: f32,
    miss: f32,
}
impl OcrEngine for Spines {
    fn detect(&self, _img: &Image) -> Result<Vec<TextBox>, OcrError> {
        Ok((0..5)
            .map(|i| vbox(50.0 + 60.0 * i as f32, 50.0, 80.0 + 60.0 * i as f32, 450.0))
            .collect())
    }
    fn recognize(&self, img: &Image, b: &[TextBox]) -> Result<Vec<Recognized>, OcrError> {
        let up = img.get_pixel(5, 5)[0] < 128;
        Ok(b.iter()
            .map(|_| scripted_recognition("корешок", if up { self.hit } else { self.miss }))
            .collect())
    }
}

fn marked(w: u32, h: u32, at: (u32, u32)) -> Image {
    let mut img = Image::from_pixel(w, h, image::Rgb([255; 3]));
    for y in at.1..at.1 + 10 {
        for x in at.0..at.0 + 10 {
            img.put_pixel(x, y, image::Rgb([0; 3]));
        }
    }
    img
}

#[test]
fn vertical_text_on_an_upright_page_is_not_rotated() {
    // ≥ 60 % vertical boxes (axis labels / spines) but 0° reads best: keep 0° (I9).
    let params = no_blank_no_deskew();
    let e = Spines {
        hit: 0.9,
        miss: 0.85,
    };
    let img = marked(900, 500, (0, 0));
    assert!(
        e.detect(&img).unwrap().iter().all(TextBox::is_vertical),
        "the 90/270 branch is exercised"
    );
    assert_eq!(ocr_image(&e, &img, &params).unwrap().rotation, 0);
}

#[test]
fn page_turned_by_90_is_rotated_when_90_wins_by_the_margin() {
    // Portrait raster with the marker bottom-left: imageops::rotate90 (clockwise) brings it to the top-left.
    let params = no_blank_no_deskew();
    let e = Spines {
        hit: 0.95,
        miss: 0.6,
    };
    let p = ocr_image(&e, &marked(500, 900, (0, 890)), &params).unwrap();
    assert_eq!(p.rotation, 90);
    assert!(p.mean_conf > 0.9);
    assert_eq!(
        (p.width, p.height),
        (900, 500),
        "boxes live in the turned raster"
    );
    assert_eq!((p.frame.src_w, p.frame.src_h), (500, 900));
}

#[test]
fn to_source_inverts_the_real_alignment() {
    // A dot in the source; align it with the real rasters (skew, then 90/180/270) and map its
    // aligned centroid back: it must land on the dot.
    let (w, h) = (300u32, 200u32);
    let dot = (210.0f32, 60.0f32);
    let src = marked(w, h, (dot.0 as u32 - 5, dot.1 as u32 - 5));
    for skew in [-1.5f32, 0.0, 1.5] {
        let deskewed = raster::rotate(&src, skew, [255; 3]);
        for rotation in [0u32, 90, 180, 270] {
            let aligned = match rotation {
                90 => image::imageops::rotate90(&deskewed),
                180 => image::imageops::rotate180(&deskewed),
                270 => image::imageops::rotate270(&deskewed),
                _ => deskewed.clone(),
            };
            let ink: Vec<(f32, f32)> = aligned
                .enumerate_pixels()
                .filter(|p| p.2[0] < 128)
                .map(|(x, y, _)| (x as f32 + 0.5, y as f32 + 0.5))
                .collect();
            let n = ink.len() as f32;
            let (cx, cy) = ink
                .iter()
                .fold((0.0, 0.0), |a, p| (a.0 + p.0 / n, a.1 + p.1 / n));
            let frame = AlignFrame {
                rotation,
                skew_deg: skew,
                src_w: w,
                src_h: h,
            };
            let r = frame.to_source([cx, cy, cx, cy]);
            let back = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
            assert!(
                (back.0 - dot.0).abs() < 1.0 && (back.1 - dot.1).abs() < 1.0,
                "skew {skew} rotation {rotation}: {back:?} vs {dot:?}"
            );
        }
    }
}

/// Counts `detect` and `detect_at` calls; one box per page.
struct Spy(
    std::sync::atomic::AtomicUsize,
    std::sync::atomic::AtomicUsize,
);
impl OcrEngine for Spy {
    fn detect(&self, _img: &Image) -> Result<Vec<TextBox>, OcrError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(vec![TextBox::from_aabb(20.0, 20.0, 80.0, 40.0, 0.9)])
    }
    fn detect_at(&self, _img: &Image, max_side: u32) -> Result<Vec<TextBox>, OcrError> {
        assert_eq!(max_side, R12_DET_MAX_SIDE);
        self.1.fetch_add(1, Ordering::Relaxed);
        Ok(vec![TextBox::from_aabb(20.0, 20.0, 80.0, 40.0, 0.9)])
    }
    fn recognize(&self, _img: &Image, b: &[TextBox]) -> Result<Vec<Recognized>, OcrError> {
        Ok(b.iter()
            .map(|_| scripted_recognition("текст", 0.9))
            .collect())
    }
}

fn spy() -> Spy {
    Spy(Default::default(), Default::default())
}

/// White page with a ruled grid: `h` horizontal rules `len` px long, `v` vertical ones.
fn ruled(h: u32, v: u32, len: u32) -> Image {
    let mut img = Image::from_pixel(700, 700, image::Rgb([255, 255, 255]));
    for i in 0..h {
        for x in 100..100 + len {
            for t in 0..2 {
                img.put_pixel(x, 200 + 100 * i + t, image::Rgb([0, 0, 0]));
            }
        }
    }
    for i in 0..v {
        for y in 100..100 + len {
            for t in 0..2 {
                img.put_pixel(150 + 150 * i + t, y, image::Rgb([0, 0, 0]));
            }
        }
    }
    img
}

fn with_lines() -> OcrParams {
    OcrParams {
        table_lines: Some(LinesParams::default()),
        ..no_blank_no_deskew()
    }
}

#[test]
fn ruled_page_gets_segments_and_full_resolution_det() {
    let e = spy();
    let page = ocr_image(&e, &ruled(3, 3, 400), &with_lines()).unwrap();
    assert!(!page.segments.is_empty());
    assert!(page.full_res_det);
    assert_eq!(e.1.load(Ordering::Relaxed), 1);
    assert_eq!(e.0.load(Ordering::Relaxed), 0);
}

#[test]
fn without_table_lines_nothing_changes() {
    let e = spy();
    let page = ocr_image(&e, &ruled(3, 3, 400), &no_blank_no_deskew()).unwrap();
    assert!(page.segments.is_empty());
    assert!(!page.full_res_det);
    assert_eq!(e.1.load(Ordering::Relaxed), 0);
    assert_eq!(e.0.load(Ordering::Relaxed), 1);
}

#[test]
fn short_rules_do_not_trigger_full_resolution_det() {
    let e = spy();
    // 150 px rules are extracted but below the 200 px horizontal threshold of R-12.
    let page = ocr_image(&e, &ruled(3, 3, 150), &with_lines()).unwrap();
    assert!(!page.segments.is_empty());
    assert!(!page.full_res_det);
    assert_eq!(e.1.load(Ordering::Relaxed), 0);
    assert_eq!(e.0.load(Ordering::Relaxed), 1);
}

/// Reads "upright" by a dark marker at the top-left; records the marker state of each `detect_at`.
struct TurnSpy(std::sync::Mutex<Vec<bool>>);
impl OcrEngine for TurnSpy {
    fn detect(&self, _img: &Image) -> Result<Vec<TextBox>, OcrError> {
        Ok(vec![TextBox::from_aabb(20.0, 20.0, 80.0, 40.0, 0.9)])
    }
    fn detect_at(&self, img: &Image, max_side: u32) -> Result<Vec<TextBox>, OcrError> {
        assert_eq!(max_side, R12_DET_MAX_SIDE);
        self.0.lock().unwrap().push(img.get_pixel(5, 5)[0] < 128);
        self.detect(img)
    }
    fn recognize(&self, img: &Image, b: &[TextBox]) -> Result<Vec<Recognized>, OcrError> {
        let up = img.get_pixel(5, 5)[0] < 128;
        Ok(b.iter()
            .map(|_| scripted_recognition("текст", if up { 0.95 } else { 0.4 }))
            .collect())
    }
}

#[test]
fn turned_ruled_page_gets_full_resolution_det_and_segments_in_the_turned_frame() {
    let mut img = ruled(2, 2, 400); // horizontals at y=200,300 (x 100..500)
    for y in 690..700 {
        for x in 690..700 {
            img.put_pixel(x, y, image::Rgb([0; 3]));
        }
    } // marker bottom-right = upside down
    let e = TurnSpy(Default::default());
    let page = ocr_image(&e, &img, &with_lines()).unwrap();
    assert_eq!(page.rotation, 180);
    assert!(page.full_res_det);
    // First R-12 det on the upside-down raster, the second on the turned one.
    assert_eq!(*e.0.lock().unwrap(), vec![false, true]);
    let h: Vec<_> = page.segments.iter().filter(|g| g.horizontal).collect();
    assert_eq!(h.len(), 2);
    // y 200/300 -> about 499/399, x 100..500 -> about 199..599
    assert!(h.iter().all(|g| g.at > 390.0 && (g.a - 199.0).abs() < 4.0));
}
