//! Stacked read-through cache in front of a primary [`ChunkSource`].

use crate::Store;
use crate::source::{ChunkSource, SourceError};
use chunkforge_chunk::ChunkId;

/// Read-through cache: check local cache store first; on miss fetch primary and
/// `put` into the cache only (never writes the primary).
///
/// Type parameters:
/// - `P`: primary [`ChunkSource`] (local store, HTTP, `file://`, …)
/// - `S`: cache backend; Phase 2 uses [`Store`] (writable local CAS)
#[derive(Debug)]
pub struct CacheSource<P, S = Store> {
    primary: P,
    cache: S,
}

impl<P> CacheSource<P, Store> {
    /// Wrap `primary` with a writable local `cache` store.
    pub fn new(primary: P, cache: Store) -> Self {
        Self { primary, cache }
    }

    /// Borrow the primary source.
    pub fn primary(&self) -> &P {
        &self.primary
    }

    /// Borrow the cache store.
    pub fn cache(&self) -> &Store {
        &self.cache
    }

    /// Consume into `(primary, cache)`.
    pub fn into_parts(self) -> (P, Store) {
        (self.primary, self.cache)
    }
}

impl<P: ChunkSource> ChunkSource for CacheSource<P, Store> {
    fn has(&self, id: &ChunkId) -> Result<bool, SourceError> {
        if ChunkSource::has(&self.cache, id)? {
            return Ok(true);
        }
        self.primary.has(id)
    }

    fn get(&self, id: &ChunkId) -> Result<Vec<u8>, SourceError> {
        match ChunkSource::get(&self.cache, id) {
            Ok(data) => Ok(data),
            Err(SourceError::NotFound(_)) => {
                let data = self.primary.get(id)?;
                // Fill cache only; never write primary.
                self.cache
                    .put_with_id(id, &data)
                    .map_err(SourceError::from)?;
                Ok(data)
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;
    use crate::meta::Compression;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::tempdir;

    /// Counting wrapper around a [`Store`] used as a fake primary.
    struct CountingSource {
        inner: Store,
        gets: Arc<AtomicUsize>,
    }

    impl ChunkSource for CountingSource {
        fn has(&self, id: &ChunkId) -> Result<bool, SourceError> {
            ChunkSource::has(&self.inner, id)
        }

        fn get(&self, id: &ChunkId) -> Result<Vec<u8>, SourceError> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            ChunkSource::get(&self.inner, id)
        }
    }

    #[test]
    fn cache_miss_fetches_primary_and_fills_cache() {
        let primary_dir = tempdir().unwrap();
        let cache_dir = tempdir().unwrap();
        let primary_store = Store::create(primary_dir.path(), Compression::None).unwrap();
        let data = b"phase2-m3-cache-fill";
        let (id, _) = primary_store.put(data).unwrap();

        let gets = Arc::new(AtomicUsize::new(0));
        let primary = CountingSource {
            inner: primary_store,
            gets: Arc::clone(&gets),
        };
        let cache = Store::create(cache_dir.path(), Compression::None).unwrap();
        assert!(!cache.has(&id));

        let src = CacheSource::new(primary, cache);
        assert_eq!(src.get(&id).unwrap(), data);
        assert_eq!(gets.load(Ordering::SeqCst), 1);
        assert!(src.cache().has(&id));

        // Second get should hit cache (no extra primary get).
        assert_eq!(src.get(&id).unwrap(), data);
        assert_eq!(gets.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cache_has_checks_cache_then_primary() {
        let primary_dir = tempdir().unwrap();
        let cache_dir = tempdir().unwrap();
        let primary_store = Store::create(primary_dir.path(), Compression::None).unwrap();
        let data = b"has-via-primary";
        let (id, _) = primary_store.put(data).unwrap();
        let cache = Store::create(cache_dir.path(), Compression::None).unwrap();

        let src = CacheSource::new(primary_store, cache);
        assert!(src.has(&id).unwrap());
        let missing = ChunkId::hash(b"absent-m3");
        assert!(!src.has(&missing).unwrap());
    }

    #[test]
    fn cache_never_writes_primary() {
        let primary_dir = tempdir().unwrap();
        let cache_dir = tempdir().unwrap();
        // Primary is empty; we will not put there.
        let primary = Store::create(primary_dir.path(), Compression::None).unwrap();
        // Pre-seed cache only.
        let cache = Store::create(cache_dir.path(), Compression::None).unwrap();
        let data = b"cache-only-blob";
        let (id, _) = cache.put(data).unwrap();

        let src = CacheSource::new(primary, cache);
        assert_eq!(src.get(&id).unwrap(), data);
        // Primary still lacks the chunk.
        assert!(!src.primary().has(&id));
    }
}
