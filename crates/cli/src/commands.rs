use crate::EXIT_USAGE;
use crate::args::{Cli, Cmd, HiddenArg, OcrArg, OutFormat, PageSeparatorArg, ParseArgs};
use crate::paths::{Collected, collect, output_paths};
use plaindoc_core::{
    Document, Format, HiddenText, Input, Level, MarkdownOptions, OcrMode, PageSeparator,
    ParseError, ParseOptions, ParseStats, image_set_id,
};
use plaindoc_formats::Registry;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const EXIT_OK: u8 = 0;
pub const EXIT_FAILED: u8 = 1;
pub const EXIT_LOW_QUALITY: u8 = 2;

/// failed -> 1; otherwise strict_low_quality -> 2; otherwise 0.
pub fn exit_code(failed: bool, strict_low_quality: bool) -> u8 {
    if failed {
        EXIT_FAILED
    } else if strict_low_quality {
        EXIT_LOW_QUALITY
    } else {
        EXIT_OK
    }
}

/// Writes a line to stderr; errors (e.g. closed stderr) are ignored instead of panicking.
macro_rules! elog {
    ($($arg:tt)*) => {{
        let _ = writeln!(io::stderr(), $($arg)*);
    }};
}

/// Result of writing the output to stdout.
#[derive(Debug, PartialEq, Eq)]
enum Emit {
    Done,
    /// Reader went away (broken pipe): stop quietly.
    Closed,
    Failed(io::ErrorKind),
}

fn emit<W: Write>(w: &mut W, text: &str) -> Emit {
    match w.write_all(text.as_bytes()).and_then(|()| w.flush()) {
        Ok(()) => Emit::Done,
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Emit::Closed,
        Err(e) => Emit::Failed(e.kind()),
    }
}

/// Writes `text` to stdout; returns `false` when output should stop.
/// A write error (other than a closed pipe) is reported and sets `failed`.
fn print(text: &str, failed: &mut bool) -> bool {
    match emit(&mut io::stdout().lock(), text) {
        Emit::Done => true,
        Emit::Closed => false,
        Emit::Failed(kind) => {
            elog!("error: writing output: {}", io::Error::from(kind));
            *failed = true;
            false
        }
    }
}

pub fn run(cli: Cli) -> u8 {
    let registry = registry(&cli);
    match cli.cmd {
        Cmd::Parse(args) => parse(&registry, args),
        Cmd::Detect { paths } => detect(&registry, &paths),
        Cmd::Check { paths } => check(&registry, &paths),
        #[cfg(feature = "ocr")]
        Cmd::Models(crate::args::ModelsCmd::List) => models_list(&cli.ocr_models),
    }
}

#[cfg(feature = "ocr")]
fn models_list(dir: &Option<PathBuf>) -> u8 {
    use plaindoc_formats::ocr::models::{MANIFEST, bundled, load_model, resolve_models_dir};
    let dir = resolve_models_dir(dir.as_deref());
    let mut out = String::new();
    for m in MANIFEST {
        let status = if bundled(m).is_some() {
            "bundled".to_string()
        } else {
            match &dir {
                None => "missing (no models directory)".to_string(),
                Some(d) => {
                    let p = d.join(m.id).join("inference.onnx");
                    match load_model(m, Some(d)) {
                        Ok(_) => format!("{} ok", p.display()),
                        Err(_) if !p.exists() => format!("{} missing", p.display()),
                        Err(e) => format!("{} unusable ({e})", p.display()),
                    }
                }
            }
        };
        out += &format!(
            "{}\t{}\t{}\t{}\t{status}\n",
            m.id, m.version, m.license, m.sha256
        );
    }
    let mut failed = false;
    print(&out, &mut failed);
    if failed { EXIT_FAILED } else { EXIT_OK }
}

#[cfg(feature = "ocr")]
fn registry(cli: &Cli) -> Registry {
    let engine = plaindoc_formats::ocr::paddle_engine(
        cli.ocr_models.clone(),
        cli.ocr_threads.map(usize::from),
    );
    Registry::with_ocr_engine(engine)
}

#[cfg(not(feature = "ocr"))]
fn registry(_cli: &Cli) -> Registry {
    Registry::with_defaults()
}

fn usage(msg: &str) -> u8 {
    elog!("error: {msg}");
    EXIT_USAGE
}

fn options(args: &ParseArgs) -> ParseOptions {
    let mut opts = ParseOptions::default();
    opts.ocr = match args.ocr {
        OcrArg::Auto => OcrMode::Auto,
        OcrArg::Always => OcrMode::Always,
        OcrArg::Never => OcrMode::Never,
    };
    opts.hidden_text = match args.hidden_text {
        HiddenArg::Off => HiddenText::Off,
        HiddenArg::Flag => HiddenText::Flag,
        HiddenArg::Strip => HiddenText::Strip,
    };
    opts.min_quality = args.min_quality;
    opts.links = !args.no_links;
    opts.images.enabled = !args.no_images;
    // Without -o the pictures are only named, their bytes are not kept.
    opts.images.keep_data = args.out.is_some();
    opts
}

/// `image_dir`: the pictures' directory relative to the output file.
fn render(doc: &Document, args: &ParseArgs, image_dir: Option<String>) -> String {
    match args.format {
        OutFormat::Json => doc.to_json() + "\n",
        OutFormat::Markdown => {
            let mut o = MarkdownOptions::default();
            o.warnings = !args.no_warnings;
            o.headers_footers = args.headers_footers;
            o.page_separator = match args.page_separator {
                PageSeparatorArg::Rule => PageSeparator::Rule,
                PageSeparatorArg::Comment => PageSeparator::Comment,
                PageSeparatorArg::None => PageSeparator::None,
            };
            o.image_dir = image_dir;
            doc.to_markdown_with(&o)
        }
    }
}

/// Id lengths tried for the pictures directory, in order.
const ID_LENGTHS: [usize; 3] = [6, 8, 10];

/// Writes the stored pictures of `doc` into `parent/img-<id>/` and returns the directory name
/// (`None` when there are no pictures). The id is a prefix of the picture-set hash
/// ([`image_set_id`]): 6 hex chars; when that directory already holds a different file under one
/// of our names, 8, then 10 are tried. A directory that holds the same bytes is reused.
fn write_images(doc: &Document, parent: &Path) -> io::Result<Option<String>> {
    if doc.images.is_empty() {
        return Ok(None);
    }
    for len in ID_LENGTHS {
        let name = format!("img-{}", image_set_id(&doc.images, len));
        let dir = parent.join(&name);
        let same = |img: &plaindoc_core::StoredImage| {
            std::fs::read(dir.join(&img.file)).is_ok_and(|b| b == *img.data)
        };
        let clash = |img: &plaindoc_core::StoredImage| dir.join(&img.file).exists() && !same(img);
        if doc.images.iter().any(clash) {
            continue;
        }
        std::fs::create_dir_all(&dir)?;
        for img in doc.images.iter().filter(|i| !same(i)) {
            std::fs::write(dir.join(&img.file), &img.data)?;
        }
        return Ok(Some(name));
    }
    Err(io::Error::other(
        "pictures directory name is taken by different files",
    ))
}

fn print_warnings(path: &Path, doc: &Document) {
    let all = doc.warnings.iter().map(|w| (None, w)).chain(
        doc.sections
            .iter()
            .flat_map(|s| s.warnings.iter().map(move |w| (Some(s.index + 1), w))),
    );
    for (section, w) in all.filter(|(_, w)| w.level >= Level::Warn) {
        let at = section
            .map(|i| format!(" [section {i}]"))
            .unwrap_or_default();
        elog!(
            "warning: {}{at}: {:?} ({:?}): {}",
            path.display(),
            w.kind,
            w.level,
            w.message
        );
    }
}

/// Collects files; walk errors are reported and the rest of the batch goes on.
fn gather(args: &[PathBuf], keep: impl Fn(&Path) -> bool) -> Collected {
    let collected = collect(args, keep);
    for e in &collected.errors {
        elog!("error: {e}");
    }
    collected
}

/// Directory entries are kept when their content-detected format has a parser (D8).
fn supported(registry: &Registry) -> impl Fn(&Path) -> bool + '_ {
    move |p| {
        // Only "no parser" is skipped; I/O and other errors stay in the list so `parse` reports them.
        match registry.detect(&Input::Path(p.to_path_buf())) {
            Ok(f) => registry.supports(f),
            Err(ParseError::Unsupported(_)) => false,
            Err(_) => true,
        }
    }
}

/// Destination of `--stats`: unbuffered stderr for `-` (one `write_all` per line), else a file.
struct StatsSink(Box<dyn Write>);

impl StatsSink {
    fn open(p: &Path) -> Result<Self, String> {
        if p == Path::new("-") {
            return Ok(StatsSink(Box::new(io::stderr())));
        }
        std::fs::File::create(p)
            .map(|f| StatsSink(Box::new(io::BufWriter::new(f))))
            .map_err(|e| format!("cannot write stats to {}: {e}", p.display()))
    }

    fn write(&mut self, path: &Path, s: &ParseStats) -> io::Result<()> {
        let file = serde_json::to_string(&path.display().to_string()).map_err(io::Error::other)?;
        let line = format!("{{\"file\":{file},\"stats\":{}}}\n", s.to_json());
        self.0.write_all(line.as_bytes())
    }

    fn finish(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// One parsed file: the result, its statistics and the outcome of writing its pictures.
type Parsed = (
    plaindoc_core::Result<Document>,
    ParseStats,
    io::Result<Option<String>>,
);

/// Parses `files` (in parallel for a batch, as `Registry::parse_many_with_stats`). As soon as a
/// document is ready its pictures are written by `pictures(i, doc)` and dropped from memory, so a
/// batch never holds the pictures of all its files.
fn parse_all(
    registry: &Registry,
    files: &[PathBuf],
    opts: &ParseOptions,
    jobs: Option<usize>,
    pictures: &(dyn Fn(usize, &mut Document) -> io::Result<Option<String>> + Sync),
) -> Result<Vec<Parsed>, String> {
    let one = |i: usize, path: &PathBuf, o: &ParseOptions| {
        let (mut r, s) = registry.parse_with_stats(Input::Path(path.clone()), o);
        let w = r.as_mut().map_or(Ok(None), |d| pictures(i, d));
        (r, s, w)
    };
    if let [path] = files {
        // Single file: parallelism across pages is allowed (D9).
        return Ok(vec![one(0, path, opts)]);
    }
    let mut batch = opts.clone();
    batch.page_parallel = false;
    let span = plaindoc_core::telemetry::Span::current();
    let run = || -> Vec<Parsed> {
        use rayon::prelude::*;
        files
            .par_iter()
            .enumerate()
            .map(|(i, p)| span.in_scope(|| one(i, p, &batch)))
            .collect()
    };
    match jobs {
        None => Ok(run()),
        Some(n) => {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .map_err(|e| format!("cannot start {n} worker threads: {e}"))?;
            Ok(pool.install(run))
        }
    }
}

fn parse(registry: &Registry, args: ParseArgs) -> u8 {
    let collected = gather(&args.paths, supported(registry));
    if args.out.is_none() && (collected.from_dir || collected.files.len() != 1) {
        return usage("several files or a directory require -o DIR");
    }
    if let Some(sp) = args.stats.as_deref().filter(|p| *p != Path::new("-"))
        && let Ok(sc) = std::fs::canonicalize(sp)
        && collected
            .files
            .iter()
            .any(|f| std::fs::canonicalize(f).is_ok_and(|fc| fc == sc))
    {
        return usage("--stats FILE is also an input file");
    }
    let mut sink = match args.stats.as_deref().map(StatsSink::open).transpose() {
        Ok(s) => s,
        Err(e) => return usage(&e),
    };
    let mut opts = options(&args);
    // Batches keep page parallelism off (D9); parse_many forces it too.
    opts.page_parallel = collected.files.len() == 1;
    let suffix = if args.format == OutFormat::Json {
        "json"
    } else {
        "md"
    };
    let cwd = std::env::current_dir().unwrap_or_default();
    let outs = args
        .out
        .as_deref()
        .map(|d| output_paths(&collected.files, &collected.roots, &cwd, d, suffix));
    let pictures = |i: usize, doc: &mut Document| match &outs {
        Some(o) => {
            let w = write_images(doc, o[i].parent().unwrap_or(Path::new("")));
            doc.images = Vec::new();
            w
        }
        None => Ok(None),
    };
    let results = match parse_all(registry, &collected.files, &opts, args.jobs, &pictures) {
        Ok(r) => r,
        Err(e) => return usage(&e),
    };
    let mut failed = !collected.errors.is_empty();
    let mut low = false;
    for (i, (path, (result, stats, pictures))) in collected.files.iter().zip(results).enumerate() {
        if let Some(s) = &mut sink
            && let Err(e) = s.write(path, &stats)
        {
            elog!("error: writing stats: {e}");
            failed = true;
        }
        let doc = match result {
            Ok(d) => d,
            Err(e) => {
                elog!("error: {}: {e}", path.display());
                failed = true;
                continue;
            }
        };
        if !args.no_warnings {
            print_warnings(path, &doc);
        }
        low |= args.min_quality.is_some_and(|m| doc.quality < m);
        match &outs {
            None => {
                if !print(&render(&doc, &args, None), &mut failed) {
                    break;
                }
            }
            Some(outs) => {
                let target = &outs[i];
                // Pictures were written first; a failure there still lets the text be written.
                let (dir, pics) = match pictures {
                    Ok(d) => (d, Ok(())),
                    Err(e) => (None, Err(e)),
                };
                let text = render(&doc, &args, dir);
                let written = target
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(target, text))
                    .and(pics);
                match written {
                    Ok(()) => elog!("{} -> {}", path.display(), target.display()),
                    Err(e) => {
                        elog!("error: {}: {e}", target.display());
                        failed = true;
                    }
                }
            }
        }
    }
    if let Some(s) = &mut sink
        && let Err(e) = s.finish()
    {
        elog!("error: writing stats: {e}");
        failed = true;
    }
    exit_code(failed, args.strict && low)
}

fn detect(registry: &Registry, paths: &[PathBuf]) -> u8 {
    let collected = gather(paths, |_| true);
    let mut failed = !collected.errors.is_empty();
    for path in &collected.files {
        let line = match registry.detect(&Input::Path(path.clone())) {
            Ok(f) | Err(ParseError::Unsupported(f)) if f != Format::Unknown => {
                format!("{}\t{f}\n", path.display())
            }
            Err(ParseError::Encrypted) => format!("{}\tencrypted\n", path.display()),
            Ok(_) | Err(ParseError::Unsupported(_)) => {
                failed = true;
                format!("{}\tunknown\n", path.display())
            }
            Err(e) => {
                elog!("error: {}: {e}", path.display());
                failed = true;
                continue;
            }
        };
        if !print(&line, &mut failed) {
            break;
        }
    }
    exit_code(failed, false)
}

fn check(registry: &Registry, paths: &[PathBuf]) -> u8 {
    let collected = gather(paths, supported(registry));
    let inputs = collected.files.iter().cloned().map(Input::Path).collect();
    let mut opts = ParseOptions::default();
    opts.images.keep_data = false;
    let results = registry.parse_many(inputs, &opts);
    let mut failed = !collected.errors.is_empty();
    for (path, result) in collected.files.iter().zip(results) {
        let line = match result {
            Ok(doc) => {
                print_warnings(path, &doc);
                let n = doc.warnings.len()
                    + doc.sections.iter().map(|s| s.warnings.len()).sum::<usize>();
                let format = doc.metadata.format.unwrap_or(Format::Unknown);
                format!(
                    "{}\t{format}\tquality={:.2}\twarnings={n}\n",
                    path.display(),
                    doc.quality
                )
            }
            Err(e) => {
                failed = true;
                format!("{}\terror\t{e}\n", path.display())
            }
        };
        if !print(&line, &mut failed) {
            break;
        }
    }
    exit_code(failed, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Failing(io::ErrorKind);
    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(self.0.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn batch_writes_pictures_per_file_and_drops_them() {
        let d = tempfile::tempdir().unwrap();
        let html =
            "<html><body><img src=\"data:image/gif;base64,R0lGODlhAQABAAAAACw=\"></body></html>";
        let files: Vec<PathBuf> = (0..3)
            .map(|i| {
                let p = d.path().join(format!("{i}.html"));
                std::fs::write(&p, html).unwrap();
                p
            })
            .collect();
        let seen = std::sync::atomic::AtomicUsize::new(0);
        let pictures = |_: usize, doc: &mut Document| {
            seen.fetch_add(doc.images.len(), std::sync::atomic::Ordering::SeqCst);
            doc.images.clear();
            Ok(None)
        };
        let reg = Registry::with_defaults();
        let got = parse_all(&reg, &files, &ParseOptions::default(), Some(2), &pictures).unwrap();
        assert_eq!(seen.into_inner(), 3);
        assert!(
            got.iter()
                .all(|(r, _, w)| w.is_ok() && r.as_ref().unwrap().images.is_empty())
        );
    }

    #[test]
    fn emit_classifies_write_results() {
        let mut ok = Vec::new();
        assert_eq!(emit(&mut ok, "hi"), Emit::Done);
        assert_eq!(ok, b"hi");
        assert_eq!(
            emit(&mut Failing(io::ErrorKind::BrokenPipe), "x"),
            Emit::Closed
        );
        assert_eq!(
            emit(&mut Failing(io::ErrorKind::StorageFull), "x"),
            Emit::Failed(io::ErrorKind::StorageFull)
        );
    }
}
