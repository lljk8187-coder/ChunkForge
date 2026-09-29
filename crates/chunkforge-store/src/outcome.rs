//! Result of putting a chunk into a store or [`crate::ChunkSink`].

/// Whether a put wrote a new chunk or reused an existing one (CAS dedup).
///
/// Phase 1–3 used the names `Inserted` / `AlreadyPresent`; Phase 4 locks the
/// public vocabulary to [`Written`](Self::Written) / [`SkippedExists`](Self::SkippedExists)
/// for both [`crate::Store`] and [`crate::ChunkSink`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutOutcome {
    /// Chunk was newly written.
    Written,
    /// Chunk already existed; write skipped (content-addressed dedup).
    SkippedExists,
}

impl PutOutcome {
    /// `true` if a new chunk was written.
    #[must_use]
    pub fn is_new(self) -> bool {
        matches!(self, Self::Written)
    }

    /// `true` if an existing chunk was reused / skipped.
    #[must_use]
    pub fn is_reused(self) -> bool {
        matches!(self, Self::SkippedExists)
    }
}
