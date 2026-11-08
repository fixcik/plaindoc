//! The engine contract and a scripted engine for tests.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{Image, Recognized, TextBox};

/// OCR failures. Engines never panic on bad input; the caller turns these into warnings.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    /// Models are missing or fail their checksum; OCR is unavailable, not the document broken.
    #[error("OCR models unavailable: {0}")]
    ModelsUnavailable(String),
    #[error("OCR engine error: {0}")]
    Engine(String),
    #[error("image decode error: {0}")]
    Decode(String),
}

/// Text detector + recognizer (spec section 6). Implementations are shared across threads.
pub trait OcrEngine: Send + Sync {
    /// Line boxes in `img` pixels, in a deterministic order.
    fn detect(&self, img: &Image) -> Result<Vec<TextBox>, OcrError>;
    /// One result per box, same order as `boxes`.
    fn recognize(&self, img: &Image, boxes: &[TextBox]) -> Result<Vec<Recognized>, OcrError>;
    /// Detection with the long side capped at `max_side`; engines without scaling ignore it.
    fn detect_at(&self, img: &Image, _max_side: u32) -> Result<Vec<TextBox>, OcrError> {
        self.detect(img)
    }
    /// How many calls the engine runs at once.
    fn max_concurrency(&self) -> usize {
        usize::MAX
    }
}

/// Engine that returns a fixed script: every `detect` yields `boxes`, `recognize` maps a box to the
/// scripted text by equality. Counts calls so tests can assert OCR did or did not run.
#[derive(Default)]
pub struct FakeEngine {
    items: Vec<(TextBox, String, f32)>,
    fail: bool,
    detect_calls: AtomicUsize,
}

impl FakeEngine {
    /// Each item: box, text, confidence. CTC steps are spread evenly over the text.
    pub fn new(items: Vec<(TextBox, String, f32)>) -> Self {
        Self {
            items,
            ..Default::default()
        }
    }

    /// Engine whose every call fails with `OcrError::Engine`.
    pub fn failing() -> Self {
        Self {
            fail: true,
            ..Default::default()
        }
    }

    pub fn detect_calls(&self) -> usize {
        self.detect_calls.load(Ordering::Relaxed)
    }
}

/// Recognition of `text` with evenly spaced steps over `T = 4 * chars` (crop fills the input).
pub fn scripted_recognition(text: &str, conf: f32) -> Recognized {
    let n = text.chars().count() as u32;
    Recognized {
        text: text.to_string(),
        conf: if n == 0 { 0.0 } else { conf },
        steps: (0..n).map(|i| i * 4 + 1).collect(),
        total_steps: (n * 4).max(1),
        width_frac: 1.0,
        flipped: false,
    }
}

impl OcrEngine for FakeEngine {
    fn detect(&self, _img: &Image) -> Result<Vec<TextBox>, OcrError> {
        self.detect_calls.fetch_add(1, Ordering::Relaxed);
        if self.fail {
            return Err(OcrError::Engine("fake failure".into()));
        }
        Ok(self.items.iter().map(|i| i.0).collect())
    }

    fn recognize(&self, _img: &Image, boxes: &[TextBox]) -> Result<Vec<Recognized>, OcrError> {
        if self.fail {
            return Err(OcrError::Engine("fake failure".into()));
        }
        Ok(boxes
            .iter()
            .map(|b| match self.items.iter().find(|i| i.0 == *b) {
                Some((_, t, c)) => scripted_recognition(t, *c),
                None => Recognized::default(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_engine_scripts_boxes_and_counts_calls() {
        let b = TextBox::from_aabb(1.0, 2.0, 30.0, 12.0, 0.9);
        let e = FakeEngine::new(vec![(b, "Акт".into(), 0.95)]);
        let img = Image::new(40, 20);
        assert_eq!(e.detect(&img).unwrap(), vec![b]);
        let r = e
            .recognize(&img, &[b, TextBox::from_aabb(0.0, 0.0, 1.0, 1.0, 1.0)])
            .unwrap();
        assert_eq!(
            (r[0].text.as_str(), r[0].conf, r[0].steps.clone()),
            ("Акт", 0.95, vec![1, 5, 9])
        );
        assert_eq!(r[1], Recognized::default(), "unknown box");
        assert_eq!(e.detect_calls(), 1);
        assert!(matches!(
            FakeEngine::failing().detect(&img),
            Err(OcrError::Engine(_))
        ));
        assert_eq!(b.aabb(), [1.0, 2.0, 30.0, 12.0]);
        assert!(!b.is_vertical());
    }
}
