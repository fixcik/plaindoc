//! An engine whose models are missing: `OcrUnavailable` is a `Warn` also under `Auto`.

use super::*;

#[test]
fn engine_without_models_is_a_warn_also_under_auto() {
    use plaindoc_ocr::{Image, OcrEngine, OcrError, Recognized};
    struct NoModels;
    impl OcrEngine for NoModels {
        fn detect(&self, _: &Image) -> Result<Vec<TextBox>, OcrError> {
            Err(OcrError::ModelsUnavailable("OCR models are missing".into()))
        }
        fn recognize(&self, _: &Image, _: &[TextBox]) -> Result<Vec<Recognized>, OcrError> {
            unreachable!()
        }
    }
    let ir = slide(vec![image_with_data(png_with_bar())]);
    let d = run(
        OcrConfig::with_engine(Arc::new(NoModels)),
        OcrMode::Auto,
        &ir,
    );
    let w = d
        .warnings
        .iter()
        .find(|w| w.kind == WarningKind::OcrUnavailable);
    assert_eq!(w.map(|w| w.level), Some(Level::Warn));
    assert!(w.unwrap().message.contains("OCR models are missing"));
}
