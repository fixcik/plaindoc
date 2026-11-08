//! Model manifest and the single resolver: bundled bytes -> models directory -> `ModelsUnavailable`
//! naming every place searched. No network access here (the build downloads bundled models, see
//! `build.rs`; `scripts/fetch-ocr-models.sh` fetches them for a models directory).

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::OcrError;

/// One pinned model file. On disk: `<models dir>/<id>/inference.onnx`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelFile {
    pub id: &'static str,
    pub version: &'static str,
    pub license: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

pub const DET_V5_MOBILE: ModelFile = ModelFile {
    id: "PP-OCRv5_mobile_det",
    version: "hf:e6f4fa85f00e168c862bc462aebca69eef9b3d3d",
    license: "Apache-2.0",
    url: "https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_det_onnx/resolve/e6f4fa85f00e168c862bc462aebca69eef9b3d3d/inference.onnx",
    sha256: "a431985659dc921974177a95adcfbb90fd9e51989a5e04d70d0b75f597b6e61d",
    size: 4_826_518,
};

pub const REC_CYRILLIC_V5_MOBILE: ModelFile = ModelFile {
    id: "cyrillic_PP-OCRv5_mobile_rec",
    version: "hf:2cef88145434beb8afa9dd82d77d799eb1ad7b29",
    license: "Apache-2.0",
    url: "https://huggingface.co/PaddlePaddle/cyrillic_PP-OCRv5_mobile_rec_onnx/resolve/2cef88145434beb8afa9dd82d77d799eb1ad7b29/inference.onnx",
    sha256: "5371ee1ddaa7983cc62d0818d99e982b6804638c85e4f960d59a574094e172e5",
    size: 8_048_799,
};

/// Every model the crate knows; `build.rs` bundles the first two under `bundled-models`. URLs and
/// hashes must match `model-manifest.txt` (checked by a test).
pub const MANIFEST: &[ModelFile] = &[DET_V5_MOBILE, REC_CYRILLIC_V5_MOBILE];

/// Environment variable with the models directory.
pub const MODELS_ENV: &str = "PLAINDOC_OCR_MODELS";

/// How to get models into a binary that has none; ends every "model missing" message.
const FIX_HINT: &str = "build with embedded models (feature `bundled-models`, see docs/building.md), \
or fetch them with scripts/fetch-ocr-models.sh DIR and pass --ocr-models DIR or set $PLAINDOC_OCR_MODELS";

/// Bytes compiled into the binary, if this model is bundled. `cfg(ocr_bundled)` is set by
/// `build.rs` only when feature `bundled-models` found the files; otherwise this is `None`.
pub fn bundled(m: &ModelFile) -> Option<&'static [u8]> {
    #[cfg(ocr_bundled)]
    {
        match m.id {
            "PP-OCRv5_mobile_det" => {
                return Some(include_bytes!(concat!(
                    env!("OUT_DIR"),
                    "/PP-OCRv5_mobile_det.onnx"
                )));
            }
            "cyrillic_PP-OCRv5_mobile_rec" => {
                return Some(include_bytes!(concat!(
                    env!("OUT_DIR"),
                    "/cyrillic_PP-OCRv5_mobile_rec.onnx"
                )));
            }
            _ => {}
        }
    }
    let _ = m;
    None
}

/// Per-user data dir: `$XDG_DATA_HOME` or `~/.local/share` (Linux), `~/Library/Application Support`
/// (macOS), `%LOCALAPPDATA%` (Windows), then `plaindoc/models`.
pub fn default_models_dir() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(windows) {
        env("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local/share")))
    }?;
    Some(base.join("plaindoc").join("models"))
}

/// First of: `explicit` (`--ocr-models`), `$PLAINDOC_OCR_MODELS`, [`default_models_dir`].
pub fn resolve_models_dir(explicit: Option<&Path>) -> Option<PathBuf> {
    explicit
        .map(Path::to_path_buf)
        .or_else(|| {
            std::env::var_os(MODELS_ENV)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
        .or_else(default_models_dir)
}

fn check_size(len: u64, m: &ModelFile, what: &str) -> Result<(), OcrError> {
    if len != m.size {
        return Err(OcrError::ModelsUnavailable(format!(
            "{what}: size {len} != {}",
            m.size
        )));
    }
    Ok(())
}

fn verify(bytes: &[u8], m: &ModelFile, what: &str) -> Result<(), OcrError> {
    check_size(bytes.len() as u64, m, what)?;
    let got = format!("{:x}", Sha256::digest(bytes));
    if got != m.sha256 {
        return Err(OcrError::ModelsUnavailable(format!(
            "{what}: sha256 {got} != {}",
            m.sha256
        )));
    }
    Ok(())
}

/// Resolves one model: bundled bytes (checked at build time), else `<dir>/<id>/inference.onnx`
/// with size and sha256 checked on every load.
pub fn load_model(m: &ModelFile, dir: Option<&Path>) -> Result<Cow<'static, [u8]>, OcrError> {
    if let Some(b) = bundled(m) {
        return Ok(Cow::Borrowed(b));
    }
    let Some(dir) = dir else {
        return Err(OcrError::ModelsUnavailable(format!(
            "OCR models are missing: model {} is not bundled and no models directory is set; {FIX_HINT}",
            m.id
        )));
    };
    let path = dir.join(m.id).join("inference.onnx");
    let not_found = |e: std::io::Error| {
        OcrError::ModelsUnavailable(format!(
            "OCR models are missing: model {} not found at {} ({e}); {FIX_HINT}",
            m.id,
            path.display()
        ))
    };
    let what = path.display().to_string();
    // Size first, so a wrong (possibly huge) file is rejected without reading it into memory.
    check_size(std::fs::metadata(&path).map_err(not_found)?.len(), m, &what)?;
    let bytes = std::fs::read(&path).map_err(not_found)?;
    verify(&bytes, m, &what)?;
    Ok(Cow::Owned(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TINY: ModelFile = ModelFile {
        id: "tiny",
        version: "1",
        license: "test",
        url: "",
        // sha256("abc")
        sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        size: 3,
    };

    #[test]
    fn checksum_and_size_are_enforced_and_path_is_named() {
        let d = tempfile::tempdir().unwrap();
        let err = |r: Result<Cow<'static, [u8]>, OcrError>| match r {
            Err(OcrError::ModelsUnavailable(m)) => m,
            other => panic!("{other:?}"),
        };
        // The platform separator: `tiny\inference.onnx` on Windows.
        let named = Path::new("tiny").join("inference.onnx");
        assert!(err(load_model(&TINY, Some(d.path()))).contains(&*named.to_string_lossy()));
        assert!(err(load_model(&TINY, None)).contains("not bundled"));
        std::fs::create_dir(d.path().join("tiny")).unwrap();
        std::fs::write(d.path().join("tiny/inference.onnx"), b"abd").unwrap();
        assert!(err(load_model(&TINY, Some(d.path()))).contains("sha256"));
        std::fs::write(d.path().join("tiny/inference.onnx"), b"abcd").unwrap();
        assert!(err(load_model(&TINY, Some(d.path()))).contains("size"));
        std::fs::write(d.path().join("tiny/inference.onnx"), b"abc").unwrap();
        assert_eq!(&*load_model(&TINY, Some(d.path())).unwrap(), b"abc");
    }

    #[test]
    fn explicit_dir_wins_and_manifest_is_pinned() {
        assert_eq!(
            resolve_models_dir(Some(Path::new("/m"))),
            Some(PathBuf::from("/m"))
        );
        assert!(
            MANIFEST
                .iter()
                .all(|m| m.sha256.len() == 64 && m.license == "Apache-2.0")
        );
    }

    #[test]
    fn manifest_matches_the_shared_manifest_file() {
        let text = include_str!("../model-manifest.txt");
        let lines: Vec<Vec<&str>> = text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(|l| l.split_whitespace().collect())
            .collect();
        assert_eq!(lines.len(), 2);
        for (m, l) in MANIFEST.iter().zip(&lines) {
            let size = m.size.to_string();
            assert_eq!(l[..], [m.id, m.sha256, size.as_str(), m.url]);
        }
    }

    #[test]
    fn missing_model_message_names_the_fixes() {
        let d = tempfile::tempdir().unwrap();
        for dir in [None, Some(d.path())] {
            let Err(OcrError::ModelsUnavailable(m)) = load_model(&TINY, dir) else {
                panic!()
            };
            assert!(m.starts_with("OCR models are missing: model tiny"), "{m}");
            assert!(
                m.contains("--ocr-models DIR") && m.contains(MODELS_ENV),
                "{m}"
            );
        }
    }

    #[cfg(ocr_bundled)]
    #[test]
    fn bundled_models_need_no_directory() {
        for m in &MANIFEST[..2] {
            assert_eq!(load_model(m, None).unwrap().len() as u64, m.size);
        }
    }
}
