//! Feature `pdf`: embed the need-OCR bigram table `data/need_ocr/bigrams.json` (see
//! `data/need_ocr/NOTICE`) and set `cfg(need_ocr_bigrams)`. A missing file builds without the
//! table (the classifier then skips its bigram rule and says so in one Info warning per PDF); a
//! file with another sha256 is an error. The pinned hash reaches `need_ocr/bigrams.rs` as
//! `NEED_OCR_BIGRAMS_SHA256`.

use sha2::{Digest, Sha256};
use std::path::PathBuf;

const BIGRAMS_SHA256: &str = "8b20b8d441981919706100317740d02ec7f3f92d1e569caf19d4358b342a170b";

fn main() {
    println!("cargo::rustc-check-cfg=cfg(need_ocr_bigrams)");
    println!("cargo:rustc-env=NEED_OCR_BIGRAMS_SHA256={BIGRAMS_SHA256}");
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("data/need_ocr");
    let path = dir.join("bigrams.json");
    // The directory, not the file: a missing file would rerun the script on every build.
    println!("cargo:rerun-if-changed={}", dir.display());
    if std::env::var_os("CARGO_FEATURE_PDF").is_none() {
        return;
    }
    let Ok(bytes) = std::fs::read(&path) else {
        println!(
            "cargo:warning={} not found: need-OCR classifier built without the bigram table",
            path.display()
        );
        return;
    };
    let got = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(
        got,
        BIGRAMS_SHA256,
        "sha256 mismatch for {}",
        path.display()
    );
    println!("cargo:rustc-cfg=need_ocr_bigrams");
}
