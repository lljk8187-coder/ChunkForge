//! Process-local sequential chunk prefetch cache for RO FUSE mounts.
//!
//! Distinct from Store `--cache` (disk). Does **not** change [`ChunkSource`]
//! method signatures. Prefetch runs on the calling thread (no tokio).

use chunkforge_store::{ChunkId, ChunkSource, SourceError};

/// Default: at most **1** subsequent chunk.
pub const DEFAULT_MAX_PREFETCH_CHUNKS: usize = 1;

/// Default hard cap on total cached plaintext bytes: **512 KiB**.
pub const DEFAULT_MAX_PREFETCH_BYTES: usize = 512 * 1024;

/// Process-local, mount-lifetime chunk prefetch cache (not Store `--cache`).
///
/// # Cap (nailed)
///
/// Default: at most **1** subsequent chunk **AND** total cached bytes **≤ 512 KiB**
/// (whichever stricter). If the next chunk alone exceeds the byte budget, it is
/// not prefetched. Cleared on cross-file / seek / backward / non-contiguous read.
#[derive(Debug)]
pub struct PrefetchCache {
    enabled: bool,
    /// Max number of prefetched plaintext chunks retained (default 1).
    max_chunks: usize,
    /// Max total plaintext bytes retained (default 512 KiB).
    max_bytes: usize,
    /// Prefetched plaintext; at most `max_chunks` entries, total ≤ `max_bytes`.
    entries: Vec<(ChunkId, Vec<u8>)>,
    /// Next expected read offset for the sequential forward window.
    next_offset: Option<u64>,
    /// File scope (BlobFs file ino / DirFs inode). `None` = cold window.
    scope: Option<u64>,
}

impl Default for PrefetchCache {
    fn default() -> Self {
        Self::enabled()
    }
}

impl PrefetchCache {
    /// Prefetch **on** with default caps (M1 default for mounts).
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            max_chunks: DEFAULT_MAX_PREFETCH_CHUNKS,
            max_bytes: DEFAULT_MAX_PREFETCH_BYTES,
            entries: Vec::new(),
            next_offset: None,
            scope: None,
        }
    }

    /// Prefetch **off** ≡ 0.9.0 on-demand `get` (CLI: `chunkforge mount --no-prefetch`).
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            max_chunks: DEFAULT_MAX_PREFETCH_CHUNKS,
            max_bytes: DEFAULT_MAX_PREFETCH_BYTES,
            entries: Vec::new(),
            next_offset: None,
            scope: None,
        }
    }

    /// Whether sequential prefetch is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable prefetch; disabling also cold-starts the window.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.invalidate();
        }
    }

    /// Drop cached chunks and sequential window (cold-start).
    pub fn invalidate(&mut self) {
        self.entries.clear();
        self.next_offset = None;
        self.scope = None;
    }

    /// Bytes currently held in the prefetch cache.
    pub fn cached_bytes(&self) -> usize {
        self.entries.iter().map(|(_, b)| b.len()).sum()
    }

    /// Number of cached chunks.
    pub fn cached_chunks(&self) -> usize {
        self.entries.len()
    }

    /// True when this read continues the forward sequential window for `scope`.
    pub fn is_sequential(&self, scope: u64, offset: u64) -> bool {
        self.enabled && self.scope == Some(scope) && self.next_offset == Some(offset)
    }

    /// Hit: remove and return cached plaintext. Miss: [`ChunkSource::get`].
    pub fn take_or_get(
        &mut self,
        id: &ChunkId,
        source: &dyn ChunkSource,
    ) -> Result<Vec<u8>, SourceError> {
        if self.enabled {
            if let Some(pos) = self.entries.iter().position(|(cid, _)| cid == id) {
                let (_, data) = self.entries.remove(pos);
                return Ok(data);
            }
        }
        source.get(id)
    }

    /// Best-effort prefetch of the next chunk after a satisfied sequential read.
    ///
    /// Errors are **ignored** so a prefetch failure never fails the current read.
    ///
    /// Cap: at most `max_chunks` subsequent chunks **AND** total cached bytes
    /// ≤ `max_bytes` (whichever stricter — nailed here).
    pub fn note_sequential_advance(
        &mut self,
        next_id: Option<&ChunkId>,
        next_len: u64,
        source: &dyn ChunkSource,
    ) {
        if !self.enabled {
            return;
        }
        let Some(id) = next_id else {
            return;
        };
        let len = next_len as usize;

        // Cap (whichever stricter): skip if the next chunk alone exceeds byte budget.
        if len > self.max_bytes {
            return;
        }

        // Evict until there is room for one more entry of `len` bytes.
        while !self.entries.is_empty()
            && (self.entries.len() >= self.max_chunks || self.cached_bytes() + len > self.max_bytes)
        {
            self.entries.remove(0);
        }
        if self.entries.len() >= self.max_chunks || self.cached_bytes() + len > self.max_bytes {
            return;
        }
        if self.entries.iter().any(|(cid, _)| cid == id) {
            return;
        }

        match source.get(id) {
            Ok(data) => {
                self.entries.push((*id, data));
            }
            Err(_) => {
                // Prefetch failure must not fail the current satisfied read.
            }
        }
    }

    /// Record the sequential window after a successful read ending at `read_end`.
    pub fn advance_window(&mut self, scope: u64, read_end: u64) {
        if !self.enabled {
            return;
        }
        self.scope = Some(scope);
        self.next_offset = Some(read_end);
    }

    /// Cold-start when the read is not a forward sequential continuation.
    pub fn prepare_for_read(&mut self, scope: u64, offset: u64) {
        if !self.enabled {
            return;
        }
        if !self.is_sequential(scope, offset) {
            self.invalidate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Mem {
        chunks: HashMap<ChunkId, Vec<u8>>,
        gets: Mutex<Vec<ChunkId>>,
    }

    impl ChunkSource for Mem {
        fn has(&self, id: &ChunkId) -> Result<bool, SourceError> {
            Ok(self.chunks.contains_key(id))
        }

        fn get(&self, id: &ChunkId) -> Result<Vec<u8>, SourceError> {
            self.gets.lock().unwrap().push(*id);
            self.chunks
                .get(id)
                .cloned()
                .ok_or(SourceError::NotFound(*id))
        }
    }

    #[test]
    fn take_or_get_hit_skips_source() {
        let id = ChunkId::hash(b"payload");
        let mut src = Mem::default();
        src.chunks.insert(id, b"payload".to_vec());
        let mut cache = PrefetchCache::enabled();
        cache.entries.push((id, b"payload".to_vec()));
        let got = cache.take_or_get(&id, &src).unwrap();
        assert_eq!(got, b"payload");
        assert!(src.gets.lock().unwrap().is_empty());
        assert_eq!(cache.cached_chunks(), 0);
    }

    #[test]
    fn note_advance_respects_byte_cap() {
        let id = ChunkId::hash(&[0u8; 600]);
        let mut src = Mem::default();
        src.chunks.insert(id, vec![0u8; 600]);
        let mut cache = PrefetchCache::enabled();
        // next_len above 512 KiB → must not call get.
        cache.note_sequential_advance(Some(&id), (512 * 1024 + 1) as u64, &src);
        assert_eq!(cache.cached_chunks(), 0);
        assert!(src.gets.lock().unwrap().is_empty());
    }

    #[test]
    fn note_advance_ignores_get_errors() {
        let id = ChunkId::hash(b"missing");
        let src = Mem::default();
        let mut cache = PrefetchCache::enabled();
        cache.note_sequential_advance(Some(&id), 7, &src);
        assert_eq!(cache.cached_chunks(), 0);
    }

    #[test]
    fn prepare_for_read_invalidates_on_seek() {
        let id = ChunkId::hash(b"cached");
        let mut cache = PrefetchCache::enabled();
        cache.entries.push((id, b"cached".to_vec()));
        cache.scope = Some(2);
        cache.next_offset = Some(50);
        // Backward / non-contiguous → cold-start.
        cache.prepare_for_read(2, 0);
        assert_eq!(cache.cached_chunks(), 0);
        assert!(!cache.is_sequential(2, 50));
    }

    #[test]
    fn prepare_for_read_invalidates_on_scope_change() {
        let id = ChunkId::hash(b"cross");
        let mut cache = PrefetchCache::enabled();
        cache.entries.push((id, b"cross".to_vec()));
        cache.scope = Some(10);
        cache.next_offset = Some(0);
        // Cross-file (different scope) → cold-start.
        cache.prepare_for_read(11, 0);
        assert_eq!(cache.cached_chunks(), 0);
    }

    #[test]
    fn disabled_never_prefetches() {
        let id = ChunkId::hash(b"x");
        let mut src = Mem::default();
        src.chunks.insert(id, b"x".to_vec());
        let mut cache = PrefetchCache::disabled();
        cache.note_sequential_advance(Some(&id), 1, &src);
        assert_eq!(cache.cached_chunks(), 0);
        assert!(src.gets.lock().unwrap().is_empty());
        // take_or_get still hits source when disabled.
        let got = cache.take_or_get(&id, &src).unwrap();
        assert_eq!(got, b"x");
        assert_eq!(src.gets.lock().unwrap().len(), 1);
    }
}
