#![cfg(feature = "office")]
use plaindoc_core::*;
use plaindoc_formats::Registry;

#[test]
#[ignore = "needs PLAINDOC_OFFICE_CORPUS=<dir>"]
fn local_corpus_has_no_internal_errors_or_hangs() {
    let dir = std::env::var("PLAINDOC_OFFICE_CORPUS").expect("set PLAINDOC_OFFICE_CORPUS");
    let reg = Registry::with_defaults();
    let mut opts = ParseOptions::default();
    opts.timeout = Some(std::time::Duration::from_secs(30));
    let (mut ok, mut warned, mut errs) = (0, 0, vec![]);
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        let Some(ext) = p.extension().and_then(|x| x.to_str()) else {
            continue;
        };
        if !["doc", "docx", "xls", "xlsx", "ppt", "pptx"]
            .contains(&ext.to_ascii_lowercase().as_str())
        {
            continue;
        }
        match reg.parse(Input::Path(p.clone()), &opts) {
            Ok(d) => {
                ok += 1;
                if !d.warnings.is_empty() || d.sections.iter().any(|s| !s.warnings.is_empty()) {
                    warned += 1
                }
            }
            Err(ParseError::Internal(m)) => errs.push(format!("INTERNAL {}: {m}", p.display())),
            Err(ParseError::Timeout(_)) => errs.push(format!("TIMEOUT {}", p.display())),
            Err(e) => eprintln!("ERR {}: {e:?}", p.display()),
        }
    }
    eprintln!("ok={ok} with_warnings={warned}");
    assert!(ok > 0, "no office files found in {dir}");
    assert!(errs.is_empty(), "{}", errs.join("\n"));
}
