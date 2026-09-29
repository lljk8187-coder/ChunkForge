use chunkforge_chunk::ChunkId;
use thiserror::Error;

/// Errors from local CAS store operations.
#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("chunk not found: {0}")]
    NotFound(ChunkId),

    #[error("chunk corrupted (BLAKE3 mismatch): {0}")]
    Corrupt(ChunkId),

    #[error("invalid store metadata: {0}")]
    InvalidMeta(String),

    #[error("not a ChunkForge store: {0}")]
    NotAStore(String),

    #[error("compression not available: store uses {0}, but this build lacks the feature")]
    CompressionUnavailable(String),

    #[error("chunk id mismatch: expected {expected}, got {actual}")]
    IdMismatch { expected: ChunkId, actual: ChunkId },
}

/// Alias for callers that prefer the `StoreError` name.
pub type StoreError = Error;
