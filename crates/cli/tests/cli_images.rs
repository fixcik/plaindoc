//! Pictures in the CLI: `img-<id>/` next to the result with -o, marks only on stdout,
//! `--no-images`, and output names of inputs outside the working directory.
use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::Path;

/// 1x1 red PNG as base64.
const RED_B64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

fn html() -> String {
    format!(
        r#"<html><body><p>Схема ниже</p><img alt="схема" src="data:image/png;base64,{RED_B64}"><img src="https://example.org/x.png"></body></html>"#
    )
}

fn write(dir: &Path, rel: &str, body: &str) -> std::path::PathBuf {
    let p = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, body).unwrap();
    p
}

/// The only `img-<id>` directory in `dir`.
fn img_dir(dir: &Path) -> String {
    let mut found: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("img-"))
        .collect();
    assert_eq!(found.len(), 1, "{found:?}");
    found.remove(0)
}

fn run(cwd: &Path, args: &[&str]) {
    cargo_bin_cmd!("plaindoc")
        .current_dir(cwd)
        .arg("parse")
        .args(args)
        .assert()
        .success();
}

#[test]
fn out_dir_gets_short_id_dir_and_relative_links() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "in/p q.html", &html());
    run(d.path(), &["in/p q.html", "-o", "out"]);
    let out = d.path().join("out/in");
    let dir = img_dir(&out);
    // `img-` and 6 lower-case hex chars; nothing of the (long) file name
    assert_eq!(dir.len(), 10, "{dir}");
    assert!(dir[4..].chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
    let md = fs::read_to_string(out.join("p q.html.md")).unwrap();
    assert_eq!(
        md,
        format!(
            "Схема ниже\n\n![схема]({dir}/image1.png)\n\n![изображение](https://example.org/x.png)\n"
        )
    );
    let png = fs::read(out.join(&dir).join("image1.png")).unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    assert_eq!(png.len(), 69);
    // JSON names the same file (without the directory); no bytes in it
    run(d.path(), &["-f", "json", "in/p q.html", "-o", "out"]);
    let json = fs::read_to_string(out.join("p q.html.json")).unwrap();
    assert!(json.contains("\"file\": \"image1.png\"") && !json.contains(RED_B64));
    assert_eq!(img_dir(&out), dir);
}

#[test]
fn dir_id_is_stable_and_independent_of_file_name() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "a.html", &html());
    write(d.path(), "a long other name (1).html", &html());
    run(d.path(), &["a.html", "-o", "o1"]);
    run(d.path(), &["a.html", "-o", "o1"]);
    run(d.path(), &["a long other name (1).html", "-o", "o2"]);
    assert_eq!(img_dir(&d.path().join("o1")), img_dir(&d.path().join("o2")));
    // two documents with the same pictures in one batch share the directory
    run(
        d.path(),
        &["a.html", "a long other name (1).html", "-o", "o3"],
    );
    assert_eq!(img_dir(&d.path().join("o3")), img_dir(&d.path().join("o1")));
}

#[test]
fn taken_dir_name_gets_a_longer_id() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "a.html", &html());
    run(d.path(), &["a.html", "-o", "o1"]);
    let short = img_dir(&d.path().join("o1"));
    // the 6-char name is taken by another picture under the same file name
    write(d.path(), &format!("o2/{short}/image1.png"), "other");
    run(d.path(), &["a.html", "-o", "o2"]);
    let long = fs::read_dir(d.path().join("o2"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with("img-") && *n != short)
        .unwrap();
    assert_eq!(long.len(), 12, "{long}");
    assert!(long.starts_with(&short));
    let md = fs::read_to_string(d.path().join("o2/a.html.md")).unwrap();
    assert!(md.contains(&format!("({long}/image1.png)")), "{md}");
    assert_eq!(
        fs::read(d.path().join("o2").join(&short).join("image1.png")).unwrap(),
        b"other"
    );
    // the same bytes already there: reused, not lengthened
    run(d.path(), &["a.html", "-o", "o1"]);
    assert_eq!(img_dir(&d.path().join("o1")), short);
}

#[test]
fn stdout_has_marks_only_and_no_images_is_bare() {
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "a.html", &html());
    let out = cargo_bin_cmd!("plaindoc")
        .current_dir(d.path())
        .args(["parse", p.to_str().unwrap()])
        .output()
        .unwrap();
    let md = String::from_utf8(out.stdout).unwrap();
    assert!(md.contains("![схема](image1.png)"), "{md}");
    assert!(!md.contains("base64"), "{md}");
    let out = cargo_bin_cmd!("plaindoc")
        .current_dir(d.path())
        .args(["parse", "--no-images", "a.html", "-o", "o"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let md = fs::read_to_string(d.path().join("o/a.html.md")).unwrap();
    assert_eq!(
        md,
        "Схема ниже\n\n<!-- image: схема -->\n\n<!-- image -->\n"
    );
    assert!(
        fs::read_dir(d.path().join("o")).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("img-"))
    );
}

#[test]
fn absolute_inputs_keep_only_their_names() {
    let src = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let a = write(src.path(), "deep/one/x.html", &html());
    let b = write(
        src.path(),
        "deep/two/x.html",
        "<html><body><p>b</p></body></html>",
    );
    cargo_bin_cmd!("plaindoc")
        .current_dir(cwd.path())
        .args([
            "parse",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "-o",
            "out",
        ])
        .assert()
        .success();
    let out = cwd.path().join("out");
    assert!(out.join("x.html.md").exists());
    assert!(out.join(img_dir(&out)).join("image1.png").exists());
    assert_eq!(fs::read_to_string(out.join("x.html~2.md")).unwrap(), "b\n");
    // a directory outside the working directory keeps its own name and inner structure
    cargo_bin_cmd!("plaindoc")
        .current_dir(cwd.path())
        .args([
            "parse",
            src.path().join("deep").to_str().unwrap(),
            "-o",
            "dir",
        ])
        .assert()
        .success();
    let dir = cwd.path().join("dir/deep");
    assert!(dir.join("one/x.html.md").exists());
    let one = dir.join("one");
    assert!(one.join(img_dir(&one)).join("image1.png").exists());
    assert!(dir.join("two/x.html.md").exists());
}
