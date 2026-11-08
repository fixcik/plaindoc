//! Text formats (contract decision D8): csv/tsv become a table; plain text,
//! source code, json, yaml, logs and xml are kept as is in a code block;
//! Markdown is passed through without parsing.
mod csv;
mod decode;
mod plain;
pub use csv::CsvParser;
pub use decode::{Decoded, decode, decode_with};
#[cfg(feature = "markup")]
pub(crate) use plain::plain_document;
pub use plain::{PlainTextParser, code_language};

use plaindoc_core::{Input, ParseOptions, Result};

use crate::read::read_limited;

/// Reads a text input, never holding more than `opts.max_text_bytes` + 1 bytes;
/// larger inputs fail with TooLarge { size, limit }.
pub(crate) fn read_text(input: Input, opts: &ParseOptions) -> Result<Vec<u8>> {
    read_limited(input, opts.max_text_bytes)
}
