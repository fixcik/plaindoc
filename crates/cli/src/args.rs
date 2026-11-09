use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "plaindoc",
    version,
    about = "Parse documents into Markdown or JSON"
)]
pub struct Cli {
    /// Show panic details instead of suppressing them.
    #[arg(long, global = true)]
    pub verbose: bool,
    /// Directory with OCR models not bundled into the binary (also $PLAINDOC_OCR_MODELS).
    #[arg(long, global = true, value_name = "DIR")]
    pub ocr_models: Option<PathBuf>,
    /// Concurrent OCR sessions, ~0.7 GB each on a mixed stream (default: min(cores, 4)).
    #[arg(long, global = true, value_name = "N", value_parser = clap::value_parser!(u16).range(1..))]
    pub ocr_threads: Option<u16>,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Parse documents and print or write Markdown/JSON.
    Parse(ParseArgs),
    /// Print the detected format of each file.
    Detect {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    /// Parse and print quality and warnings, without content.
    Check {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    /// OCR models: list the pinned models and where they are found.
    #[cfg(feature = "ocr")]
    #[command(subcommand)]
    Models(ModelsCmd),
}

#[cfg(feature = "ocr")]
#[derive(Subcommand, Debug)]
pub enum ModelsCmd {
    /// One line per model: id, version, license, sha256, status (bundled | <path> ok | missing | bad).
    List,
}

#[derive(Args, Debug)]
pub struct ParseArgs {
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,
    #[arg(short = 'f', long = "format", value_enum, default_value_t = OutFormat::Markdown)]
    pub format: OutFormat,
    /// OCR: where text is missing (auto), everything (always), nothing (never), or as auto plus
    /// the images on PDF pages with a text layer (images). Under auto such images are not
    /// recognised: they are kept as pictures and the page gets an info warning.
    #[arg(long, value_enum, default_value_t = OcrArg::Auto)]
    pub ocr: OcrArg,
    /// Hidden text (white on white, microscopic, hidden layers, invisible Unicode): remove, flag or keep.
    #[arg(long, value_enum, default_value_t = HiddenArg::Strip)]
    pub hidden_text: HiddenArg,
    /// Write results into DIR (required for several files or a directory). Pictures are written as
    /// files only with -o; without it stderr gets one note per document with pictures.
    #[arg(short = 'o', long = "out", value_name = "DIR")]
    pub out: Option<PathBuf>,
    /// Number of files parsed in parallel (0 = rayon default).
    #[arg(short = 'j', long = "jobs", value_name = "N")]
    pub jobs: Option<usize>,
    /// Pages of one PDF parsed at once (default: min(cores, 6)); only for a single input file,
    /// several files run in parallel (-j) with each file's pages one by one. Pages that need OCR
    /// are bounded by --ocr-threads instead.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u16).range(1..))]
    pub page_threads: Option<u16>,
    /// Quality threshold (0..=1) for --strict.
    #[arg(long, value_name = "X", value_parser = parse_quality)]
    pub min_quality: Option<f32>,
    /// Exit with 2 when a document is below --min-quality (and nothing failed).
    #[arg(long, requires = "min_quality")]
    pub strict: bool,
    /// Do not print warnings to stderr nor embed them in Markdown.
    #[arg(long)]
    pub no_warnings: bool,
    /// Do not read link annotations (PDF): no `[text](url)` in Markdown, no `links` in JSON.
    #[arg(long)]
    pub no_links: bool,
    /// Do not extract embedded pictures (office, ODF, RTF, HTML, PDF): bare `<!-- image -->` marks.
    /// With -o DIR pictures go to `img-<id>/imageN.<ext>` next to the result (<id>: 6 hex chars of the pictures' hash); without -o they
    /// are named in the links but not written.
    #[arg(long)]
    pub no_images: bool,
    /// Embed running headers, footers and page numbers in Markdown as HTML comments (always in JSON).
    #[arg(long)]
    pub headers_footers: bool,
    /// Page separator in Markdown: a `---` rule between pages, the `<!-- page N -->` comment, or nothing.
    #[arg(long, value_enum, default_value_t = PageSeparatorArg::Rule)]
    pub page_separator: PageSeparatorArg,
    /// Write per-file parse statistics as JSON Lines to FILE ('-' = stderr): times, page counts,
    /// limits, warning counts; never document content.
    #[arg(long, value_name = "FILE")]
    pub stats: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum OutFormat {
    Markdown,
    Json,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum OcrArg {
    Auto,
    Always,
    Never,
    Images,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum PageSeparatorArg {
    Rule,
    Comment,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum HiddenArg {
    Off,
    Flag,
    Strip,
}

fn parse_quality(s: &str) -> Result<f32, String> {
    let v: f32 = s.parse().map_err(|e| format!("{e}"))?;
    if v.is_finite() && (0.0..=1.0).contains(&v) {
        Ok(v)
    } else {
        Err("must be a number between 0 and 1".into())
    }
}
