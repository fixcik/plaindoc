//! PaddleOCR PP-OCRv5 mobile (det + cyrillic rec) on ONNX Runtime, CPU EP.

use std::borrow::Cow;
use std::path::Path;
use std::time::Duration;

use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;

use crate::det::{DbParams, db_boxes, det_input_size, det_tensor};
use crate::models::{DET_V5_MOBILE, REC_CYRILLIC_V5_MOBILE, load_model};
use crate::pool::Pool;
use crate::rec::{Charset, REC_H, crop_quad, ctc_greedy, rec_tensor};
use crate::{Image, OcrEngine, OcrError, Recognized, TextBox};

/// Execution device. v1 ships CPU only; GPU providers are a later, measured addition.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Device {
    #[default]
    Cpu,
}

/// Engine settings.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct PaddleConfig {
    /// Detector input long-side limit; 0 = full resolution (tables, small print).
    pub det_limit: u32,
    /// Maximum live session pairs. The CPU memory arena is disabled (see `session`), so memory
    /// follows the current input instead of growing to the largest one seen. Measured with one pair
    /// on a mixed stream (A4 scans and 500-3000 px images, 60 documents): peak ~0.6 GB RSS, no
    /// growth after warm-up. Budget ~0.8 GB per pair to stay conservative, so size it from the
    /// memory limit: `(mem_limit - 0.5 GB) / 0.8 GB`. It is also the number of threads that can
    /// run OCR at once.
    pub max_sessions: usize,
    /// How long a caller waits for a busy pool before `OcrError::Engine` (no indefinite waits).
    pub session_wait: Duration,
    pub device: Device,
    pub db: DbParams,
}

impl Default for PaddleConfig {
    fn default() -> Self {
        Self {
            det_limit: 960,
            // min(cores, 4): one pair per core that can run OCR, at most 4 (~3.2 GB budget) by default.
            max_sessions: std::thread::available_parallelism().map_or(1, |n| n.get().min(4)),
            session_wait: Duration::from_secs(120),
            device: Device::Cpu,
            db: DbParams::default(),
        }
    }
}

struct Pair {
    det: Session,
    rec: Session,
}

/// PaddleOCR engine. `Session::run` needs `&mut`, so sessions are pooled, never shared.
pub struct PaddleEngine {
    det_model: Cow<'static, [u8]>,
    rec_model: Cow<'static, [u8]>,
    charset: Charset,
    cfg: PaddleConfig,
    pool: Pool<Pair>,
}

fn engine_err(e: impl std::fmt::Display) -> OcrError {
    OcrError::Engine(e.to_string())
}

fn session(model: &[u8]) -> Result<Session, OcrError> {
    // intra = inter = 1: inference runs on the calling thread, so the caller's pool is the only
    // thread budget (no ORT thread pools on top of rayon).
    // The CPU arena is off: with it the RSS kept growing to the largest input seen (+37 % between
    // documents 40 and 60 of a mixed stream, 779 MB) and was never returned; without it the RSS
    // plateaus at ~0.6 GB at the same speed.
    Session::builder()
        .map_err(engine_err)?
        .with_execution_providers([ort::ep::CPU::default().with_arena_allocator(false).build()])
        .map_err(engine_err)?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(engine_err)?
        .with_intra_threads(1)
        .map_err(engine_err)?
        .with_inter_threads(1)
        .map_err(engine_err)?
        .commit_from_memory(model)
        .map_err(engine_err)
}

impl PaddleEngine {
    /// Resolves models (bundled, else `models_dir`, verified); sessions are created lazily.
    pub fn new(models_dir: Option<&Path>, cfg: PaddleConfig) -> Result<Self, OcrError> {
        let det_model = load_model(&DET_V5_MOBILE, models_dir)?;
        let rec_model = load_model(&REC_CYRILLIC_V5_MOBILE, models_dir)?;
        Ok(Self {
            det_model,
            rec_model,
            charset: Charset::cyrillic(),
            pool: Pool::new(cfg.max_sessions, cfg.session_wait),
            cfg,
        })
    }

    fn with_pair<T>(
        &self,
        f: impl FnOnce(&mut Pair) -> Result<T, OcrError>,
    ) -> Result<T, OcrError> {
        let create = || {
            Ok(Pair {
                det: session(&self.det_model)?,
                rec: session(&self.rec_model)?,
            })
        };
        self.pool.with(create, f)
    }

    fn rec_one(&self, s: &mut Session, crop: &Image) -> Result<Recognized, OcrError> {
        let (data, iw, rw) = rec_tensor(crop);
        let input = Tensor::from_array((
            [1usize, 3, REC_H as usize, iw as usize],
            data.into_boxed_slice(),
        ))
        .map_err(engine_err)?;
        let out = s.run(ort::inputs![input]).map_err(engine_err)?;
        let (shape, probs) = out[0].try_extract_tensor::<f32>().map_err(engine_err)?;
        let (t, c) = (shape[1] as usize, shape[2] as usize);
        if c != self.charset.classes() {
            return Err(OcrError::Engine(format!(
                "rec classes {c} != {}",
                self.charset.classes()
            )));
        }
        let mut r = ctc_greedy(probs, t, c, &self.charset);
        r.width_frac = rw as f32 / iw as f32;
        Ok(r)
    }
}

impl OcrEngine for PaddleEngine {
    fn detect(&self, img: &Image) -> Result<Vec<TextBox>, OcrError> {
        self.detect_at(img, self.cfg.det_limit)
    }

    fn detect_at(&self, img: &Image, max_side: u32) -> Result<Vec<TextBox>, OcrError> {
        let (w, h) = img.dimensions();
        let (rw, rh) = det_input_size(w, h, max_side);
        let data = det_tensor(img, rw, rh);
        self.with_pair(|p| {
            let input = Tensor::from_array((
                [1usize, 3, rh as usize, rw as usize],
                data.into_boxed_slice(),
            ))
            .map_err(engine_err)?;
            let out = p.det.run(ort::inputs![input]).map_err(engine_err)?;
            let (_, prob) = out[0].try_extract_tensor::<f32>().map_err(engine_err)?;
            Ok(db_boxes(
                prob,
                rw as usize,
                rh as usize,
                w as f32 / rw as f32,
                h as f32 / rh as f32,
                (w, h),
                &self.cfg.db,
            ))
        })
    }

    fn max_concurrency(&self) -> usize {
        self.cfg.max_sessions.max(1)
    }

    fn recognize(&self, img: &Image, boxes: &[TextBox]) -> Result<Vec<Recognized>, OcrError> {
        if boxes.is_empty() {
            // Nothing to read: do not check out (or create) a session pair.
            return Ok(Vec::new());
        }
        self.with_pair(|p| {
            boxes
                .iter()
                .map(|b| {
                    let crop = crop_quad(img, b, false);
                    if crop.dimensions() == (1, 1) {
                        // Degenerate box (crop_quad's fallback or a 1 px sliver): nothing to read.
                        return Ok(Recognized::default());
                    }
                    let mut r = self.rec_one(&mut p.rec, &crop)?;
                    if b.is_vertical() {
                        // Vertical text may run either way: keep the more confident reading.
                        let mut f = self.rec_one(&mut p.rec, &crop_quad(img, b, true))?;
                        if f.conf > r.conf {
                            f.flipped = true;
                            r = f;
                        }
                    }
                    Ok(r)
                })
                .collect()
        })
    }
}
