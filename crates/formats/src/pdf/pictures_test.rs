use super::*;
use plaindoc_core::{BBox, Level, Warning, WarningKind};

#[test]
fn service_scan_bounds() {
    assert!(service_scan(SERVICE_COVER, SERVICE_TEXT - 1));
    assert!(!service_scan(SERVICE_COVER, SERVICE_TEXT));
    assert!(!service_scan(SERVICE_COVER - 0.001, 0));
    assert!(service_scan(1.0, 0));
}

#[test]
fn which_pages_take_pictures() {
    use PageDecision::*;
    let t = |d, c, n| takes_pictures(d, c, n, SCAN_COVER);
    assert!(t(UseTextLayer, 0.5, 0), "text page: no text-volume rule");
    assert!(t(UseTextLayer, SCAN_COVER - 0.001, 0));
    assert!(!t(UseTextLayer, SCAN_COVER, 5000), "scan or background");
    assert!(!t(UseTextLayer, 0.0, 5000), "no images");
    assert!(t(RunOcrAndMerge, 0.34, 320), "brochure page 1");
    assert!(!t(RunOcrAndMerge, 0.7, 100), "scan with service text");
    assert!(t(RunOcrAndMerge, 0.7, SERVICE_TEXT));
    assert!(!t(RunOcr, 0.3, 500));
    assert!(!t(Reject, 0.3, 500));
    assert!(
        !takes_pictures(UseTextLayer, 0.6, 0, 0.6),
        "the classifier's scan cover"
    );
}

fn cand(key: u64, band: bool, small: bool) -> Candidate {
    Candidate {
        key,
        decode_key: key,
        paint_order: 0,
        bbox: BBox::new(0.0, 0.0, 100.0, 100.0),
        band,
        small,
    }
}

fn page(c: Vec<Candidate>) -> PagePictures {
    PagePictures {
        cands: c,
        ocr_skipped: 0,
        lowered: false,
    }
}

#[test]
fn running_repeats_need_three_pages_in_the_band() {
    let pages = [
        page(vec![cand(1, true, true), cand(2, false, false)]),
        page(vec![cand(1, true, false), cand(2, false, false)]),
        page(vec![
            cand(1, true, true),
            cand(2, false, false),
            cand(1, true, true),
        ]),
    ];
    let r = running_repeats(pages.iter(), 3);
    assert_eq!(r, HashSet::from([1]), "out of the band never repeats");
    assert!(
        running_repeats(pages.iter(), 2).is_empty(),
        "short document"
    );
    assert!(
        running_repeats(pages[..2].iter(), 3).is_empty(),
        "two pages"
    );
}

fn diag(level: Level) -> PageDiagnosis {
    PageDiagnosis {
        warnings: vec![Warning::new(WarningKind::ImageTextPage, level, "hint")],
        decision: PageDecision::RunOcrAndMerge,
        quality: 1.0,
    }
}

#[test]
fn skip_image_ocr_only_on_merge_pages_with_pictures() {
    let mut d = diag(Level::Warn);
    let mut p = page(vec![cand(1, false, false), cand(2, false, true)]);
    skip_image_ocr(&mut d, PageDecision::RunOcrAndMerge, &mut p);
    assert_eq!(d.decision, PageDecision::UseTextLayer);
    assert_eq!(d.warnings[0].level, Level::Info);
    assert_eq!(p.ocr_skipped, 1, "decoration is not counted");
    // decoration only: OCR stays
    let mut d = diag(Level::Warn);
    let mut p = page(vec![cand(2, false, true)]);
    skip_image_ocr(&mut d, PageDecision::RunOcrAndMerge, &mut p);
    assert_eq!(d.decision, PageDecision::RunOcrAndMerge);
    assert_eq!((d.warnings[0].level, p.ocr_skipped), (Level::Warn, 0));
    // the SP2 table's merge is not acted on (`ocr_needs` gives `UseTextLayer`)
    let mut d = diag(Level::Info);
    let mut p = page(vec![cand(1, false, false)]);
    skip_image_ocr(&mut d, PageDecision::UseTextLayer, &mut p);
    assert_eq!(
        (d.decision, p.ocr_skipped),
        (PageDecision::RunOcrAndMerge, 0)
    );
}

#[test]
fn plate_range_bound() {
    let g = |lo: u8, hi: u8| {
        image::DynamicImage::ImageLuma8(image::GrayImage::from_fn(4, 2, |x, _| {
            image::Luma([if x % 2 == 0 { lo } else { hi }])
        }))
    };
    assert!(plate(&g(10, 10 + PLATE_RANGE)));
    assert!(!plate(&g(10, 11 + PLATE_RANGE)));
    // white logo: colour flat, alpha shaped
    let logo = image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(4, 2, |x, _| {
        image::Rgba([255, 255, 255, if x == 0 { 0 } else { 255 }])
    }));
    assert!(!plate(&logo));
}

#[test]
fn place_by_top_marks_drops_and_warns() {
    let para = |y: f32| Block::paragraph("t", Some(BBox::new(0.0, y, 300.0, 20.0)));
    let mut blocks = vec![para(700.0), para(300.0)];
    let mut warnings = Vec::new();
    let mut a = cand(1, false, false);
    a.bbox = BBox::new(0.0, 400.0, 200.0, 200.0);
    let mut pg = page(vec![
        a,
        cand(2, true, false),
        cand(3, false, true),
        cand(4, false, false),
    ]);
    pg.ocr_skipped = 3;
    let repeats = HashSet::from([2]);
    let decoded = HashMap::from([(1, Pic::Data(Arc::from(&b"\x89PNG"[..]))), (4, Pic::Plate)]);
    let n = place(&mut blocks, &mut warnings, &pg, &repeats, Some(&decoded));
    assert_eq!(
        n,
        Placed {
            linked: 1,
            dropped: 3
        }
    );
    assert!(
        matches!(&blocks[1], Block::Image { image: Some(i), .. } if i.pending_data().is_some())
    );
    assert_eq!(blocks.len(), 3);
    assert_eq!(
        warnings[0].message,
        "3 image(s) not recognised: OCR is skipped on a page with a text layer (1 kept as pictures)"
    );
    // pictures off: a bare mark, no decoding needed
    let mut blocks = Vec::new();
    let pg = page(vec![cand(5, false, false)]);
    let n = place(&mut blocks, &mut Vec::new(), &pg, &HashSet::new(), None);
    assert_eq!(n.linked, 1);
    assert!(matches!(&blocks[0], Block::Image { image: None, .. }));
}
