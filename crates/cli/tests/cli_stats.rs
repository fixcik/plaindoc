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

fn stats_lines(p: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(p)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// Drops durations: `wall_us` and the values of `stages_us`.
fn untimed(mut v: serde_json::Value) -> serde_json::Value {
    let s = v["stats"].as_object_mut().unwrap();
    s.remove("wall_us");
    if let Some(st) = s.get_mut("stages_us").and_then(|x| x.as_object_mut()) {
        st.values_mut().for_each(|x| *x = 0.into());
    }
    v
}

#[test]
fn stats_file_one_line_per_input_in_order() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "in/a.csv", "a,b\n1,2\n");
    write(d.path(), "in/b.csv", "x\n");
    let s = d.path().join("s.jsonl");
    bin()
        .args(["parse", "-j", "2", "-o"])
        .arg(d.path().join("out"))
        .arg("--stats")
        .arg(&s)
        .arg(d.path().join("in"))
        .assert()
        .success();
    let v = stats_lines(&s);
    assert_eq!(v.len(), 2);
    assert!(v[0]["file"].as_str().unwrap().ends_with("a.csv"));
    assert!(v[1]["file"].as_str().unwrap().ends_with("b.csv"));
    assert_eq!(v[0]["stats"]["format"], "csv");
    assert_eq!(v[0]["stats"]["outcome"], "ok");
}

#[test]
fn stats_do_not_change_output() {
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "t.md", "# Hi\n\ntext\n");
    let plain = bin()
        .args(["parse", "-f", "json"])
        .arg(&p)
        .output()
        .unwrap();
    let with = bin()
        .args(["parse", "-f", "json", "--stats"])
        .arg(d.path().join("s.jsonl"))
        .arg(&p)
        .output()
        .unwrap();
    assert_eq!(plain.stdout, with.stdout);
}

#[test]
fn stats_report_failures() {
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "bad.pdf", "%PDF-1.4\nnot a pdf");
    let s = d.path().join("s.jsonl");
    bin()
        .args(["parse", "--stats"])
        .arg(&s)
        .arg(&p)
        .assert()
        .code(1);
    let v = stats_lines(&s);
    assert_eq!(v.len(), 1);
    assert_ne!(v[0]["stats"]["outcome"], "ok");
}

#[test]
fn stats_to_stderr() {
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "t.csv", "a\n");
    bin()
        .args(["parse", "--stats", "-"])
        .arg(&p)
        .assert()
        .success()
        .stderr(predicate::str::contains("\"stats\":{\"format\":\"csv\""));
}

#[test]
fn stats_file_has_no_content() {
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "t.csv", "Qx7731SecretMarker,b\n1,2\n");
    let s = d.path().join("s.jsonl");
    bin()
        .args(["parse", "--stats"])
        .arg(&s)
        .arg(&p)
        .assert()
        .success();
    let text = fs::read_to_string(&s).unwrap();
    assert!(!text.contains("Qx7731SecretMarker"), "{text}");
}

#[test]
fn stats_same_for_j1_and_j4_without_timings() {
    let d = tempfile::tempdir().unwrap();
    for i in 0..6 {
        write(d.path(), &format!("in/{i}.csv"), &"a,b\n".repeat(i + 1));
    }
    let run = |j: &str| {
        let s = d.path().join(format!("s{j}.jsonl"));
        bin()
            .args(["parse", "-j", j, "-o"])
            .arg(d.path().join(format!("out{j}")))
            .arg("--stats")
            .arg(&s)
            .arg(d.path().join("in"))
            .assert()
            .success();
        stats_lines(&s).into_iter().map(untimed).collect::<Vec<_>>()
    };
    assert_eq!(run("1"), run("4"));
}

#[test]
fn stats_unwritable_path_is_a_usage_error() {
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "t.csv", "a\n");
    bin()
        .args(["parse", "--stats"])
        .arg(d.path().join("no/such/dir/s.jsonl"))
        .arg(&p)
        .assert()
        .code(64) // EXIT_USAGE (crates/cli/src/main.rs)
        .stderr(predicate::str::contains("cannot write stats"));
}

#[test]
fn stats_path_equal_to_input_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "a.csv", "x,y\n1,2\n");
    bin()
        .args(["parse", "--stats"])
        .arg(&p)
        .arg(&p)
        .assert()
        .code(64);
    assert_eq!(fs::read_to_string(&p).unwrap(), "x,y\n1,2\n");
}
