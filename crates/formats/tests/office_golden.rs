#![cfg(feature = "office")]
mod common;
use common::*;

#[test]
fn markdown_matches_golden() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let mut diffs = vec![];
    for f in all_fixtures()
        .into_iter()
        .filter(|f| !f.ends_with("encrypted.docx"))
    {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let rel = f
            .strip_prefix(fixture(""))
            .unwrap()
            .to_string_lossy()
            .to_string();
        let md = parse_fixture(&rel).unwrap().to_markdown();
        let golden = fixture(&format!("expected/md/{name}.md"));
        if update {
            std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
            std::fs::write(&golden, &md).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&golden)
            .unwrap_or_else(|_| panic!("no golden for {name}; run with UPDATE_GOLDEN=1"));
        if want != md {
            let line = want
                .lines()
                .zip(md.lines())
                .position(|(a, b)| a != b)
                .unwrap_or(want.lines().count().min(md.lines().count()));
            diffs.push(format!("{name}: first diff at line {}", line + 1));
        }
    }
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
}
