//! Write-only chunk sink abstraction shared by local store and (Phase4) HTTP PUT.

use crate::Error;
use crate::PutOutcome;
use crate::Store;
use chunkforge_chunk::ChunkId;
use thiserror::Error;

/// Errors from a [`ChunkSink`] (local CAS, HTTP PUT, …).
#[derive(Debug, Error)]
pub enum SinkError {
    #[error("chunk not found: {0}")]
    NotFound(ChunkId),

    #[error("chunk corrupted (BLAKE3 mismatch): {0}")]
    Corrupt(ChunkId),

    #[error("chunk id mismatch: expected {expected}, got {actual}")]
    IdMismatch { expected: ChunkId, actual: ChunkId },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Backend-specific failure (HTTP status, remote protocol, etc.).
    #[error("{0}")]
    Backend(String),
}

impl From<Error> for SinkError {
    fn from(e: Error) -> Self {
        match e {
            Error::NotFound(id) => Self::NotFound(id),
            Error::Corrupt(id) => Self::Corrupt(id),
            Error::IdMismatch { expected, actual } => Self::IdMismatch { expected, actual },
            Error::Io(io) => Self::Io(io),
            other => Self::Backend(other.to_string()),
        }
    }
}

/// Write face for content-addressed chunks. Kept separate from [`crate::ChunkSource`]
/// so read-only backends are not forced to implement `put`.
///
/// `put` receives **plaintext** bytes. Implementations must verify
/// `blake3(plain) == id` (or an equivalent locked check) and reject mismatches.
/// If the chunk already exists, return [`PutOutcome::SkippedExists`] (idempotent).
pub trait ChunkSink: Send + Sync {
    /// Whether chunk `id` is already present in this sink.
    fn has(&self, id: &ChunkId) -> Result<bool, SinkError>;

    /// Store plaintext `plain` under `id`.
    ///
    /// Must verify `blake3(plain) == id`. Returns [`PutOutcome::Written`] on a
    /// new insert, or [`PutOutcome::SkippedExists`] when the id is already present.
    fn put(&self, id: &ChunkId, plain: &[u8]) -> Result<PutOutcome, SinkError>;
}

impl<T: ChunkSink + ?Sized> ChunkSink for Box<T> {
    fn has(&self, id: &ChunkId) -> Result<bool, SinkError> {
        (**self).has(id)
    }

    fn put(&self, id: &ChunkId, plain: &[u8]) -> Result<PutOutcome, SinkError> {
        (**self).put(id, plain)
    }
}

impl<T: ChunkSink + ?Sized> ChunkSink for std::sync::Arc<T> {
    fn has(&self, id: &ChunkId) -> Result<bool, SinkError> {
        (**self).has(id)
    }

    fn put(&self, id: &ChunkId, plain: &[u8]) -> Result<PutOutcome, SinkError> {
        (**self).put(id, plain)
    }
}

impl ChunkSink for Store {
    fn has(&self, id: &ChunkId) -> Result<bool, SinkError> {
        Ok(Store::has(self, id))
    }

    fn put(&self, id: &ChunkId, plain: &[u8]) -> Result<PutOutcome, SinkError> {
        // Store::put_with_id already verifies blake3(plain) == id.
        Store::put_with_id(self, id, plain).map_err(SinkError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::Compression;
    use std::fs;
    use tempfile::tempdir;

    fn count_cnk(root: &std::path::Path) -> usize {
        let chunks = root.join("chunks");
        let mut n = 0usize;
        let mut stack = vec![chunks];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in rd {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().and_then(|e| e.to_str()) == Some("cnk") {
                    n += 1;
                }
            }
        }
        n
    }

    #[test]
    fn store_as_chunk_sink_put_idempotent_skips_second() {
        let dir = tempdir().unwrap();
        let store = Store::create(dir.path(), Compression::None).unwrap();
        let plain = b"phase4-m1-chunk-sink";
        let id = ChunkId::hash(plain);

        let sink: &dyn ChunkSink = &store;
        assert!(!sink.has(&id).unwrap());

        let first = sink.put(&id, plain).unwrap();
        assert_eq!(first, PutOutcome::Written);
        assert!(sink.has(&id).unwrap());
        assert_eq!(count_cnk(dir.path()), 1);

        let second = sink.put(&id, plain).unwrap();
        assert_eq!(second, PutOutcome::SkippedExists);
        assert_eq!(count_cnk(dir.path()), 1);
        assert_eq!(store.get(&id).unwrap(), plain);
    }

    #[test]
    fn store_as_chunk_sink_rejects_id_mismatch() {
        let dir = tempdir().unwrap();
        let store = Store::create(dir.path(), Compression::None).unwrap();
        let wrong = ChunkId::hash(b"expected");
        let sink: &dyn ChunkSink = &store;
        let err = sink.put(&wrong, b"actual-bytes").unwrap_err();
        assert!(
            matches!(
                err,
                SinkError::IdMismatch {
                    expected,
                    actual
                } if expected == wrong && actual == ChunkId::hash(b"actual-bytes")
            ),
            "{err:?}"
        );
        assert_eq!(count_cnk(dir.path()), 0);
    }

    #[test]
    fn sink_error_from_store_error() {
        let id = ChunkId::hash(b"x");
        assert!(matches!(
            SinkError::from(Error::NotFound(id)),
            SinkError::NotFound(c) if c == id
        ));
        assert!(matches!(
            SinkError::from(Error::Corrupt(id)),
            SinkError::Corrupt(c) if c == id
        ));
        let mm = SinkError::from(Error::IdMismatch {
            expected: id,
            actual: ChunkId::hash(b"y"),
        });
        assert!(matches!(mm, SinkError::IdMismatch { .. }));
        let backend = SinkError::from(Error::InvalidMeta("bad".into()));
        assert!(matches!(backend, SinkError::Backend(_)));
    }
}
