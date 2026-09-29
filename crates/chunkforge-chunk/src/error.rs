use thiserror::Error;

/// Errors from chunking / parameter validation / ChunkId parsing.
#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid chunk parameters: {0}")]
    InvalidParams(String),

    #[error("invalid ChunkId: {0}")]
    InvalidChunkId(String),

    #[error("I/O error while chunking: {0}")]
    Io(#[from] std::io::Error),

    #[error("chunker error: {0}")]
    Chunker(String),
}

/// Alias kept for callers that prefer the `ChunkError` name.
pub type ChunkError = Error;
