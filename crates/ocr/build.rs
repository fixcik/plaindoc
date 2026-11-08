//! `bundled-models`: copy the pinned ONNX files (`model-manifest.txt`) into OUT_DIR after a
//! sha256 check and set `cfg(ocr_bundled)`. Each model is taken from the first of
//! `$PLAINDOC_OCR_BUNDLE_DIR`, `crates/ocr/models` (scripts/fetch-ocr-models.sh) and the user
//! models dir (the runtime default, `models::default_models_dir`); if still missing it is
//! downloaded into the user models dir with `curl` (sha256-checked, atomic rename).
//! `PLAINDOC_NO_FETCH=1` or `CARGO_NET_OFFLINE=true` skip the download. Soft by default: a
//! failed lookup gives a `cargo:warning` and a build without embedded models (the runtime resolver
//! then reports `OcrUnavailable`); `PLAINDOC_REQUIRE_MODELS=1` (alias
//! `PLAINDOC_OCR_REQUIRE_BUNDLE=1`) makes it a hard error. A file in the bundle dir or
//! `crates/ocr/models` with the wrong sha256 is always an error; in the user dir it is re-fetched.
//! See docs/building.md.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

const ENVS: &[&str] = &[
    "PLAINDOC_OCR_BUNDLE_DIR",
    "PLAINDOC_NO_FETCH",
    "CARGO_NET_OFFLINE",
    "PLAINDOC_REQUIRE_MODELS",
    "PLAINDOC_OCR_REQUIRE_BUNDLE",
    "PLAINDOC_CURL",
    "HOME",
    "XDG_DATA_HOME",
    "LOCALAPPDATA",
];

struct Model {
    id: String,
    sha256: String,
    url: String,
}

fn env_on(k: &str) -> bool {
    std::env::var(k).is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn parse_manifest(text: &str) -> Vec<Model> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.split_whitespace().collect::<Vec<_>>()[..] {
            [id, sha, _size, url] => Model {
                id: id.into(),
                sha256: sha.into(),
                url: url.into(),
            },
            _ => panic!("model-manifest.txt: bad line {l:?}"),
        })
        .collect()
}

/// Same as `src/models.rs::default_models_dir` (keep in sync): `$XDG_DATA_HOME` or
/// `~/.local/share` (Linux), `~/Library/Application Support` (macOS), `%LOCALAPPDATA%` (Windows),
/// then `plaindoc/models`.
fn user_models_dir() -> Option<PathBuf> {
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

fn model_path(dir: &Path, m: &Model) -> PathBuf {
    dir.join(&m.id).join("inference.onnx")
}

/// The model from an explicit dir (wrong hash: hard error) or the user dir (wrong hash: missing).
fn find(m: &Model, local: &[PathBuf], user: Option<&Path>) -> Option<(PathBuf, Vec<u8>)> {
    for dir in local {
        let path = model_path(dir, m);
        if let Ok(bytes) = std::fs::read(&path) {
            assert_eq!(
                sha256(&bytes),
                m.sha256,
                "sha256 mismatch for {}",
                path.display()
            );
            return Some((path, bytes));
        }
    }
    let path = model_path(user?, m);
    let bytes = std::fs::read(&path).ok()?;
    (sha256(&bytes) == m.sha256).then_some((path, bytes))
}

/// Removes `*.part.*` files older than a day (left by a killed build).
fn remove_stale_parts(dir: &Path) {
    let day = std::time::Duration::from_secs(24 * 3600);
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|d| d > day));
        if old && e.file_name().to_string_lossy().contains(".part.") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Downloads `m` into `dir` via a per-process `.part` file and a rename.
fn download(m: &Model, dir: &Path) -> Result<(PathBuf, Vec<u8>), String> {
    let path = model_path(dir, m);
    let parent = path.parent().unwrap();
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    remove_stale_parts(parent);
    let part = path.with_extension(format!("onnx.part.{}", std::process::id()));
    let curl = std::env::var("PLAINDOC_CURL").unwrap_or_else(|_| "curl".into());
    let status = Command::new(&curl)
        .args(["-fsSL", "--proto", "=https", "--proto-redir", "=https"])
        .args(["--retry", "2", "--connect-timeout", "20", "--max-time", "600", "-o"])
        .arg(&part)
        .arg(&m.url)
        .status()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!(
                "`{curl}` not found: install curl or point PLAINDOC_CURL at it, or fetch the models yourself"
            ),
            _ => format!("cannot run `{curl}`: {e}"),
        })?;
    let bytes = match status.success() {
        true => std::fs::read(&part).map_err(|e| format!("{}: {e}", part.display())),
        false => Err(format!("`{curl}` failed ({status}) for {}", m.url)),
    };
    let bytes = bytes.and_then(|b| match sha256(&b) == m.sha256 {
        true => Ok(b),
        false => Err(format!("sha256 mismatch for the download of {}", m.url)),
    });
    if bytes.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    let bytes = bytes?;
    if let Err(e) = std::fs::rename(&part, &path) {
        // A parallel build may have won the race: a correct file at the destination is fine.
        let won = std::fs::read(&path).is_ok_and(|b| sha256(&b) == m.sha256);
        let _ = std::fs::remove_file(&part);
        if !won {
            return Err(format!("cannot move {}: {e}", part.display()));
        }
    }
    Ok((path, bytes))
}

fn fetch(missing: &[&Model], user: Option<&Path>) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    if env_on("PLAINDOC_NO_FETCH") {
        return Err("download disabled by PLAINDOC_NO_FETCH".into());
    }
    if env_on("CARGO_NET_OFFLINE") {
        return Err("download disabled: cargo is offline (CARGO_NET_OFFLINE)".into());
    }
    let dir = user.ok_or("no user models dir (HOME / XDG_DATA_HOME / LOCALAPPDATA unset)")?;
    let got = missing
        .iter()
        .map(|m| download(m, dir))
        .collect::<Result<Vec<_>, _>>()?;
    println!(
        "cargo:warning=downloaded OCR models ({} file(s)) into {}",
        got.len(),
        dir.display()
    );
    Ok(got)
}

fn main() {
    println!("cargo::rustc-check-cfg=cfg(ocr_bundled)");
    for k in ENVS {
        println!("cargo:rerun-if-env-changed={k}");
    }
    let crate_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let manifest = crate_dir.join("model-manifest.txt");
    println!("cargo:rerun-if-changed={}", manifest.display());
    if std::env::var_os("CARGO_FEATURE_BUNDLED_MODELS").is_none() {
        return;
    }
    let require = env_on("PLAINDOC_REQUIRE_MODELS") || env_on("PLAINDOC_OCR_REQUIRE_BUNDLE");
    let models = parse_manifest(&std::fs::read_to_string(&manifest).unwrap());
    let local: Vec<PathBuf> = std::env::var_os("PLAINDOC_OCR_BUNDLE_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .into_iter()
        .chain([crate_dir.join("models")])
        .collect();
    let user = user_models_dir();
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for m in &models {
        match find(m, &local, user.as_deref()) {
            Some(f) => found.push((m, f)),
            None => missing.push(m),
        }
    }
    if !missing.is_empty() {
        match fetch(&missing, user.as_deref()) {
            Ok(got) => found.extend(missing.iter().copied().zip(got)),
            Err(why) => {
                let msg = format!(
                    "feature `bundled-models`: cannot get OCR model {} ({why}); run scripts/fetch-ocr-models.sh or set PLAINDOC_OCR_BUNDLE_DIR (see docs/building.md)",
                    missing[0].id
                );
                if require {
                    panic!("{msg} (PLAINDOC_REQUIRE_MODELS=1)");
                }
                println!("cargo:warning={msg}; building without embedded OCR models");
                // Rerun once a candidate appears. Only existing paths: a missing one reruns on
                // every build. A directory is scanned by cargo, so a new file in it is noticed.
                if let Some(u) = &user {
                    let _ = std::fs::create_dir_all(u);
                }
                for dir in local.iter().chain(user.as_ref()) {
                    let dir = if dir.exists() { dir } else { &crate_dir };
                    println!("cargo:rerun-if-changed={}", dir.display());
                }
                return;
            }
        }
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    for (m, (path, bytes)) in found {
        println!("cargo:rerun-if-changed={}", path.display());
        let dst = out.join(format!("{}.onnx", m.id));
        // Unchanged bytes keep their mtime, so a rerun does not recompile the crate.
        if std::fs::read(&dst).ok().as_deref() != Some(&bytes[..]) {
            std::fs::write(&dst, &bytes).unwrap();
        }
    }
    println!("cargo:rustc-cfg=ocr_bundled");
}
