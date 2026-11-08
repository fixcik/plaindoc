use super::*;
use crate::pdf::open::open_input;
use crate::pdf::tables::DetectedTable;
use plaindoc_core::Cell;
use plaindoc_core::Input;
use plaindoc_ocr::lines::Segment;
use plaindoc_ocr::{AlignFrame, OcrEngine, OcrWord, Recognized, TextBox, scripted_recognition};

/// One-page PDF, MediaBox [10 20 622 812], /Rotate 90, "Chapter 0" in Helvetica 12 at (72, 720).
pub(crate) fn rotated_offset_pdf() -> Vec<u8> {
    pdf_with_page_keys("")
}

/// As `rotated_offset_pdf`, plus `extra` in the page dictionary.
fn pdf_with_page_keys(extra: &str) -> Vec<u8> {
    let content = "BT /F1 12 Tf 72 720 Td (Chapter 0) Tj ET";
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [10 20 622 812] /Rotate 90 {extra} \
             /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
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

/// AABB of dark pixels.
pub(crate) fn ink_box(img: &Image) -> Option<[f32; 4]> {
    let mut b: Option<[u32; 4]> = None;
    for (x, y, p) in img.enumerate_pixels() {
        if p[0] < 128 {
            let r = b.get_or_insert([x, y, x, y]);
            *r = [r[0].min(x), r[1].min(y), r[2].max(x + 1), r[3].max(y + 1)];
        }
    }
    b.map(|r| r.map(|v| v as f32))
}

/// Engine that finds the rendered text itself (box = ink of the raster) plus one scripted box
/// elsewhere, so the duplicate box does not go through `PageGeom` (I3: a wrong 90° mapping
/// would move both alike).
pub(crate) struct InkSpy;
impl OcrEngine for InkSpy {
    fn detect(&self, img: &Image) -> Result<Vec<TextBox>, OcrError> {
        let [x0, y0, x1, y1] = ink_box(img).expect("rendered text");
        Ok(vec![
            TextBox::from_aabb(x0 - 2.0, y0 - 2.0, x1 + 2.0, y1 + 2.0, 0.9),
            TextBox::from_aabb(100.0, 100.0, 400.0, 140.0, 0.9),
        ])
    }
    fn recognize(&self, _img: &Image, b: &[TextBox]) -> Result<Vec<Recognized>, OcrError> {
        Ok(b.iter()
            .map(|t| {
                let text = if t.aabb()[0] == 100.0 {
                    "stamp text"
                } else {
                    "Chapter 0"
                };
                scripted_recognition(text, 0.9)
            })
            .collect())
    }
}

#[test]
fn overlapping_regions_are_merged_and_too_many_mean_full_page() {
    let page = (1000, 1000);
    let r = merge_regions(
        vec![
            [100, 100, 300, 200],
            [250, 150, 400, 260],
            [600, 600, 700, 700],
        ],
        page,
    );
    assert_eq!(r, vec![[100, 100, 400, 260], [600, 600, 700, 700]]);
    let many: Vec<_> = (0..20).map(|i| [i * 40, 0, i * 40 + 20, 20]).collect();
    assert!(
        merge_regions(many, page).is_empty(),
        "20 separate regions -> full page"
    );
    assert!(
        merge_regions(vec![[0, 0, 900, 900]], page).is_empty(),
        "81 % of the page"
    );
}

#[test]
fn rotated_offset_pdf_renders_rotated_and_spans_hit_the_ink() {
    let o = open_input(Input::Bytes(rotated_offset_pdf()), &ParseOptions::default()).unwrap();
    let r = render(&o.doc, 0, 72).unwrap();
    assert_eq!(r.geom.rotation, 90);
    assert_eq!(r.img.dimensions(), (792, 612), "visible page is landscape");
    let ink = ink_box(&r.img).expect("text rendered");
    let spans = span_boxes_px(&o.doc, 0, &r.geom).unwrap();
    let (cx, cy) = ((ink[0] + ink[2]) / 2.0, (ink[1] + ink[3]) / 2.0);
    assert!(
        spans
            .iter()
            .any(|t| t[0] <= cx && cx <= t[2] && t[1] <= cy && cy <= t[3]),
        "span box covers the rendered text: ink {ink:?} spans {spans:?}"
    );
}

#[test]
fn cropped_page_spans_hit_the_ink() {
    // The renderer draws CropBox ∩ MediaBox = [50 400 400 812]; rotated: 412 x 350 pt.
    let pdf = pdf_with_page_keys("/CropBox [50 400 400 900]");
    let o = open_input(Input::Bytes(pdf), &ParseOptions::default()).unwrap();
    let r = render(&o.doc, 0, 72).unwrap();
    assert_eq!(r.img.dimensions(), (412, 350));
    let ink = ink_box(&r.img).expect("text rendered");
    let spans = span_boxes_px(&o.doc, 0, &r.geom).unwrap();
    let (cx, cy) = ((ink[0] + ink[2]) / 2.0, (ink[1] + ink[3]) / 2.0);
    assert!(
        spans
            .iter()
            .any(|t| t[0] <= cx && cx <= t[2] && t[1] <= cy && cy <= t[3]),
        "span box covers the rendered text: ink {ink:?} spans {spans:?}"
    );
    assert_eq!(ink_signals(&o.doc, 0).unwrap().glyph_cc_outside, 0);
}

#[test]
fn span_extraction_error_is_an_error_not_an_empty_layer() {
    // An empty layer would disable dedup on the merge path and duplicate the layer's text.
    let geom = PageGeom::image(100, 100);
    let r = layer_boxes(Err(&pdf_oxide::Error::InvalidHeader("x".into())), &geom);
    assert!(r.is_err(), "{r:?}");
    assert_eq!(layer_boxes(Ok(&[]), &geom).unwrap(), Vec::<[f32; 4]>::new());
}

#[test]
fn scan_inputs_are_points_in_the_aligned_frame_with_page_size() {
    let mut page = OcrPage {
        width: 1000,
        height: 2000,
        ..Default::default()
    };
    page.words.push(OcrWord {
        text: "Итого".into(),
        bbox: [100.0, 100.0, 200.0, 130.0],
        conf: 0.9,
        line: 0,
        block: 0,
        angle: 0.0,
        font_px: 30.0,
        char_x: vec![(100.0, 120.0), (120.0, 200.0)],
    });
    page.segments.push(Segment {
        horizontal: true,
        a: 10.0,
        b: 500.0,
        at: 300.0,
        width: 2.0,
    });
    page.segments.push(Segment {
        horizontal: false,
        a: 100.0,
        b: 700.0,
        at: 40.0,
        width: 2.0,
    });
    let i = scan_inputs(&page, 3, 2.0);
    assert_eq!(
        (i.page_index, i.page_width, i.page_height),
        (3, 500.0, 1000.0)
    );
    let b = i.words[0].bbox;
    assert_eq!(
        (b.x, b.y, b.w, b.h, i.words[0].font_size),
        (50.0, 935.0, 50.0, 15.0, 15.0)
    );
    assert_eq!(i.words[0].char_x, vec![(50.0, 60.0), (60.0, 100.0)]);
    assert_eq!(
        i.segments,
        vec![
            ScanSegment {
                x1: 5.0,
                y1: 850.0,
                x2: 250.0,
                y2: 850.0
            },
            ScanSegment {
                x1: 20.0,
                y1: 950.0,
                x2: 20.0,
                y2: 650.0
            }
        ]
    );
}

#[test]
fn nested_table_bboxes_are_mapped_too() {
    let inner = Block::Table {
        rows: vec![vec![Cell::new("b".to_string())]],
        header_rows: 0,
        bbox: Some(BBox::new(1.0, 1.0, 1.0, 1.0)),
    };
    let mut cell = Cell::new("a".to_string());
    cell.blocks.push(inner);
    let mut t = Block::Table {
        rows: vec![vec![cell]],
        header_rows: 0,
        bbox: Some(BBox::new(0.0, 0.0, 9.0, 9.0)),
    };
    remap_table_bboxes(&mut t, &|b| BBox::new(b.x + 100.0, b.y, b.w, b.h));
    let Block::Table {
        rows,
        bbox: Some(o),
        ..
    } = &t
    else {
        unreachable!()
    };
    let Some(Block::Table { bbox: Some(n), .. }) = rows[0][0]
        .blocks
        .iter()
        .find(|b| matches!(b, Block::Table { .. }))
    else {
        panic!()
    };
    assert_eq!((o.x, n.x), (100.0, 101.0));
}

#[test]
fn table_bbox_maps_back_to_user_space_on_a_rotated_skewed_page() {
    // Expected values are computed by hand from the px_to_pt formulas, not through the code under
    // test. Table in the aligned visible frame: x 100..300, y 300..400 pt (y up).
    let t = BBox::new(100.0, 300.0, 200.0, 100.0);
    let page_for = |g: &PageGeom, skew: f32| {
        let (w, h) = g.image_size();
        OcrPage {
            width: w,
            height: h,
            frame: AlignFrame {
                rotation: 0,
                skew_deg: skew,
                src_w: w,
                src_h: h,
            },
            ..Default::default()
        }
    };
    let close = |a: BBox, e: (f32, f32, f32, f32)| {
        (a.x - e.0).abs() < 0.5
            && (a.y - e.1).abs() < 0.5
            && (a.w - e.2).abs() < 0.5
            && (a.h - e.3).abs() < 0.5
    };
    // 1) no /Rotate, MediaBox at 0, no skew: identity.
    let g0 = PageGeom {
        media: [0.0, 0.0, 612.0, 792.0],
        rotation: 0,
        scale: 2.0,
    };
    let u = table_bbox_to_user(&t, &page_for(&g0, 0.0), &g0);
    assert!(close(u, (100.0, 300.0, 200.0, 100.0)), "{u:?}");
    // 2) /Rotate 90, MediaBox [10 20 622 812] (612x792): the visible page is 792 x 612 pt. A visible
    //    point (X, Y) is raster (u, v) = (X, 612 - Y) pt, and px_to_pt(rot 90) = (mx + v, my + u):
    //    user x = 10 + 612 - Y in [222, 322], user y = 20 + X in [120, 320].
    let g90 = PageGeom {
        media: [10.0, 20.0, 622.0, 812.0],
        rotation: 90,
        scale: 200.0 / 72.0,
    };
    let u = table_bbox_to_user(&t, &page_for(&g90, 0.0), &g90);
    assert!(close(u, (222.0, 120.0, 100.0, 200.0)), "{u:?}");
    // 3) + 1° deskew: the aligned raster is the source rotated by `skew_deg` about the raster centre,
    //    so the source position of the table centre is the aligned centre rotated back by -1°.
    let p = page_for(&g90, 1.0);
    let u = table_bbox_to_user(&t, &p, &g90);
    let s = g90.scale;
    let (cx, cy) = (200.0 * s, (612.0 - 350.0) * s); // aligned centre, px
    let (ox, oy) = (p.width as f32 / 2.0, p.height as f32 / 2.0);
    // aligned = M(+skew) * source, so source = M(-skew) * aligned:
    // x = ox + (x'-ox)cos(skew) - (y'-oy)sin(skew), y = oy + (x'-ox)sin(skew) + (y'-oy)cos(skew).
    let a = p.frame.skew_deg.to_radians(); // +1 deg
    let (sx, sy) = (
        ox + (cx - ox) * a.cos() - (cy - oy) * a.sin(),
        oy + (cx - ox) * a.sin() + (cy - oy) * a.cos(),
    );
    let (ex, ey) = (10.0 + sy / s, 20.0 + sx / s); // px_to_pt(rot 90), by hand
    assert!(
        (u.x + u.w / 2.0 - ex).abs() < 1.0 && (u.y + u.h / 2.0 - ey).abs() < 1.0,
        "{u:?} vs ({ex}, {ey})"
    );
}

#[test]
fn tables_are_placed_in_aligned_reading_order_on_a_rotated_page() {
    // /Rotate 90: user y follows the aligned x, so user-space placement misorders stacked blocks.
    let g = PageGeom {
        media: [0.0, 0.0, 612.0, 792.0],
        rotation: 90,
        scale: 1.0,
    };
    let word = |text: &str, bbox: [f32; 4], line: u32| OcrWord {
        text: text.into(),
        bbox,
        conf: 0.9,
        line,
        block: line,
        angle: 0.0,
        font_px: 20.0,
        char_x: vec![],
    };
    let line = |block: u32| plaindoc_ocr::OcrLine {
        text: String::new(),
        bbox: [0.0; 4],
        conf: 0.9,
        block,
    };
    let page = OcrPage {
        width: 792,
        height: 612,
        words: vec![
            word("above", [100.0, 50.0, 300.0, 80.0], 0),
            word("below", [100.0, 400.0, 700.0, 430.0], 1),
        ],
        lines: vec![line(0), line(1)],
        ..Default::default()
    };
    let a = [100.0, 200.0, 700.0, 300.0];
    let table = || {
        let b = g.bbox(a);
        DetectedTable::new(
            b,
            Block::Table {
                rows: vec![vec![Cell::new("t")]],
                header_rows: 0,
                bbox: Some(b),
            },
        )
    };
    fn order(bs: &[Block]) -> Vec<&str> {
        bs.iter()
            .map(|b| match b {
                Block::Paragraph { text, .. } => text.as_str(),
                _ => "table",
            })
            .collect()
    }
    // The user-space placement (SP4 `place_tables`) gets it wrong here: the table goes last.
    let mut user = crate::ocr::ocr_blocks(&page, &g, (0.0, 0.0), &|_| true);
    super::super::table_place::place_tables(&mut user, &[table()]);
    assert_eq!(order(&user), ["above", "below", "table"]);
    let mut blocks = ocr_blocks_aligned(&page, &g, (0.0, 0.0), &|_| true);
    place_tables_aligned(&mut blocks, vec![(table().block, a)], g.scale);
    let blocks: Vec<Block> = blocks.into_iter().map(|(b, _)| b).collect();
    assert_eq!(order(&blocks), ["above", "table", "below"]);
}

#[test]
fn ocr_empty_page_gets_no_scan_tables() {
    struct Count(std::sync::atomic::AtomicUsize);
    impl TableDetector for Count {
        fn detect(
            &self,
            _: &PdfDocument,
            _: &pdf_oxide::structured::StructuredPage,
        ) -> plaindoc_core::Result<Vec<DetectedTable>> {
            Ok(vec![])
        }
        fn detect_scan(&self, _: &ScanTableInputs) -> plaindoc_core::Result<Vec<DetectedTable>> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Err(plaindoc_core::ParseError::Internal("boom".into()))
        }
    }
    let scan = include_bytes!("../../tests/fixtures/pdf/scan.pdf").to_vec();
    let o = open_input(Input::Bytes(scan), &ParseOptions::default()).unwrap();
    let run = |text: &str| {
        let e = std::sync::Arc::new(plaindoc_ocr::FakeEngine::new(vec![(
            TextBox::from_aabb(100.0, 100.0, 900.0, 160.0, 0.9),
            text.to_string(),
            0.92,
        )]));
        let det = Count(Default::default());
        let out = run_page_ocr(
            &o.doc,
            0,
            PageDecision::RunOcr,
            None,
            &OcrConfig::with_engine(e),
            &det,
            &ParseOptions::default(),
            &AtomicBool::new(false),
            &mut OcrPageCounts::default(),
        );
        let PageOcr::Done { warnings, .. } = out else {
            panic!("OCR not done")
        };
        let failed = warnings
            .iter()
            .any(|w| w.message.starts_with("table detection failed"));
        (det.0.into_inner(), failed)
    };
    assert_eq!(run("Акт"), (0, false), "OcrEmpty: no detection, no warning");
    assert_eq!(run("Акт приёмки"), (1, true));
}

#[test]
fn structured_spans_hit_the_ink_like_extracted_spans() {
    // the layer cross-check maps `extract_structured` spans as `span_boxes_px` maps
    // `extract_spans`
    for pdf in [
        rotated_offset_pdf(),
        pdf_with_page_keys("/CropBox [50 400 400 900]"),
    ] {
        let o = open_input(Input::Bytes(pdf), &ParseOptions::default()).unwrap();
        let r = render(&o.doc, 0, 72).unwrap();
        let ink = ink_box(&r.img).expect("text rendered");
        let page = o.doc.extract_structured(0).unwrap();
        let b = page.regions[0].spans[0].bbox;
        let s = r.geom.px_rect(b.x, b.y, b.width, b.height);
        let (cx, cy) = ((ink[0] + ink[2]) / 2.0, (ink[1] + ink[3]) / 2.0);
        assert!(
            s[0] <= cx && cx <= s[2] && s[1] <= cy && cy <= s[3],
            "{ink:?} {s:?}"
        );
    }
}

#[test]
fn ocr_of_image_regions_does_not_judge_the_layer() {
    // c10: an image of 16 % of the page on a page of text
    let pdf = include_bytes!("../../tests/fixtures/pdf/hidden/c10_text_over_image.pdf");
    let o = open_input(Input::Bytes(pdf.to_vec()), &ParseOptions::default()).unwrap();
    let page = o.doc.extract_structured(0).unwrap();
    let engine = plaindoc_ocr::FakeEngine::new(vec![(
        TextBox::from_aabb(250.0, 1000.0, 800.0, 1060.0, 0.9),
        "visible words".into(),
        0.9,
    )]);
    let mut cfg = OcrConfig::with_engine(std::sync::Arc::new(engine));
    cfg.params.blank_check = false;
    let opts = ParseOptions::default();
    let run = |d| {
        let mut c = OcrPageCounts::default();
        match run_page_ocr(
            &o.doc,
            0,
            d,
            Some(&page),
            &cfg,
            &crate::pdf::NoTables,
            &opts,
            &AtomicBool::new(false),
            &mut c,
        ) {
            PageOcr::Done { mismatch, .. } => mismatch.map(|m| (m.check.unread, m.check.counted)),
            _ => panic!("OCR ran"),
        }
    };
    // the whole page is judged (OCR reads none of the layer), OCR of the image only is not
    assert!(run(PageDecision::UseTextLayer).is_some_and(|(u, n)| u == n && n > 100));
    assert_eq!(run(PageDecision::RunOcrAndMerge), None);
}

#[test]
fn text_page_ink_is_masked_by_its_layer() {
    let o = open_input(Input::Bytes(rotated_offset_pdf()), &ParseOptions::default()).unwrap();
    let f = ink_signals(&o.doc, 0).expect("rendered");
    assert_eq!(f.glyph_cc_outside, 0, "{f:?}");
    assert!(f.glyph_cc_all > 0, "{f:?}");
}
