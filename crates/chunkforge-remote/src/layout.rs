//! Shared CAS path / URL layout helpers (aligned with local store).

use chunkforge_store::ChunkId;

/// Relative URL/path of a chunk under a store or HTTP base:
/// `chunks/<2hex>/<62hex>.cnk`.
pub fn chunk_http_path(id: &ChunkId) -> String {
    let hex = id.to_hex();
    debug_assert_eq!(hex.len(), 64);
    format!("chunks/{}/{}.cnk", &hex[0..2], &hex[2..])
}

/// Join `base` (no required trailing slash) with the chunk relative path.
pub fn chunk_url(base: &str, id: &ChunkId) -> String {
    let base = base.trim_end_matches('/');
    format!("{}/{}", base, chunk_http_path(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chunkforge_store::ChunkId;

    #[test]
    fn path_and_url_match_store_layout() {
        let id = ChunkId::hash(b"hello");
        let hex = id.to_hex();
        assert_eq!(
            chunk_http_path(&id),
            format!("chunks/{}/{}.cnk", &hex[0..2], &hex[2..])
        );
        assert_eq!(
            chunk_url("http://127.0.0.1:8000/cf-base", &id),
            format!(
                "http://127.0.0.1:8000/cf-base/chunks/{}/{}.cnk",
                &hex[0..2],
                &hex[2..]
            )
        );
        // Trailing slash on base is normalized away (no double slash before chunks/).
        assert_eq!(
            chunk_url("http://127.0.0.1:8000/cf-base/", &id),
            chunk_url("http://127.0.0.1:8000/cf-base", &id)
        );
    }
}
