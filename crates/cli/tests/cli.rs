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
fn single_csv_to_stdout_as_gfm() {
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "t.csv", "a,b\n1,2\n");
    bin()
        .arg("parse")
        .arg(&p)
        .assert()
        .success()
        .stdout("<!-- sheet 1 -->\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n");
}

#[test]
fn json_output() {
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "t.md", "# Hi\n");
    bin()
        .args(["parse", "-f", "json"])
        .arg(&p)
        .assert()
        .success()
        .stdout(
            predicate::str::contains("\"type\": \"paragraph\"")
                .and(predicate::str::contains("\"format\": \"md\"")),
        );
}

#[test]
fn several_files_without_out_dir_is_usage_error() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.txt", "a");
    let b = write(d.path(), "b.txt", "b");
    bin()
        .arg("parse")
        .arg(&a)
        .arg(&b)
        .assert()
        .code(64)
        .stderr(predicate::str::contains("require -o"));
}

#[test]
fn usage_errors_exit_64_and_help_exits_0() {
    bin().arg("parse").assert().code(64);
    bin()
        .args(["parse", "-f", "yaml", "x.txt"])
        .assert()
        .code(64);
    bin().arg("--help").assert().success();
    bin().arg("--version").assert().success();
}

#[test]
fn unsupported_format_is_error_exit_1() {
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "a.pdf", "%PDF-1.7");
    bin().arg("parse").arg(&p).assert().code(1).stderr(
        predicate::str::contains("unsupported format: pdf")
            .or(predicate::str::contains("corrupt document")),
    );
}

#[test]
fn directory_without_out_dir_is_usage_error() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "in/a.txt", "a");
    bin()
        .arg("parse")
        .arg(d.path().join("in"))
        .assert()
        .code(64);
}

#[test]
fn directory_batch_keeps_relative_paths_and_skips_unsupported() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "in/a/x.csv", "h\n1\n");
    write(d.path(), "in/b/x.csv", "h\n2\n");
    write(d.path(), "in/c/skip.bin", "\0\0");
    write(d.path(), "in/d/run.log", "INFO ok\n");
    bin()
        .current_dir(d.path())
        .args(["parse", "in", "-o", "out", "-j", "2"])
        .assert()
        .success();
    let out = d.path().join("out");
    assert!(
        fs::read_to_string(out.join("in/a/x.csv.md"))
            .unwrap()
            .contains("| 1 |")
    );
    assert!(
        fs::read_to_string(out.join("in/b/x.csv.md"))
            .unwrap()
            .contains("| 2 |")
    );
    assert!(!out.join("in/c").exists());
    assert_eq!(
        fs::read_to_string(out.join("in/d/run.log.md")).unwrap(),
        "```\nINFO ok\n```\n"
    );
}

#[test]
fn one_failure_gives_exit_1_but_others_are_written() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "ok.txt", "fine");
    bin()
        .current_dir(d.path())
        .args(["parse", "ok.txt", "missing.txt", "-o", "out", "-f", "json"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("missing.txt"));
    assert!(
        fs::read_to_string(d.path().join("out/ok.txt.json"))
            .unwrap()
            .contains(r#""text": "fine\n""#)
    );
}

#[test]
fn strict_low_quality_exits_2_and_warning_goes_to_stderr() {
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "r.csv", "a,b\n1\n");
    bin()
        .arg("parse")
        .arg(&p)
        .args(["--min-quality", "0.95", "--strict"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("TableUncertain"))
        .stdout(predicate::str::contains(
            "<!-- warning(warn) TableUncertain",
        ));
    bin()
        .arg("parse")
        .arg(&p)
        .args(["--min-quality", "0.95"])
        .assert()
        .success();
    bin()
        .arg("parse")
        .arg(&p)
        .arg("--no-warnings")
        .assert()
        .success()
        .stderr(predicate::str::contains("TableUncertain").not())
        .stdout(predicate::str::contains("warning(").not());
}

#[test]
fn failure_beats_low_quality() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "r.csv", "a,b\n1\n");
    bin()
        .current_dir(d.path())
        .args([
            "parse",
            "r.csv",
            "nope.txt",
            "-o",
            "out",
            "--min-quality",
            "0.95",
            "--strict",
        ])
        .assert()
        .code(1);
}

#[test]
fn strict_requires_min_quality() {
    bin().args(["parse", "--strict", "x.txt"]).assert().code(64);
}

#[test]
fn detect_prints_formats() {
    let d = tempfile::tempdir().unwrap();
    let pdf = write(d.path(), "a.pdf", "%PDF-1.7");
    let md = write(d.path(), "b.md", "# x");
    bin()
        .arg("detect")
        .arg(&pdf)
        .arg(&md)
        .assert()
        .success()
        .stdout(
            predicate::str::contains("a.pdf\tpdf\n").and(predicate::str::contains("b.md\tmd\n")),
        );
    let cfb = d.path().join("e.docx");
    // With the office CFB probe a real container is needed (magic alone is Corrupt).
    fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../formats/tests/fixtures/office/generated/encrypted.docx"
        ),
        &cfb,
    )
    .unwrap();
    bin()
        .arg("detect")
        .arg(&cfb)
        .assert()
        .success()
        .stdout(predicate::str::contains("e.docx\tencrypted"));
    let xls = write(
        d.path(),
        "1c.xls",
        "<html><table><tr><td>1</td></tr></table></html>",
    );
    bin()
        .arg("detect")
        .arg(&xls)
        .assert()
        .success()
        .stdout(predicate::str::contains("1c.xls\thtml"));
    let bin_file = write(d.path(), "c.bin", "\0\0");
    bin()
        .arg("detect")
        .arg(&bin_file)
        .assert()
        .code(1)
        .stdout(predicate::str::contains("c.bin\tunknown"));
}

#[cfg(unix)]
#[test]
fn unreadable_file_in_directory_gives_exit_1() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "in/ok.txt", "fine");
    let bad = write(d.path(), "in/bad.txt", "secret");
    fs::set_permissions(&bad, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(&bad).is_ok() {
        return; // running as root: permissions are not enforced
    }
    bin()
        .current_dir(d.path())
        .args(["parse", "in", "-o", "out"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("bad.txt"));
    assert!(d.path().join("out/in/ok.txt.md").exists());
}

#[test]
fn min_quality_is_validated() {
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "r.csv", "a,b\n1,2\n");
    for bad in ["nan", "inf", "1.5", "-0.1"] {
        bin()
            .arg("parse")
            .arg(&p)
            .arg(format!("--min-quality={bad}"))
            .assert()
            .code(64);
    }
    for ok in ["0", "1", "0.5"] {
        bin()
            .arg("parse")
            .arg(&p)
            .arg(format!("--min-quality={ok}"))
            .assert()
            .success();
    }
}

#[test]
fn low_quality_not_last_in_batch_exits_2() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "a.csv", "a,b\n1\n");
    write(d.path(), "b.csv", "a,b\n1,2\n");
    bin()
        .current_dir(d.path())
        .args([
            "parse",
            "a.csv",
            "b.csv",
            "-o",
            "out",
            "--min-quality",
            "0.95",
            "--strict",
        ])
        .assert()
        .code(2);
}

#[test]
fn check_reports_quality_without_content() {
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "r.csv", "secret,b\n1\n");
    bin().arg("check").arg(&p).assert().success().stdout(
        predicate::str::contains("r.csv\tcsv\tquality=0.90\twarnings=1")
            .and(predicate::str::contains("secret").not()),
    );
    bin()
        .arg("check")
        .arg(d.path().join("nope.txt"))
        .assert()
        .code(1)
        .stdout(predicate::str::contains("nope.txt\terror\t"));
}

#[cfg(unix)]
#[test]
fn unreadable_subdirectory_is_reported_and_batch_continues() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "in/ok.txt", "fine");
    write(d.path(), "in/locked/x.txt", "hidden");
    let locked = d.path().join("in/locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let readable = fs::read_dir(&locked).is_ok();
    if !readable {
        bin()
            .current_dir(d.path())
            .args(["parse", "in", "-o", "out"])
            .assert()
            .code(1)
            .stderr(predicate::str::contains("error: ").and(predicate::str::contains("locked")));
        assert!(d.path().join("out/in/ok.txt.md").exists());
    }
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn batch_output_never_overwrites_an_input_file() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "a.txt", "hello");
    write(d.path(), "a.txt.md", "original-source");
    bin()
        .current_dir(d.path())
        .args(["parse", ".", "-o", "."])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(d.path().join("a.txt.md")).unwrap(),
        "original-source"
    );
    assert!(d.path().join("a.txt~2.md").exists());
}

#[test]
fn closed_stdout_stops_quietly() {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let d = tempfile::tempdir().unwrap();
    let body: String = (0..100_000).map(|i| format!("{i},cell {i}\n")).collect();
    let p = write(d.path(), "big.csv", &body);
    for format in ["markdown", "json"] {
        let mut child = Command::new(assert_cmd::cargo::cargo_bin!("plaindoc"))
            .args(["parse", "-f", format])
            .arg(&p)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut head = [0u8; 100];
        child.stdout.take().unwrap().read_exact(&mut head).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{format}: {:?}", out.status);
        assert!(
            out.stderr.is_empty(),
            "{format}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
