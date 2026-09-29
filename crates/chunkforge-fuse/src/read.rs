//! Map blob byte ranges onto index entries and fetch plaintext from a [`ChunkSource`].

use chunkforge_index::{Index, IndexEntry, entry_length};
use chunkforge_store::{ChunkSource, SourceError};

/// Read up to `size` bytes starting at `offset` from the logical blob described by `index`.
///
/// Walks overlapping index entries, calls [`ChunkSource::get`] for each needed chunk, and
/// splices the requested plaintext slice. Returns an empty buffer on EOF (`offset >= total_size`
/// or `size == 0`).
pub fn read_range(
    index: &Index,
    source: &dyn ChunkSource,
    offset: u64,
    size: u32,
) -> Result<Vec<u8>, SourceError> {
    read_entries(&index.entries, index.total_size, source, offset, size)
}

/// Same as [`read_range`], but operates on a bare chunk table + size (e.g. a `.cfdir` file entry).
pub fn read_entries(
    entries: &[IndexEntry],
    total_size: u64,
    source: &dyn ChunkSource,
    offset: u64,
    size: u32,
) -> Result<Vec<u8>, SourceError> {
    if size == 0 || offset >= total_size {
        return Ok(Vec::new());
    }

    let want_end = offset.saturating_add(u64::from(size)).min(total_size);
    let mut out = Vec::with_capacity((want_end - offset) as usize);
    let mut chunk_start = 0u64;

    for (i, entry) in entries.iter().enumerate() {
        let chunk_end = entry.end_offset;
        if chunk_end <= offset {
            chunk_start = chunk_end;
            continue;
        }
        if chunk_start >= want_end {
            break;
        }

        let plain = source.get(&entry.chunk_id)?;
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
    }

    Ok(out)
}
