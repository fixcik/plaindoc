use crate::Format;

#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("unsupported format: {0}")]
    Unsupported(Format),
    #[error("corrupt document: {0}")]
    Corrupt(String),
    #[error("document is encrypted")]
    Encrypted,
    #[error("input too large: {size} bytes > limit {limit}")]
    TooLarge { size: u64, limit: u64 },
    /// Internal or derived limit exceeded (e.g. decompression bomb); `TooLarge` is for input size.
    #[error("limit exceeded: {kind} > {limit}")]
    Limit { kind: String, limit: u64 },
    #[error("timed out after {0:?}")]
    Timeout(std::time::Duration),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("internal error (parser panicked): {0}")]
    Internal(String),
}

pub type Result<T, E = ParseError> = std::result::Result<T, E>;
