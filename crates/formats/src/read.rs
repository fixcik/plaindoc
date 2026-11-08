use std::io::Read;

use plaindoc_core::{Input, ParseError, Result};

/// Reads an input, never holding more than `limit` + 1 bytes; larger inputs fail
/// with TooLarge { size, limit } (size is what was read when the real size is
/// unknown, e.g. for pipes).
pub(crate) fn read_limited(input: Input, limit: u64) -> Result<Vec<u8>> {
    let too_large = |size| ParseError::TooLarge { size, limit };
    match input {
        Input::Path(p) => {
            let file = std::fs::File::open(p)?;
            let size = file.metadata()?.len();
            if size > limit {
                return Err(too_large(size));
            }
            let mut buf = Vec::new();
            file.take(limit.saturating_add(1)).read_to_end(&mut buf)?;
            if buf.len() as u64 > limit {
                return Err(too_large(buf.len() as u64));
            }
            Ok(buf)
        }
        input => {
            let bytes = input.into_bytes()?;
            if bytes.len() as u64 > limit {
                return Err(too_large(bytes.len() as u64));
            }
            Ok(bytes)
        }
    }
}
