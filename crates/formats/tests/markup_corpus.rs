#![cfg(all(feature = "markup", feature = "odf", feature = "rtf"))]
use plaindoc_core::*;
use plaindoc_formats::Registry;

/// Large public corpora stay outside the repository (see the plan's corpus list).
#[test]
#[ignore = "needs PLAINDOC_MARKUP_CORPUS=<dir>"]
fn local_corpus_has_no_internal_errors_or_hangs() {
    let dir = std::env::var("PLAINDOC_MARKUP_CORPUS").expect("set PLAINDOC_MARKUP_CORPUS");
    let reg = Registry::with_defaults();
    let mut opts = ParseOptions::default();
    opts.timeout = Some(std::time::Duration::from_secs(30));
    let (mut ok, mut warned, mut errs) = (0, 0, vec![]);
    for e in walk(std::path::Path::new(&dir)) {
        let ext = e
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !["html", "htm", "xml", "xls", "odt", "ods", "rtf"].contains(&ext.as_str()) {
            continue;
        }
        match reg.parse(Input::Path(e.clone()), &opts) {
            Ok(d) => {
                ok += 1;
                if !d.warnings.is_empty() || d.sections.iter().any(|s| !s.warnings.is_empty()) {
                    warned += 1;
                }
            }
            Err(ParseError::Internal(m)) => errs.push(format!("INTERNAL {}: {m}", e.display())),
            Err(ParseError::Timeout(_)) => errs.push(format!("TIMEOUT {}", e.display())),
            Err(err) => eprintln!("ERR {}: {err}", e.display()),
        }
    }
    eprintln!("ok={ok} with_warnings={warned}");
    assert!(errs.is_empty(), "{}", errs.join("\n"));
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}
