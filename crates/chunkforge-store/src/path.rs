//! CAS path layout: `chunks/<2hex>/<62hex>.cnk`.

use chunkforge_chunk::ChunkId;
use std::path::{Path, PathBuf};

/// Relative path of a chunk file under the store root.
///
/// `hex = id.to_hex()` (64 lowercase); path is
/// `chunks / &hex[0..2] / format!("{}.cnk", &hex[2..])`.
pub fn chunk_rel_path(id: &ChunkId) -> PathBuf {
    let hex = id.to_hex();
    debug_assert_eq!(hex.len(), 64);
    PathBuf::from("chunks")
        .join(&hex[0..2])
        .join(format!("{}.cnk", &hex[2..]))
}

/// Absolute path of a chunk file.
pub fn chunk_abs_path(store_root: &Path, id: &ChunkId) -> PathBuf {
    store_root.join(chunk_rel_path(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chunkforge_chunk::ChunkId;

    #[test]
    fn layout_matches_spec() {
        let id = ChunkId::hash(b"hello");
        let hex = id.to_hex();
        let rel = chunk_rel_path(&id);
        assert_eq!(
            rel,
            PathBuf::from("chunks")
                .join(&hex[0..2])
                .join(format!("{}.cnk", &hex[2..]))
        );
        assert_eq!(rel.extension().and_then(|e| e.to_str()), Some("cnk"));
    }
}
