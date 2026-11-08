//! OCR for plaindoc: the `OcrEngine` contract, shared types and a scripted test engine.

mod blank;
pub mod det;
mod engine;
pub mod fix;
pub mod layout;
pub mod lines;
pub mod models;
mod orient;
#[cfg(feature = "paddle")]
mod paddle;
mod pipeline;
#[cfg(any(test, feature = "paddle"))]
mod pool;
pub mod raster;
pub mod rec;
mod types;
pub mod words;

pub use blank::{BlankStats, blank_stats, is_blank};
pub use engine::{FakeEngine, OcrEngine, OcrError, scripted_recognition};
pub use models::{
    DET_V5_MOBILE, MANIFEST, MODELS_ENV, ModelFile, REC_CYRILLIC_V5_MOBILE, bundled,
    default_models_dir, load_model, resolve_models_dir,
};
#[cfg(feature = "paddle")]
pub use paddle::{Device, PaddleConfig, PaddleEngine};
pub use pipeline::{OcrParams, REC_CHUNK, ocr_image, ocr_image_cancellable};
pub use types::{AlignFrame, Image, OcrLine, OcrPage, OcrWord, Recognized, TextBox};
