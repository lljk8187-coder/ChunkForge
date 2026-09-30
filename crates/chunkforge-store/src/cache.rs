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
///
/// Soft budget (Phase 15): optional [`max_bytes`](CacheSource::with_max_bytes)
/// caps fill using `cache.stats().bytes_on_disk`. Over budget skips `put` but
/// still returns primary plaintext. Never evicts / removes existing cache
/// entries. `None` / [`CacheSource::new`] ≡ 1.4 unbounded fill.
#[derive(Debug)]
pub struct CacheSource<P, S = Store> {
    primary: P,
    cache: S,
    /// Soft fill budget over `cache.stats().bytes_on_disk`. `None` = unbounded.
    max_bytes: Option<u64>,
}

impl<P> CacheSource<P, Store> {
    /// Wrap `primary` with a writable local `cache` store (unbounded fill ≡ 1.4).
    pub fn new(primary: P, cache: Store) -> Self {
        Self {
            primary,
            cache,
            max_bytes: None,
        }
    }

    /// Wrap `primary` with a writable local `cache` and an optional soft fill
    /// budget. `None` ≡ [`CacheSource::new`] (1.4 unbounded). When `Some(max)`,
    /// a miss fill is skipped if
    /// `cache.stats().bytes_on_disk + plaintext.len() as u64 > max`; the get
    /// still returns primary plaintext. Existing cache entries are never
    /// removed.
    pub fn with_max_bytes(primary: P, cache: Store, max_bytes: Option<u64>) -> Self {
        Self {
            primary,
            cache,
            max_bytes,
        }
    }

    /// Soft fill budget, if any (`None` = unbounded).
    pub fn max_bytes(&self) -> Option<u64> {
        self.max_bytes
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
                // Soft budget: skip fill when adding this plaintext would
                // exceed max; never evict. Still return primary data.
                if let Some(max) = self.max_bytes {
                    let on_disk = self
                        .cache
                        .stats()
                        .map_err(SourceError::from)?
                        .bytes_on_disk;
                    if on_disk.saturating_add(data.len() as u64) > max {
                        return Ok(data);
                    }
                }
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

    #[test]
    fn soft_budget_second_chunk_skips_fill_but_get_succeeds() {
        let primary_dir = tempdir().unwrap();
        let cache_dir = tempdir().unwrap();
        let primary = Store::create(primary_dir.path(), Compression::None).unwrap();
        // Compression::None → bytes_on_disk == plaintext len per chunk.
        let a = vec![b'a'; 100];
        let b = vec![b'b'; 100];
        let (id_a, _) = primary.put(&a).unwrap();
        let (id_b, _) = primary.put(&b).unwrap();
        let cache = Store::create(cache_dir.path(), Compression::None).unwrap();

        // max=150: first 100 fits (0+100<=150); after fill on_disk≈100;
        // second 100+100>150 → refuse fill.
        let src = CacheSource::with_max_bytes(primary, cache, Some(150));
        assert_eq!(src.max_bytes(), Some(150));

        assert_eq!(src.get(&id_a).unwrap(), a);
        assert!(src.cache().has(&id_a), "first chunk should fill cache");
        let after_a = src.cache().stats().unwrap().bytes_on_disk;
        assert_eq!(after_a, 100);

        assert_eq!(src.get(&id_b).unwrap(), b, "get must still succeed from primary");
        assert!(
            !src.cache().has(&id_b),
            "second chunk must not enter cache under soft budget"
        );
        assert_eq!(
            src.cache().stats().unwrap().bytes_on_disk,
            after_a,
            "bytes_on_disk must not grow after refused fill"
        );
        // Never removes the first chunk.
        assert!(src.cache().has(&id_a));
    }

    #[test]
    fn soft_budget_none_or_new_fills_unbounded_like_1_4() {
        let primary_dir = tempdir().unwrap();
        let cache_dir = tempdir().unwrap();
        let primary = Store::create(primary_dir.path(), Compression::None).unwrap();
        let a = b"budget-none-chunk-a".to_vec();
        let b = b"budget-none-chunk-b".to_vec();
        let (id_a, _) = primary.put(&a).unwrap();
        let (id_b, _) = primary.put(&b).unwrap();
        let cache = Store::create(cache_dir.path(), Compression::None).unwrap();

        let src = CacheSource::with_max_bytes(primary, cache, None);
        assert_eq!(src.max_bytes(), None);
        assert_eq!(src.get(&id_a).unwrap(), a);
        assert_eq!(src.get(&id_b).unwrap(), b);
        assert!(src.cache().has(&id_a));
        assert!(src.cache().has(&id_b));
    }

    #[test]
    fn soft_budget_new_equiv_unbounded() {
        let primary_dir = tempdir().unwrap();
        let cache_dir = tempdir().unwrap();
        let primary = Store::create(primary_dir.path(), Compression::None).unwrap();
        let a = b"new-equiv-a".to_vec();
        let b = b"new-equiv-b".to_vec();
        let (id_a, _) = primary.put(&a).unwrap();
        let (id_b, _) = primary.put(&b).unwrap();
        let cache = Store::create(cache_dir.path(), Compression::None).unwrap();

        let src = CacheSource::new(primary, cache);
        assert_eq!(src.max_bytes(), None);
        assert_eq!(src.get(&id_a).unwrap(), a);
        assert_eq!(src.get(&id_b).unwrap(), b);
        assert!(src.cache().has(&id_a));
        assert!(src.cache().has(&id_b));
    }

    #[test]
    fn soft_budget_existing_cache_hit_unaffected() {
        let primary_dir = tempdir().unwrap();
        let cache_dir = tempdir().unwrap();
        let primary = Store::create(primary_dir.path(), Compression::None).unwrap();
        let data = b"already-cached-under-tiny-budget";
        let (id, _) = primary.put(data).unwrap();
        let cache = Store::create(cache_dir.path(), Compression::None).unwrap();
        // Pre-seed cache so get is a hit even with max=0 (which refuses any fill).
        cache.put(data).unwrap();
        assert!(cache.has(&id));

        let src = CacheSource::with_max_bytes(primary, cache, Some(0));
        assert_eq!(src.get(&id).unwrap(), data);
        assert!(src.cache().has(&id), "hit must not remove existing entry");
    }
}
