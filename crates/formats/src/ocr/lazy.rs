//! Default PaddleOCR engine, created on first use so a parse without OCR pages never loads models.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use plaindoc_ocr::models::resolve_models_dir;
use plaindoc_ocr::{Image, OcrEngine, OcrError, PaddleConfig, PaddleEngine, Recognized, TextBox};

struct LazyPaddle {
    dir: Option<PathBuf>,
    cfg: PaddleConfig,
    engine: OnceLock<Result<PaddleEngine, String>>,
}

impl LazyPaddle {
    fn get(&self) -> Result<&PaddleEngine, OcrError> {
        self.engine
            .get_or_init(|| {
                let dir = resolve_models_dir(self.dir.as_deref());
                PaddleEngine::new(dir.as_deref(), self.cfg.clone()).map_err(|e| match e {
                    OcrError::ModelsUnavailable(m) => m,
                    e => e.to_string(),
                })
            })
            .as_ref()
            .map_err(|e| OcrError::ModelsUnavailable(e.clone()))
    }
}

impl OcrEngine for LazyPaddle {
    fn detect(&self, img: &Image) -> Result<Vec<TextBox>, OcrError> {
        self.get()?.detect(img)
    }
    fn detect_at(&self, img: &Image, max_side: u32) -> Result<Vec<TextBox>, OcrError> {
        self.get()?.detect_at(img, max_side)
    }
    fn recognize(&self, img: &Image, boxes: &[TextBox]) -> Result<Vec<Recognized>, OcrError> {
        self.get()?.recognize(img, boxes)
    }
    fn max_concurrency(&self) -> usize {
        self.cfg.max_sessions.max(1)
    }
}

/// PaddleOCR engine with models from `models_dir` (else env / cache dir) and at most
/// `max_sessions` concurrent session pairs (default: min(cores, 4); budget ~0.7 GB each on a mixed
/// stream). A model resolution error is cached as `ModelsUnavailable` for the life of the engine
/// (until restart): a long-running service whose models are installed later needs a new engine,
/// i.e. a new `Registry`.
pub fn paddle_engine(
    models_dir: Option<PathBuf>,
    max_sessions: Option<usize>,
) -> Arc<dyn OcrEngine> {
    let mut cfg = PaddleConfig::default(); // max_sessions: min(cores, 4)
    if let Some(n) = max_sessions {
        cfg.max_sessions = n;
    }
    Arc::new(LazyPaddle {
        dir: models_dir,
        cfg,
        engine: OnceLock::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use plaindoc_ocr::models::{DET_V5_MOBILE, bundled};

    #[test]
    fn models_missing_is_unavailable_not_failure() {
        if bundled(&DET_V5_MOBILE).is_some() {
            return; // bundled build: the directory is never consulted
        }
        let d = tempfile::tempdir().unwrap();
        let e = paddle_engine(Some(d.path().to_path_buf()), Some(1));
        let img = Image::new(10, 10);
        assert!(
            matches!(e.detect(&img), Err(OcrError::ModelsUnavailable(m)) if m.contains("PP-OCRv5_mobile_det"))
        );
        assert!(
            matches!(e.detect(&img), Err(OcrError::ModelsUnavailable(_))),
            "error is cached until restart"
        );
    }
}
