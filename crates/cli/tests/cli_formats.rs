use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

fn bin() -> assert_cmd::Command {
    cargo_bin_cmd!("plaindoc")
}

fn write(dir: &Path, rel: &str, body: &str) -> PathBuf {
    let p = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, body).unwrap();
    p
}

#[test]
fn pdf_table_becomes_markdown_table() {
    let pdf = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../formats/tests/fixtures/pdf/ru.pdf"
    );
    let out = bin().args(["parse", pdf]).output().unwrap();
    assert!(out.status.success());
    let md = String::from_utf8(out.stdout).unwrap();
    assert!(
        md.contains("| Квартал | Выручка | Прибыль |\n| --- | --- | --- |\n| Q1 | 1 200 | 300 |"),
        "{md}"
    );
    assert_eq!(
        md.matches("1 200").count(),
        1,
        "table text is not repeated as prose"
    );
}

#[test]
fn html_saved_as_xls_and_rtf_are_parsed() {
    let d = tempfile::tempdir().unwrap();
    let xls = write(
        d.path(),
        "1c.xls",
        "<html><table><tr><th>a</th><th>b</th></tr><tr><td>1</td><td>2</td></tr></table></html>",
    );
    bin()
        .arg("parse")
        .arg(&xls)
        .assert()
        .success()
        .stdout("| a | b |\n| --- | --- |\n| 1 | 2 |\n");
    let rtf = write(d.path(), "r.rtf", r"{\rtf1\ansi Hello\par}");
    bin()
        .arg("parse")
        .arg(&rtf)
        .assert()
        .success()
        .stdout("Hello\n");
}

#[test]
fn headers_footers_flag_shows_them_in_markdown() {
    let d = tempfile::tempdir().unwrap();
    let rtf = write(
        d.path(),
        "h.rtf",
        r"{\rtf1{\header \pard Head\par}\pard Body\par}",
    );
    bin()
        .arg("parse")
        .arg(&rtf)
        .assert()
        .success()
        .stdout("Body\n");
    bin()
        .args(["parse", "--headers-footers"])
        .arg(&rtf)
        .assert()
        .success()
        .stdout("<!-- header: Head -->\n\nBody\n");
    bin()
        .args(["parse", "-f", "json"])
        .arg(&rtf)
        .assert()
        .success()
        .stdout(predicate::str::contains("\"kind\": \"header\""));
}

#[cfg(feature = "ocr")]
#[test]
fn ocr_flags_parse_and_models_list_prints_manifest() {
    let d = tempfile::tempdir().unwrap();
    // The flag beats $PLAINDOC_OCR_MODELS: unless bundled, the printed paths are under the flag's dir.
    let out = bin()
        .env("PLAINDOC_OCR_MODELS", "/nonexistent-env-models")
        .args(["--ocr-models", d.path().to_str().unwrap(), "models", "list"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains(d.path().to_str().unwrap()) || !text.contains("missing"));
    assert!(!text.contains("nonexistent-env-models"), "{text}");
    bin()
        .args(["--ocr-models", d.path().to_str().unwrap(), "models", "list"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("PP-OCRv5_mobile_det")
                .and(predicate::str::contains("Apache-2.0"))
                .and(predicate::str::contains("bundled").or(predicate::str::contains("missing"))),
        );
    bin()
        .args(["--ocr-threads", "0", "detect", "x"])
        .assert()
        .code(64);
}

/// With bundled models the scan is read; without them (soft bundling, clean clone) the parse still
/// succeeds and the warning names the missing model.
#[cfg(feature = "ocr")]
#[test]
fn scan_pdf_parses_with_bundled_ocr() {
    let scan = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../formats/tests/fixtures/pdf/scan.pdf"
    );
    let d = tempfile::tempdir().unwrap(); // empty models dir: only bundled models can work
    let list = bin()
        .args(["--ocr-models", d.path().to_str().unwrap(), "models", "list"])
        .output()
        .unwrap();
    let bundled = String::from_utf8_lossy(&list.stdout).contains("\tbundled");
    let a = bin()
        .args([
            "--ocr-models",
            d.path().to_str().unwrap(),
            "parse",
            "-f",
            "json",
            scan,
        ])
        .assert()
        .success();
    if bundled {
        a.stdout(predicate::str::contains("Договор поставки"));
    } else {
        a.stdout(
            predicate::str::contains("OcrUnavailable")
                .and(predicate::str::contains("PP-OCRv5_mobile_det")),
        )
        .stderr(predicate::str::contains("OCR models are missing"));
    }
}

#[test]
fn hidden_text_flag_is_validated() {
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "a.txt", "plain text\n");
    for mode in ["off", "flag", "strip"] {
        bin()
            .args(["parse", "--hidden-text", mode])
            .arg(&f)
            .assert()
            .code(0);
    }
    bin()
        .args(["parse", "--hidden-text", "bogus"])
        .arg(&f)
        .assert()
        .code(64);
}
