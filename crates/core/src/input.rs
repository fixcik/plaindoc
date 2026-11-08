use std::path::PathBuf;

use crate::{Document, Format, ParseOptions, ParseStats, Result};

#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Path(PathBuf),
    Bytes(Vec<u8>),
}

impl Input {
    pub fn into_bytes(self) -> Result<Vec<u8>> {
        match self {
            Input::Path(p) => Ok(std::fs::read(p)?),
            Input::Bytes(b) => Ok(b),
        }
    }

    /// Lowercased file extension for `Path`; `None` for `Bytes`.
    pub fn extension(&self) -> Option<String> {
        match self {
            Input::Path(p) => p.extension().map(|e| e.to_string_lossy().to_lowercase()),
            Input::Bytes(_) => None,
        }
    }
}

/// The format is detected once in the registry and passed in.
pub trait Parser: Send + Sync {
    fn formats(&self) -> &[Format];
    fn parse(&self, input: Input, format: Format, opts: &ParseOptions) -> Result<Document>;

    /// [`Parser::parse`] that also fills the parser's in-flight counters in `stats` (stage times,
    /// page counts, limits, OCR, detector input). The registry fills the post-hoc counters from the
    /// returned document. Default: `parse`, `stats` untouched.
    fn parse_with_stats(
        &self,
        input: Input,
        format: Format,
        opts: &ParseOptions,
        stats: &mut ParseStats,
    ) -> Result<Document> {
        let _ = stats;
        self.parse(input, format, opts)
    }
}
