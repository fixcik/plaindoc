use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

pub struct Collected {
    pub files: Vec<PathBuf>,
    /// For each file, the directory argument it was found in (`None`: given explicitly).
    pub roots: Vec<Option<PathBuf>>,
    pub from_dir: bool,
    /// Directory walk errors (unreadable subdirectory, ...); the rest of the walk goes on.
    pub errors: Vec<String>,
}

/// Directories are walked recursively (sorted by name); only regular files accepted by
/// `keep` are taken from them. Explicit file arguments are always taken.
pub fn collect(args: &[PathBuf], keep: impl Fn(&Path) -> bool) -> Collected {
    let mut files = Vec::new();
    let mut roots = Vec::new();
    let mut errors = Vec::new();
    let mut from_dir = false;
    for arg in args {
        if arg.is_dir() {
            from_dir = true;
            for entry in WalkDir::new(arg).sort_by_file_name() {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        errors.push(match (e.path(), e.io_error()) {
                            (Some(p), Some(io)) => format!("{}: {io}", p.display()),
                            _ => e.to_string(),
                        });
                        continue;
                    }
                };
                if entry.file_type().is_file() && keep(entry.path()) {
                    files.push(entry.into_path());
                    roots.push(Some(arg.clone()));
                }
            }
        } else {
            files.push(arg.clone());
            roots.push(None);
        }
    }
    Collected {
        files,
        roots,
        from_dir,
        errors,
    }
}

/// Output path for each input, joined to `out` and suffixed with `.{suffix}`: an input under
/// `cwd` keeps its path relative to `cwd` (a/x.pdf -> out/a/x.pdf.md); one outside `cwd` (absolute
/// or through `..`) keeps only its name (/abs/y.csv -> out/y.csv.md), or, when found in a
/// directory argument `roots[i]`, that directory's name and its path inside it
/// (/data/corpus/a/x.pdf -> out/corpus/a/x.pdf.md). Root, `.` and `..` components are dropped.
/// Remaining collisions get `~2`, `~3` before the suffix. The result never leaves `out` and never
/// equals one of the `inputs`.
pub fn output_paths(
    inputs: &[PathBuf],
    roots: &[Option<PathBuf>],
    cwd: &Path,
    out: &Path,
    suffix: &str,
) -> Vec<PathBuf> {
    // Case-insensitive filesystems (APFS) must not see two names as distinct.
    let key = |p: &Path| lexical_abs(p, cwd).to_string_lossy().to_lowercase();
    let mut used: HashSet<String> = inputs.iter().map(|p| key(p)).collect();
    let normal = |p: &Path| -> PathBuf {
        p.components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s),
                _ => None,
            })
            .collect()
    };
    inputs
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let abs = lexical_abs(p, cwd);
            let root = roots.get(i).and_then(Option::as_deref);
            let rel: PathBuf = if abs.starts_with(cwd) {
                normal(p.strip_prefix(cwd).unwrap_or(p))
            } else if let Some(root) = root.map(|r| lexical_abs(r, cwd))
                && let Ok(inside) = abs.strip_prefix(&root)
            {
                let name = root.file_name().map(PathBuf::from).unwrap_or_default();
                name.join(normal(inside))
            } else {
                abs.file_name().map(PathBuf::from).unwrap_or_default()
            };
            let base = out.join(rel).into_os_string();
            let mut n = 1;
            loop {
                let mut candidate = base.clone();
                if n > 1 {
                    candidate.push(format!("~{n}"));
                }
                candidate.push(format!(".{suffix}"));
                let candidate = PathBuf::from(candidate);
                if used.insert(key(&candidate)) {
                    break candidate;
                }
                n += 1;
            }
        })
        .collect()
}

/// Absolute form of `p` with `.` and `..` resolved lexically (no filesystem access).
fn lexical_abs(p: &Path, cwd: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in cwd.join(p).components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_relative_structure_and_dedupes() {
        let inputs: Vec<PathBuf> = [
            "a/x.pdf",
            "b/x.pdf",
            "./a/x.pdf",
            "/abs/y.csv",
            "../c.md",
            "/work/d/z.txt",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        let got = output_paths(&inputs, &[], Path::new("/work"), Path::new("out"), "md");
        let want: Vec<PathBuf> = [
            "out/a/x.pdf.md",
            "out/b/x.pdf.md",
            "out/a/x.pdf~2.md",
            "out/y.csv.md",
            "out/c.md.md",
            "out/d/z.txt.md",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn outside_cwd_keeps_the_name_or_the_walked_directory() {
        let inputs: Vec<PathBuf> = [
            "/Users/u/Downloads/x.docx",
            "/data/corpus/a/y.pdf",
            "/data/corpus/z.txt",
            "/other/x.docx",
            "/work/in/w.md",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        let corpus = Some(PathBuf::from("/data/corpus"));
        let roots = [
            None,
            corpus.clone(),
            corpus,
            None,
            Some(PathBuf::from("in")),
        ];
        let got = output_paths(&inputs, &roots, Path::new("/work"), Path::new("out"), "md");
        let want: Vec<PathBuf> = [
            "out/x.docx.md",
            "out/corpus/a/y.pdf.md",
            "out/corpus/z.txt.md",
            "out/x.docx~2.md",
            "out/in/w.md.md",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn names_differing_only_by_case_get_different_paths() {
        let inputs: Vec<PathBuf> = ["Reports/q.txt", "reports/q.txt"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let got = output_paths(&inputs, &[], Path::new("/work"), Path::new("out"), "md");
        assert_eq!(
            got,
            [
                PathBuf::from("out/Reports/q.txt.md"),
                PathBuf::from("out/reports/q.txt~2.md")
            ]
        );
    }

    #[test]
    fn output_never_equals_an_input() {
        let paths = |v: &[&str]| v.iter().map(PathBuf::from).collect::<Vec<_>>();
        let cwd = Path::new("/work");
        let got = output_paths(
            &paths(&["./a.txt", "./A.TXT.md"]),
            &[],
            cwd,
            Path::new("."),
            "md",
        );
        assert_eq!(got, paths(&["./a.txt~2.md", "./A.TXT.md.md"]));
        let got = output_paths(&paths(&["x.json.json"]), &[], cwd, Path::new("."), "json");
        assert_eq!(got, paths(&["./x.json.json.json"]));
        let got = output_paths(
            &paths(&["x.json", "x.json.json"]),
            &[],
            cwd,
            Path::new("/work"),
            "json",
        );
        assert_eq!(
            got,
            paths(&["/work/x.json~2.json", "/work/x.json.json.json"])
        );
    }
}
