use plaindoc_ocr::{Image, OcrEngine, PaddleConfig, PaddleEngine, TextBox};
#[test]
#[ignore = "needs OCR models"]
fn paddle_engine_runs_and_is_deterministic() {
    let dir = plaindoc_ocr::resolve_models_dir(None);
    let e = PaddleEngine::new(dir.as_deref(), PaddleConfig::default()).unwrap();
    assert_eq!(
        e.max_concurrency(),
        PaddleConfig::default().max_sessions.max(1)
    );
    let img = Image::from_pixel(640, 200, image::Rgb([255; 3]));
    assert!(
        e.detect(&img).unwrap().is_empty(),
        "blank raster has no boxes"
    );
    let boxes = [TextBox::from_aabb(10.0, 10.0, 300.0, 50.0, 1.0)];
    let a = e.recognize(&img, &boxes).unwrap();
    let b = e.recognize(&img, &boxes).unwrap();
    assert_eq!(a, b);
    assert!(a[0].width_frac > 0.0 && a[0].total_steps > 0);
}
