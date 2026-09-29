//! Result of putting a chunk into the store.

/// Whether a [`crate::Store::put`] wrote a new chunk file or reused an existing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutOutcome {
    /// Chunk was newly written to the store.
    Inserted,
    /// Chunk already existed; write skipped (content-addressed dedup).
    AlreadyPresent,
}

impl PutOutcome {
    /// `true` if a new `.cnk` file was written.
    #[must_use]
    pub fn is_new(self) -> bool {
        matches!(self, Self::Inserted)
    }

    /// `true` if an existing chunk was reused.
    #[must_use]
    pub fn is_reused(self) -> bool {
        matches!(self, Self::AlreadyPresent)
    }
}
