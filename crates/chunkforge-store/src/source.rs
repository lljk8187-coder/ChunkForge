//! Read-only chunk source abstraction shared by local store, remote, and FUSE.

use crate::Error;
use crate::Store;
use chunkforge_chunk::ChunkId;
use thiserror::Error;

/// Errors from a [`ChunkSource`] (local CAS, HTTP, `file://`, cache layers).
#[derive(Debug, Error)]
pub enum SourceError {
    #[error("chunk not found: {0}")]
    NotFound(ChunkId),

    #[error("chunk corrupted (BLAKE3 mismatch): {0}")]
    Corrupt(ChunkId),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Backend-specific failure (HTTP status, URL parse, remote protocol, etc.).
    #[error("{0}")]
    Backend(String),
}

impl From<Error> for SourceError {
    fn from(e: Error) -> Self {
        match e {
            Error::NotFound(id) => Self::NotFound(id),
            Error::Corrupt(id) => Self::Corrupt(id),
            Error::Io(io) => Self::Io(io),
            other => Self::Backend(other.to_string()),
        }
    }
}

/// Read-only chunk face shared by local CAS / HTTP / `file://` / stacked cache.
///
/// `get` must return **plaintext** bytes. Implementations decompress according
/// to the source format (if any) and should verify BLAKE3 over the plaintext.
pub trait ChunkSource: Send + Sync {
    /// Whether chunk `id` is available from this source.
    ///
    /// Local stores map existence checks to infallible disk lookups; remote
    /// backends may return `Err` on transport failure.
    fn has(&self, id: &ChunkId) -> Result<bool, SourceError>;

    /// Fetch plaintext bytes for `id`.
    ///
    /// Implementations are responsible for decompressing on-disk/wire encodings
    /// and are encouraged to verify `blake3(plain) == id`.
    fn get(&self, id: &ChunkId) -> Result<Vec<u8>, SourceError>;
}

impl<T: ChunkSource + ?Sized> ChunkSource for Box<T> {
    fn has(&self, id: &ChunkId) -> Result<bool, SourceError> {
        (**self).has(id)
    }

    fn get(&self, id: &ChunkId) -> Result<Vec<u8>, SourceError> {
        (**self).get(id)
    }
}

impl<T: ChunkSource + ?Sized> ChunkSource for std::sync::Arc<T> {
    fn has(&self, id: &ChunkId) -> Result<bool, SourceError> {
        (**self).has(id)
    }

    fn get(&self, id: &ChunkId) -> Result<Vec<u8>, SourceError> {
        (**self).get(id)
    }
}

impl ChunkSource for Store {
    fn has(&self, id: &ChunkId) -> Result<bool, SourceError> {
        Ok(Store::has(self, id))
    }

    fn get(&self, id: &ChunkId) -> Result<Vec<u8>, SourceError> {
        Store::get(self, id).map_err(SourceError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::Compression;
    use tempfile::tempdir;

    #[test]
    fn store_as_chunk_source_roundtrip() {
        let dir = tempdir().unwrap();
        let store = Store::create(dir.path(), Compression::None).unwrap();
        let data = b"phase2-m1-chunk-source";
        let (id, _) = store.put(data).unwrap();

        let src: &dyn ChunkSource = &store;
        assert!(src.has(&id).unwrap());
        assert_eq!(src.get(&id).unwrap(), data);

        let missing = ChunkId::hash(b"absent");
        assert!(!src.has(&missing).unwrap());
        assert!(matches!(
            src.get(&missing),
            Err(SourceError::NotFound(c)) if c == missing
        ));
    }

    #[test]
    fn store_as_chunk_source_corrupt_maps_to_source_error() {
        let dir = tempdir().unwrap();
        let store = Store::create(dir.path(), Compression::None).unwrap();
        let data = b"corrupt-via-trait";
        let (id, _) = store.put(data).unwrap();
        let path = store.chunk_path(&id);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[0] ^= 0xff;
        std::fs::write(&path, &bytes).unwrap();

        let src: &dyn ChunkSource = &store;
        assert!(matches!(
            src.get(&id),
            Err(SourceError::Corrupt(c)) if c == id
        ));
    }

    #[test]
    fn source_error_from_store_error() {
        let id = ChunkId::hash(b"x");
        assert!(matches!(
            SourceError::from(Error::NotFound(id)),
            SourceError::NotFound(c) if c == id
        ));
        assert!(matches!(
            SourceError::from(Error::Corrupt(id)),
            SourceError::Corrupt(c) if c == id
        ));
        let backend = SourceError::from(Error::InvalidMeta("bad".into()));
        assert!(matches!(backend, SourceError::Backend(_)));
    }
}
