//! Map blob byte ranges onto index entries and fetch plaintext from a [`ChunkSource`].

use crate::prefetch::PrefetchCache;
use chunkforge_index::{Index, IndexEntry, entry_length};
use chunkforge_store::{ChunkSource, SourceError};

/// Read up to `size` bytes starting at `offset` from the logical blob described by `index`.
///
/// Walks overlapping index entries, calls [`ChunkSource::get`] for each needed chunk, and
/// splices the requested plaintext slice. Returns an empty buffer on EOF (`offset >= total_size`
/// or `size == 0`).
///
/// This path does **not** use sequential prefetch (on-demand only). Prefer
/// [`read_range_cached`] / [`BlobFs::read_at`](crate::BlobFs::read_at) for mount reads.
pub fn read_range(
    index: &Index,
    source: &dyn ChunkSource,
    offset: u64,
    size: u32,
) -> Result<Vec<u8>, SourceError> {
    read_entries(&index.entries, index.total_size, source, offset, size)
}

/// Same as [`read_range`], but operates on a bare chunk table + size (e.g. a `.cfdir` file entry).
///
/// On-demand only (no prefetch). See [`read_entries_cached`] for the mount path.
pub fn read_entries(
    entries: &[IndexEntry],
    total_size: u64,
    source: &dyn ChunkSource,
    offset: u64,
    size: u32,
) -> Result<Vec<u8>, SourceError> {
    read_entries_inner(entries, total_size, source, offset, size, None, 0)
}

/// Like [`read_range`], but uses process-local [`PrefetchCache`] for sequential forward reads.
pub fn read_range_cached(
    index: &Index,
    source: &dyn ChunkSource,
    offset: u64,
    size: u32,
    cache: &mut PrefetchCache,
    scope: u64,
) -> Result<Vec<u8>, SourceError> {
    read_entries_cached(
        &index.entries,
        index.total_size,
        source,
        offset,
        size,
        cache,
        scope,
    )
}

/// Like [`read_entries`], with sequential prefetch into `cache`.
///
/// After a forward sequential read that consumes into/past a chunk, the **next**
/// index entry is best-effort prefetched (errors ignored). Cross-file / seek /
/// backward / non-contiguous `offset` cold-starts the window via `scope`.
pub fn read_entries_cached(
    entries: &[IndexEntry],
    total_size: u64,
    source: &dyn ChunkSource,
    offset: u64,
    size: u32,
    cache: &mut PrefetchCache,
    scope: u64,
) -> Result<Vec<u8>, SourceError> {
    read_entries_inner(
        entries,
        total_size,
        source,
        offset,
        size,
        Some(cache),
        scope,
    )
}

fn read_entries_inner(
    entries: &[IndexEntry],
    total_size: u64,
    source: &dyn ChunkSource,
    offset: u64,
    size: u32,
    mut cache: Option<&mut PrefetchCache>,
    scope: u64,
) -> Result<Vec<u8>, SourceError> {
    if size == 0 || offset >= total_size {
        if let Some(c) = cache.as_mut() {
            // Non-data read still counts as a discontinuity if offset ≠ window.
            c.prepare_for_read(scope, offset);
        }
        return Ok(Vec::new());
    }

    if let Some(c) = cache.as_mut() {
        c.prepare_for_read(scope, offset);
    }

    let want_end = offset.saturating_add(u64::from(size)).min(total_size);
    let mut out = Vec::with_capacity((want_end - offset) as usize);
    let mut chunk_start = 0u64;
    let mut last_consumed_idx: Option<usize> = None;

    for (i, entry) in entries.iter().enumerate() {
        let chunk_end = entry.end_offset;
        if chunk_end <= offset {
            chunk_start = chunk_end;
            continue;
        }
        if chunk_start >= want_end {
            break;
        }

        let plain = match cache.as_mut() {
            Some(c) => c.take_or_get(&entry.chunk_id, source)?,
            None => source.get(&entry.chunk_id)?,
        };
        let expected = entry_length(entries, i).expect("entry index in range");
        if plain.len() as u64 != expected {
            return Err(SourceError::Backend(format!(
                "chunk {} length mismatch: got {}, expected {expected}",
                entry.chunk_id,
                plain.len()
            )));
        }

        let from = (offset.max(chunk_start) - chunk_start) as usize;
        let to = (want_end.min(chunk_end) - chunk_start) as usize;
        out.extend_from_slice(&plain[from..to]);
        chunk_start = chunk_end;
        last_consumed_idx = Some(i);
    }

    if let Some(c) = cache.as_mut() {
        let read_end = offset + out.len() as u64;
        c.advance_window(scope, read_end);

        // After satisfying the current request: prefetch the next chunk after the
        // last one this read consumed into/past (best-effort; errors ignored).
        if let Some(i) = last_consumed_idx {
            let next_i = i + 1;
            if next_i < entries.len() {
                let next_id = entries[next_i].chunk_id;
                let next_len = entry_length(entries, next_i).unwrap_or(0);
                c.note_sequential_advance(Some(&next_id), next_len, source);
            }
        }
    }

    Ok(out)
}
