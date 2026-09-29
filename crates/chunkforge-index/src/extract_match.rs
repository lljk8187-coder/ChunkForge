//! Extract destination vs listing match helpers (Phase 9 `--skip-unchanged`).
//!
//! Size fast-reject then content BLAKE3 ≡ listing `blob_blake3`. Does **not**
//! consult mtime (P0). Does **not** change `.cfdir` / `.cfidx` v1 byte layouts.

use crate::hash_reader;
use chunkforge_chunk::ChunkId;
use std::fs::{self, File};
use std::io;
use std::path::Path;

/// Local destination vs listing File entry verdict for extract skip.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum UnchangedVerdict {
    /// Destination path does not exist.
    Missing,
    /// Exists but is not a regular file (directory / special) — type conflict.
    TypeMismatch,
    /// Regular file whose length ≠ listing size (no content hash).
    SizeMismatch,
    /// Size matches but content BLAKE3 ≠ listing `blob_blake3`.
    ContentMismatch,
    /// Size + content BLAKE3 match → safe to skip fetch and write.
    Unchanged,
}

/// Judge whether `dest` already matches a listing File entry.
///
/// 1. Missing path → [`UnchangedVerdict::Missing`].
/// 2. Exists but not a regular file → [`UnchangedVerdict::TypeMismatch`].
/// 3. `metadata.len() != size` → [`UnchangedVerdict::SizeMismatch`] (no read).
/// 4. Stream-hash content; equal to `blob_blake3` → [`UnchangedVerdict::Unchanged`],
///    else [`UnchangedVerdict::ContentMismatch`].
///
/// Uses `fs::metadata` (follows symlinks) so type checks align with extract's
/// existing `is_dir` / overwrite gates. Does **not** trust mtime.
pub fn judge_extract_unchanged(
    dest: &Path,
    size: u64,
    blob_blake3: &ChunkId,
) -> io::Result<UnchangedVerdict> {
    let meta = match fs::metadata(dest) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(UnchangedVerdict::Missing);
        }
        Err(e) => return Err(e),
    };

    if !meta.is_file() {
        return Ok(UnchangedVerdict::TypeMismatch);
    }

    if meta.len() != size {
        return Ok(UnchangedVerdict::SizeMismatch);
    }

    let mut file = File::open(dest)?;
    let blob = hash_reader(&mut file).map_err(|e| match e {
        crate::Error::Io(ioe) => ioe,
        other => io::Error::other(other.to_string()),
    })?;

    if &blob == blob_blake3 {
        Ok(UnchangedVerdict::Unchanged)
    } else {
        Ok(UnchangedVerdict::ContentMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct TmpDir(std::path::PathBuf);
    impl TmpDir {
        fn new() -> Self {
            let base = std::env::temp_dir().join(format!(
                "cf-extract-match-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&base).unwrap();
            Self(base)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_file(dir: &Path, name: &str, data: &[u8]) -> std::path::PathBuf {
        let p = dir.join(name);
        let mut f = File::create(&p).unwrap();
        f.write_all(data).unwrap();
        f.sync_all().unwrap();
        p
    }

    #[test]
    fn missing_path() {
        let dir = TmpDir::new();
        let p = dir.path().join("nope.txt");
        let blob = ChunkId::hash(b"x");
        assert_eq!(
            judge_extract_unchanged(&p, 1, &blob).unwrap(),
            UnchangedVerdict::Missing
        );
    }

    #[test]
    fn type_mismatch_directory() {
        let dir = TmpDir::new();
        let sub = dir.path().join("subdir");
        fs::create_dir(&sub).unwrap();
        let blob = ChunkId::hash(b"");
        assert_eq!(
            judge_extract_unchanged(&sub, 0, &blob).unwrap(),
            UnchangedVerdict::TypeMismatch
        );
    }

    #[test]
    fn size_mismatch_fast_reject() {
        let dir = TmpDir::new();
        let p = write_file(dir.path(), "a.txt", b"hello");
        let wrong = ChunkId::hash(b"other");
        assert_eq!(
            judge_extract_unchanged(&p, 999, &wrong).unwrap(),
            UnchangedVerdict::SizeMismatch
        );
    }

    #[test]
    fn content_match_unchanged() {
        let dir = TmpDir::new();
        let data = b"hello-extract-skip";
        let p = write_file(dir.path(), "a.txt", data);
        let blob = ChunkId::hash(data);
        assert_eq!(
            judge_extract_unchanged(&p, data.len() as u64, &blob).unwrap(),
            UnchangedVerdict::Unchanged
        );
    }

    #[test]
    fn content_mismatch_same_size() {
        let dir = TmpDir::new();
        let p = write_file(dir.path(), "a.txt", b"AAAA");
        let other = ChunkId::hash(b"BBBB");
        assert_eq!(
            judge_extract_unchanged(&p, 4, &other).unwrap(),
            UnchangedVerdict::ContentMismatch
        );
    }

    #[test]
    fn empty_file_unchanged() {
        let dir = TmpDir::new();
        let p = write_file(dir.path(), "empty", b"");
        let blob = ChunkId::hash(b"");
        assert_eq!(
            judge_extract_unchanged(&p, 0, &blob).unwrap(),
            UnchangedVerdict::Unchanged
        );
    }
}
