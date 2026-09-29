//! `file://` and plain-path [`ChunkSource`] wrapping a local [`Store`].

use chunkforge_store::{ChunkId, ChunkSource, SourceError, Store};
use std::path::{Path, PathBuf};

/// Parse a store location: `file:///abs/path`, `file://localhost/abs/path`, or a
/// plain filesystem path.
pub fn parse_store_location(spec: &str) -> Result<PathBuf, SourceError> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(SourceError::Backend(
            "empty store location (expected path or file:// URL)".into(),
        ));
    }

    if let Some(rest) = spec.strip_prefix("file://") {
        return parse_file_url_rest(rest);
    }
    // Also accept the uncommon `file:/path` (single slash) form.
    if let Some(rest) = spec.strip_prefix("file:") {
        if rest.starts_with('/') {
            return Ok(PathBuf::from(rest));
        }
        return Err(SourceError::Backend(format!(
            "unsupported file URL: {spec}"
        )));
    }

    Ok(PathBuf::from(spec))
}

fn parse_file_url_rest(rest: &str) -> Result<PathBuf, SourceError> {
    // file:///abs/path  → rest = "/abs/path"
    // file://localhost/abs/path → rest = "localhost/abs/path"
    // file://hostname/abs → reject non-local hosts (Phase 2: local only)
    if rest.starts_with('/') {
        // Absolute path: file:///foo → "/foo"
        return Ok(PathBuf::from(rest));
    }

    // Authority + path
    let (authority, path) = match rest.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => {
            return Err(SourceError::Backend(format!(
                "file URL missing path: file://{rest}"
            )));
        }
    };

    let authority = authority.to_ascii_lowercase();
    if !(authority.is_empty() || authority == "localhost" || authority == "127.0.0.1") {
        return Err(SourceError::Backend(format!(
            "file URL host not supported (local only): {authority}"
        )));
    }
    Ok(PathBuf::from(path))
}

/// [`ChunkSource`] backed by a local CAS store opened from a `file://` URL or path.
#[derive(Debug)]
pub struct FileUrlSource {
    store: Store,
}

impl FileUrlSource {
    /// Open a store from `file:///…` or a plain local path.
    pub fn open(spec: impl AsRef<str>) -> Result<Self, SourceError> {
        let path = parse_store_location(spec.as_ref())?;
        Self::open_path(path)
    }

    /// Open a store at an already-resolved filesystem path.
    pub fn open_path(path: impl AsRef<Path>) -> Result<Self, SourceError> {
        let store = Store::open(path.as_ref()).map_err(SourceError::from)?;
        Ok(Self { store })
    }

    /// Wrap an existing [`Store`].
    pub fn from_store(store: Store) -> Self {
        Self { store }
    }

    /// Borrow the underlying store.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Consume into the underlying store.
    pub fn into_store(self) -> Store {
        self.store
    }
}

impl ChunkSource for FileUrlSource {
    fn has(&self, id: &ChunkId) -> Result<bool, SourceError> {
        ChunkSource::has(&self.store, id)
    }

    fn get(&self, id: &ChunkId) -> Result<Vec<u8>, SourceError> {
        ChunkSource::get(&self.store, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chunkforge_store::Compression;
    use tempfile::tempdir;

    #[test]
    fn parse_file_url_and_plain_path() {
        assert_eq!(
            parse_store_location("file:///tmp/cf-store").unwrap(),
            PathBuf::from("/tmp/cf-store")
        );
        assert_eq!(
            parse_store_location("file://localhost/var/store").unwrap(),
            PathBuf::from("/var/store")
        );
        assert_eq!(
            parse_store_location("/plain/path").unwrap(),
            PathBuf::from("/plain/path")
        );
        assert_eq!(
            parse_store_location("relative/store").unwrap(),
            PathBuf::from("relative/store")
        );
        assert!(parse_store_location("file://remote.example/x").is_err());
        assert!(parse_store_location("").is_err());
    }

    #[test]
    fn file_url_source_roundtrip() {
        let dir = tempdir().unwrap();
        let store = Store::create(dir.path(), Compression::None).unwrap();
        let data = b"phase2-m2-file-url";
        let (id, _) = store.put(data).unwrap();
        drop(store);

        let url = format!("file://{}", dir.path().display());
        let src = FileUrlSource::open(&url).unwrap();
        assert!(src.has(&id).unwrap());
        assert_eq!(src.get(&id).unwrap(), data);

        let plain = FileUrlSource::open(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(plain.get(&id).unwrap(), data);
    }
}
