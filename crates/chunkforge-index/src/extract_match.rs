//! Extract destination vs listing match helpers (Phase 9 `--skip-unchanged`;
//! Phase 11 `--skip-trust-mtime`).
//!
//! Size fast-reject, optional mtime trust fast-path, then content BLAKE3 ≡
//! listing `blob_blake3`. Does **not** change `.cfdir` / `.cfidx` v1 byte layouts.

use crate::hash_reader;
use chunkforge_chunk::ChunkId;
use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::time::UNIX_EPOCH;

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
    /// Size (+ optional mtime) + content BLAKE3 match → safe to skip fetch and write.
    Unchanged,
}

/// Judge whether `dest` already matches a listing File entry.
///
/// Equivalent to [`judge_extract_unchanged_opts`] with `listing_mtime_secs = 0`
/// and `trust_mtime = false` (≡ 1.0.0 / Phase 9 content path).
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
    judge_extract_unchanged_opts(
        dest,
        size,
        blob_blake3,
        /* listing_mtime_secs */ 0,
        false,
    )
}

/// Judge dest vs listing with optional mtime trust (Phase 11 `--skip-trust-mtime`).
///
/// Decision order:
/// 1. Missing path → [`UnchangedVerdict::Missing`].
/// 2. Exists but not a regular file → [`UnchangedVerdict::TypeMismatch`].
/// 3. `metadata.len() != size` → [`UnchangedVerdict::SizeMismatch`] (no content read).
/// 4. **`trust_mtime` and dest `mtime_secs` == `listing_mtime_secs`** →
///    [`UnchangedVerdict::Unchanged`] **without** reading content / computing BLAKE3.
/// 5. else stream-hash content vs `blob_blake3` (≡ [`judge_extract_unchanged`] / 1.0.0).
///
/// Dest mtime is taken from `meta.modified()` as whole seconds since
/// [`UNIX_EPOCH`] (same semantics as the CLI `file_mtime_secs` helper).
///
/// **Warning:** trusting mtime can miss content changes when mtime is forged,
/// drifted across clocks, or preserved incorrectly (e.g. `cp -p`, some network
/// FS). Default callers should pass `trust_mtime = false`. Symmetrical to
/// [`crate::decide_seed_trust_mtime`].
pub fn judge_extract_unchanged_opts(
    dest: &Path,
    size: u64,
    blob_blake3: &ChunkId,
    listing_mtime_secs: u64,
    trust_mtime: bool,
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

    if trust_mtime && dest_mtime_secs(&meta) == listing_mtime_secs {
        return Ok(UnchangedVerdict::Unchanged);
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

/// Whole seconds since UNIX epoch from `meta.modified()` (≡ CLI `file_mtime_secs`).
fn dest_mtime_secs(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::FileTimes;
    use std::io::Write;
    use std::time::{Duration, SystemTime};

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

    fn set_mtime_secs(path: &Path, secs: u64) {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
        let times = FileTimes::new().set_modified(t);
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(times)
            .unwrap();
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

    // --- Phase 11 M1: --skip-trust-mtime ---

    #[test]
    fn trust_mtime_size_mtime_hit_unchanged_without_content_hash() {
        // Deliberately wrong blob_blake3: if content were hashed we would get
        // ContentMismatch. Matching size+mtime with trust_mtime → Unchanged.
        let dir = TmpDir::new();
        let data = b"aaaaaaaa";
        let p = write_file(dir.path(), "risk.txt", data);
        let mtime = 1_700_000_042u64;
        set_mtime_secs(&p, mtime);
        let wrong_blob = ChunkId::hash(b"not-the-real-content!!!!");
        assert_eq!(
            judge_extract_unchanged_opts(&p, data.len() as u64, &wrong_blob, mtime, true).unwrap(),
            UnchangedVerdict::Unchanged
        );
    }

    #[test]
    fn trust_mtime_mtime_differ_falls_to_blake3() {
        let dir = TmpDir::new();
        let data = b"same-size-ok";
        let p = write_file(dir.path(), "z.txt", data);
        set_mtime_secs(&p, 200);
        let blob = ChunkId::hash(data);
        // listing mtime differs → must hash; matching content → Unchanged
        assert_eq!(
            judge_extract_unchanged_opts(&p, data.len() as u64, &blob, 100, true).unwrap(),
            UnchangedVerdict::Unchanged
        );
        // wrong content + mtime differ → ContentMismatch
        let wrong = ChunkId::hash(b"xxxxxxxxxxxx");
        assert_eq!(
            judge_extract_unchanged_opts(&p, data.len() as u64, &wrong, 100, true).unwrap(),
            UnchangedVerdict::ContentMismatch
        );
    }

    #[test]
    fn trust_mtime_size_mismatch_no_content_read() {
        // Size mismatch returns before open/hash; wrong blob is irrelevant.
        let dir = TmpDir::new();
        let p = write_file(dir.path(), "a.txt", b"hello");
        set_mtime_secs(&p, 42);
        let wrong = ChunkId::hash(b"other");
        assert_eq!(
            judge_extract_unchanged_opts(&p, 999, &wrong, 42, true).unwrap(),
            UnchangedVerdict::SizeMismatch
        );
    }

    #[test]
    fn default_judge_forwards_to_opts_no_trust() {
        // Same size + matching mtime but wrong content: default path must hash
        // and report ContentMismatch (trust off).
        let dir = TmpDir::new();
        let p = write_file(dir.path(), "a.txt", b"AAAA");
        set_mtime_secs(&p, 7);
        let other = ChunkId::hash(b"BBBB");
        assert_eq!(
            judge_extract_unchanged(&p, 4, &other).unwrap(),
            UnchangedVerdict::ContentMismatch
        );
        // Explicit opts(…, 0, false) ≡ default.
        assert_eq!(
            judge_extract_unchanged_opts(&p, 4, &other, 0, false).unwrap(),
            UnchangedVerdict::ContentMismatch
        );
        // Even with matching listing mtime, trust_mtime=false still hashes.
        assert_eq!(
            judge_extract_unchanged_opts(&p, 4, &other, 7, false).unwrap(),
            UnchangedVerdict::ContentMismatch
        );
    }
}
