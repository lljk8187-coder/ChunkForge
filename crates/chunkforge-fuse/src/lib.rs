//! Read-only single-blob FUSE filesystem for ChunkForge.
//!
//! Phase 2 M4: given a `.cfidx` [`Index`] and a [`ChunkSource`], present **one**
//! virtual regular file under the mount root. Kernel mounts are forced
//! [`MountOption::RO`]; write-side FUSE ops return `EROFS` / `EACCES`.
//!
//! This crate is a **library** only — the CLI `mount` subcommand is M5.
//!
//! # Layout
//!
//! - Root directory inode = [`ROOT_INO`] (1)
//! - Blob file inode = [`FILE_INO`] (2), size = `index.total_size`
//! - Default file name = index path stem (strip `.cfidx`); overridable
//!
//! # Testing without `/dev/fuse`
//!
//! [`BlobFs::read_at`] / [`read_range`] exercise the offset→chunk→splice path
//! with an in-memory [`ChunkSource`]. Real mount tests are `#[ignore]`.

mod fs;
mod mount;
mod read;

pub use fs::{BlobFs, FILE_INO, ROOT_INO};
pub use mount::{mount_options, mount_ro};
pub use read::read_range;

pub use chunkforge_index::Index;
pub use chunkforge_store::{ChunkSource, SourceError};
pub use fuser::{Filesystem, MountOption};

use std::path::Path;

/// Default mount file name from an index path: strip a trailing `.cfidx` suffix.
///
/// Examples: `hello.cfidx` → `hello`; `blob` → `blob`; `.cfidx` → `blob`.
pub fn default_blob_name(index_path: &Path) -> String {
    let file_name = index_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("blob");
    match file_name.strip_suffix(".cfidx") {
        Some(stem) if !stem.is_empty() => stem.to_string(),
        Some(_) => "blob".to_string(), // bare ".cfidx"
        None if file_name.is_empty() => "blob".to_string(),
        None => file_name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chunkforge_chunk::{ChunkId, ChunkParams};
    use chunkforge_index::IndexEntry;
    use chunkforge_store::SourceError;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// In-memory [`ChunkSource`] for unit tests (no disk, no FUSE).
    #[derive(Clone, Default)]
    struct MemSource {
        chunks: HashMap<ChunkId, Vec<u8>>,
    }

    impl ChunkSource for MemSource {
        fn has(&self, id: &ChunkId) -> Result<bool, SourceError> {
            Ok(self.chunks.contains_key(id))
        }

        fn get(&self, id: &ChunkId) -> Result<Vec<u8>, SourceError> {
            self.chunks
                .get(id)
                .cloned()
                .ok_or(SourceError::NotFound(*id))
        }
    }

    /// Build an index + mem source by splitting `data` at fixed cut points (exclusive ends).
    /// Avoids FastCDC minimum-size constraints so small fixtures can be multi-chunk.
    fn index_from_cuts(data: &[u8], cuts: &[usize]) -> (Index, MemSource) {
        assert!(!cuts.is_empty());
        assert_eq!(*cuts.last().unwrap(), data.len());
        let mut src = MemSource::default();
        let mut entries = Vec::new();
        let mut prev = 0usize;
        for &end in cuts {
            assert!(end > prev);
            let slice = &data[prev..end];
            let id = ChunkId::hash(slice);
            src.chunks.insert(id, slice.to_vec());
            entries.push(IndexEntry {
                end_offset: end as u64,
                chunk_id: id,
            });
            prev = end;
        }
        let index = Index::new(
            0,
            ChunkParams::default(),
            data.len() as u64,
            ChunkId::hash(data),
            entries,
        )
        .unwrap();
        (index, src)
    }

    fn hello_bytes() -> Vec<u8> {
        // fixtures/hello.txt — "hello chunkforge\n"
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/hello.txt");
        std::fs::read(&path).unwrap_or_else(|_| b"hello chunkforge\n".to_vec())
    }

    #[test]
    fn default_blob_name_strips_cfidx() {
        assert_eq!(default_blob_name(Path::new("hello.cfidx")), "hello");
        assert_eq!(
            default_blob_name(Path::new("/tmp/store/photo.bin.cfidx")),
            "photo.bin"
        );
        assert_eq!(default_blob_name(Path::new("no-suffix")), "no-suffix");
        assert_eq!(default_blob_name(Path::new(".cfidx")), "blob");
    }

    #[test]
    fn read_at_matches_hello_fixture() {
        let data = hello_bytes();
        // Single chunk is fine for hello; also cover multi-chunk splice below.
        let (index, src) = index_from_cuts(&data, &[data.len()]);
        assert_eq!(index.total_size, data.len() as u64);

        let fs = BlobFs::new(index, src, "hello");
        assert_eq!(fs.name(), "hello");
        assert_eq!(fs.total_size(), data.len() as u64);

        let all = fs.read_at(0, data.len() as u32).unwrap();
        assert_eq!(all, data);

        let mid = fs.read_at(6, 5).unwrap();
        assert_eq!(mid, &data[6..11]);

        assert!(fs.read_at(data.len() as u64, 16).unwrap().is_empty());

        let tail = fs.read_at((data.len() - 3) as u64, 100).unwrap();
        assert_eq!(tail, &data[data.len() - 3..]);
    }

    #[test]
    fn read_range_multi_chunk_hello_and_binary() {
        let hello = hello_bytes();
        // Force several small chunks across the 17-byte hello fixture.
        let cuts = vec![3usize, 7, 12, hello.len()];
        let (index, src) = index_from_cuts(&hello, &cuts);
        assert_eq!(index.entries.len(), 4);

        for off in [0u64, 1, 2, 3, 6, 7, 11, 12, 16] {
            for len in [1u32, 2, 5, 8, 17, 64] {
                let got = read_range(&index, &src, off, len).unwrap();
                let end = ((off as usize) + len as usize).min(hello.len());
                let start = (off as usize).min(hello.len());
                assert_eq!(&got[..], &hello[start..end], "hello off={off} len={len}");
            }
        }

        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/binary-256.bin");
        let data = std::fs::read(&path).expect("binary-256.bin fixture");
        let cuts = vec![16usize, 48, 100, 180, 256];
        let (index, src) = index_from_cuts(&data, &cuts);
        assert!(index.entries.len() > 1);

        for off in [0u64, 1, 15, 16, 47, 48, 99, 100, 179, 180, 255] {
            for len in [1u32, 7, 16, 33, 64, 256] {
                let got = read_range(&index, &src, off, len).unwrap();
                let end = ((off as usize) + len as usize).min(data.len());
                let start = (off as usize).min(data.len());
                assert_eq!(&got[..], &data[start..end], "bin off={off} len={len}");
            }
        }
    }

    #[test]
    fn read_range_empty_blob() {
        let index = Index::empty(ChunkParams::default());
        let src = MemSource::default();
        assert!(read_range(&index, &src, 0, 10).unwrap().is_empty());
    }

    #[test]
    fn read_range_missing_chunk_errors() {
        let id = ChunkId::hash(b"missing-chunk-payload");
        let index = Index::new(
            0,
            ChunkParams::default(),
            21,
            ChunkId::hash(b"x"),
            vec![IndexEntry {
                end_offset: 21,
                chunk_id: id,
            }],
        )
        .unwrap();
        let src = MemSource::default();
        let err = read_range(&index, &src, 0, 21).unwrap_err();
        assert!(matches!(err, SourceError::NotFound(c) if c == id));
    }

    #[test]
    fn blob_fs_with_arc_source() {
        let data = b"arc-source-blob-bytes!!"; // 23 bytes
        let (index, src) = index_from_cuts(data, &[8, 16, data.len()]);
        let fs = BlobFs::new(index, Arc::new(src), "arc-source-blob");
        assert_eq!(fs.read_at(0, 64).unwrap(), data);
        assert_eq!(fs.read_at(8, 8).unwrap(), &data[8..16]);
    }

    #[test]
    fn mount_options_hardcodes_ro() {
        let opts = mount_options([MountOption::FSName("chunkforge".into()), MountOption::RW]);
        assert!(opts.contains(&MountOption::RO));
        assert!(!opts.contains(&MountOption::RW));
    }

    /// Real FUSE mount smoke test — needs fuse3 + /dev/fuse; skipped by default.
    #[test]
    #[ignore = "requires fuse3 + /dev/fuse; run with --ignored when available"]
    fn real_mount_hello_cmp() {
        use std::fs;
        use std::process::Command;
        use std::thread;
        use std::time::Duration;
        use tempfile::tempdir;

        let data = hello_bytes();
        let (index, src) = index_from_cuts(&data, &[data.len()]);
        let fs = BlobFs::new(index, src, "hello");

        let dir = tempdir().unwrap();
        let mnt = dir.path().join("mnt");
        fs::create_dir(&mnt).unwrap();

        let mnt2 = mnt.clone();
        let handle = thread::spawn(move || {
            let _ = mount_ro(
                fs,
                &mnt2,
                [
                    MountOption::FSName("chunkforge-test".into()),
                    MountOption::AutoUnmount,
                ],
            );
        });

        let file = mnt.join("hello");
        for _ in 0..50 {
            if file.is_file() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        assert!(file.is_file(), "mount did not appear");
        let got = fs::read(&file).unwrap();
        assert_eq!(got, data);

        let write_err = fs::write(&file, b"x");
        assert!(write_err.is_err(), "write should fail on RO mount");

        let _ = Command::new("fusermount3").args(["-u"]).arg(&mnt).status();
        let _ = Command::new("fusermount").args(["-u"]).arg(&mnt).status();
        let _ = handle.join();
    }
}
