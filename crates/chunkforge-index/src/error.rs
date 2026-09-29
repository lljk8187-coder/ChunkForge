use thiserror::Error;

/// Errors from `.cfidx` encode / decode / validation.
#[derive(Debug, Error)]
pub enum Error {
    #[error(
        "unsupported .cfidx major version {found} (expected {expected}); please upgrade chunkforge"
    )]
    UnsupportedMajor { found: u8, expected: u8 },

    #[error("not a ChunkForge index (.cfidx): bad magic")]
    BadMagic,

    #[error("truncated or incomplete .cfidx: {0}")]
    Truncated(String),

    #[error("trailer checksum mismatch (file truncated or corrupted)")]
    TrailerMismatch,

    #[error("invalid index structure: {0}")]
    InvalidStructure(String),

    #[error("unsupported format_version {found} (this build writes/reads {supported})")]
    UnsupportedFormatVersion { found: u16, supported: u16 },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Alias for callers that prefer the `IndexError` name.
pub type IndexError = Error;
