//! Letter-bigram tables: log10 P(b|a), add-one smoothing, "ё" folded into "е".
//!
//! The table `data/need_ocr/bigrams.json` (`{"ru": {"alphabet": "...", "logp": [[..]]}, "en":
//! {...}}`) is counted from public-domain Russian texts (ru.wikisource) and the opendataloader-bench
//! ground truth (sources in `data/need_ocr/NOTICE`). `build.rs` embeds it when present
//! ([`Bigrams::bundled`]). The classifier thresholds (`bigram` -1.35 / -1.9) and the golden
//! `bigram_lp` values are checked against exactly that file (task 14a gate), so its sha256 is
//! pinned and any other table is refused.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;

/// sha256 of the calibrated `bigrams.json` (pinned in `build.rs`).
pub(crate) const BIGRAMS_SHA256: &str = env!("NEED_OCR_BIGRAMS_SHA256");

/// Russian (32 letters, no "ё") and English (26 letters) bigram tables.
#[derive(Clone, Debug, PartialEq)]
pub struct Bigrams {
    pub(crate) ru_alphabet: Vec<char>,
    pub(crate) ru_logp: [[f32; 32]; 32],
    pub(crate) en_alphabet: Vec<char>,
    pub(crate) en_logp: [[f32; 26]; 26],
}

impl Bigrams {
    /// Loads and checks the pinned table from a file (see [`Bigrams::from_bytes`]).
    pub fn load(path: &Path) -> Result<Bigrams, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_bytes(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Checks and parses the pinned table (sha256 and shapes 32x32 / 26x26).
    pub fn from_bytes(bytes: &[u8]) -> Result<Bigrams, String> {
        let hash: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if hash != BIGRAMS_SHA256 {
            return Err(format!("sha256 {hash}, expected {BIGRAMS_SHA256}"));
        }
        let v: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let (ru_alphabet, ru_logp) = table::<32>(&v, "ru")?;
        let (en_alphabet, en_logp) = table::<26>(&v, "en")?;
        Ok(Bigrams {
            ru_alphabet,
            ru_logp,
            en_alphabet,
            en_logp,
        })
    }

    /// The table compiled into this build (parsed once), `None` when `build.rs` found no
    /// `data/need_ocr/bigrams.json` (or, defensively, when the embedded bytes do not parse).
    pub fn bundled() -> Option<Arc<Bigrams>> {
        #[cfg(need_ocr_bigrams)]
        {
            static TABLE: std::sync::OnceLock<Option<Arc<Bigrams>>> = std::sync::OnceLock::new();
            const BYTES: &[u8] = include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/data/need_ocr/bigrams.json"
            ));
            let t = TABLE.get_or_init(|| Self::from_bytes(BYTES).ok().map(Arc::new));
            return t.clone();
        }
        #[allow(unreachable_code)]
        None
    }
}

fn table<const N: usize>(v: &Value, lang: &str) -> Result<(Vec<char>, [[f32; N]; N]), String> {
    let bad = || format!("bigrams.json: bad `{lang}` table, expected {N}x{N}");
    let alphabet: Vec<char> = v[lang]["alphabet"]
        .as_str()
        .ok_or_else(bad)?
        .chars()
        .collect();
    let rows = v[lang]["logp"].as_array().ok_or_else(bad)?;
    if alphabet.len() != N || rows.len() != N {
        return Err(bad());
    }
    let mut logp = [[0f32; N]; N];
    for (dst, row) in logp.iter_mut().zip(rows) {
        let row = row.as_array().filter(|r| r.len() == N).ok_or_else(bad)?;
        for (d, x) in dst.iter_mut().zip(row) {
            *d = x.as_f64().ok_or_else(bad)? as f32;
        }
    }
    Ok((alphabet, logp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bigrams_load_rejects_wrong_hash() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bigrams.json");
        std::fs::write(&p, r#"{"ru": {"alphabet": "", "logp": []}, "en": {}}"#).unwrap();
        let e = Bigrams::load(&p).unwrap_err();
        assert!(e.contains("sha256"), "{e}");
    }

    #[cfg(need_ocr_bigrams)]
    #[test]
    fn bundled_table_is_parsed_once() {
        let (a, b) = (Bigrams::bundled().unwrap(), Bigrams::bundled().unwrap());
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!((a.ru_alphabet.len(), a.en_alphabet.len()), (32, 26));
    }
}
