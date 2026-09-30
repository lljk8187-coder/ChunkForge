//! Read-only FUSE filesystems for ChunkForge.
//!
//! - [`BlobFs`]: single-blob mount from a `.cfidx` [`Index`] (Phase 2)
//! - [`DirFs`]: directory-tree mount from a `.cfdir` [`DirArchive`] (Phase 5 M4)
//! - Phase 21 M1: [`chunkforge_index::filter_dir_archive`] subsets a listing
//!   (matching Files + ancestor Dirs) before [`DirFs::new`]; empty filter ≡ 1.10 full tree.
//!
//! Kernel mounts are forced [`MountOption::RO`]; write-side FUSE ops return
//! `EROFS` / `EACCES`. Sequential forward reads prefetch subsequent chunk(s) into a
//! process-local [`PrefetchCache`] (default on, depth 1; distinct from Store `--cache`).
//!
//! # Layout (BlobFs)
//!
//! - Root directory inode = [`ROOT_INO`] (1)
//! - Blob file inode = [`FILE_INO`] (2), size = `index.total_size`
//! - Default file name = index path stem (strip `.cfidx`); overridable
//!
//! # Testing without `/dev/fuse`
//!
//! [`BlobFs::read_at`] / [`read_range`] / [`DirFs::read_at_path`] /
//! [`DirFs::lookup_path`] exercise the offset→chunk→splice path with an
//! in-memory [`ChunkSource`]. Real mount tests are `#[ignore]`.

mod dir_fs;
mod fs;
mod mount;
mod prefetch;
mod read;

pub use dir_fs::{DirFs, DirFsError};
pub use fs::{BlobFs, FILE_INO, ROOT_INO};
pub use mount::{mount_options, mount_ro};
pub use prefetch::{
    DEFAULT_MAX_PREFETCH_BYTES, DEFAULT_MAX_PREFETCH_CHUNKS, MAX_PREFETCH_CHUNKS_HARD_CAP,
    PrefetchCache,
};
pub use read::{read_entries, read_entries_cached, read_range, read_range_cached};

pub use chunkforge_index::{DirArchive, Index};
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
    use chunkforge_index::{DirEntry, DirEntryKind, IndexEntry};
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

    /// Split `data` into chunks at `cuts`, store in `src`, return chunk table.
    fn chunks_from_cuts(data: &[u8], cuts: &[usize], src: &mut MemSource) -> Vec<IndexEntry> {
        assert!(!cuts.is_empty());
        assert_eq!(*cuts.last().unwrap(), data.len());
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
        entries
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

    // --- Phase 5 M4: DirFs ---

    fn sample_tree() -> (DirArchive, MemSource, Vec<u8>, Vec<u8>) {
        let a = b"hello-tree-root\n".to_vec();
        let b = hello_bytes();
        let mut src = MemSource::default();
        let a_chunks = chunks_from_cuts(&a, &[a.len()], &mut src);
        // Multi-chunk nested file.
        let cuts = vec![3usize, 7, 12, b.len()];
        let b_chunks = chunks_from_cuts(&b, &cuts, &mut src);

        let arch = DirArchive::new(
            0,
            vec![
                DirEntry {
                    path: "a.txt".into(),
                    kind: DirEntryKind::File {
                        mode: 0o644,
                        size: a.len() as u64,
                        mtime_secs: 1_700_000_000,
                        blob_blake3: ChunkId::hash(&a),
                        chunks: a_chunks,
                    },
                },
                DirEntry {
                    path: "sub/b.txt".into(),
                    kind: DirEntryKind::File {
                        mode: 0o600,
                        size: b.len() as u64,
                        mtime_secs: 1_700_000_001,
                        blob_blake3: ChunkId::hash(&b),
                        chunks: b_chunks,
                    },
                },
                DirEntry {
                    path: "sub/empty-dir".into(),
                    kind: DirEntryKind::Dir { mode: 0o755 },
                },
                DirEntry {
                    path: "sub/nested/c.txt".into(),
                    kind: DirEntryKind::File {
                        mode: 0o644,
                        size: 0,
                        mtime_secs: 0,
                        blob_blake3: ChunkId::hash(b""),
                        chunks: vec![],
                    },
                },
            ],
        )
        .unwrap();
        (arch, src, a, b)
    }

    #[test]
    fn dir_fs_lookup_nested_paths() {
        let (arch, src, _, _) = sample_tree();
        let fs = DirFs::new(arch, src);

        assert!(fs.lookup_path("").is_some());
        assert!(fs.lookup_path(".").is_some());
        assert!(fs.is_dir_path(""));
        assert!(fs.is_file_path("a.txt"));
        assert!(fs.is_dir_path("sub"));
        assert!(fs.is_file_path("sub/b.txt"));
        assert!(fs.is_dir_path("sub/empty-dir"));
        assert!(fs.is_dir_path("sub/nested"));
        assert!(fs.is_file_path("sub/nested/c.txt"));

        assert!(fs.lookup_path("missing").is_none());
        assert!(fs.lookup_path("sub/missing").is_none());
        assert!(fs.lookup_path("a.txt/nope").is_none());
        assert!(fs.lookup_path("..").is_none());
        assert!(fs.lookup_path("sub/../a.txt").is_none());
    }

    #[test]
    fn dir_fs_readdir_root_and_sub() {
        let (arch, src, _, _) = sample_tree();
        let fs = DirFs::new(arch, src);

        let root = fs.readdir_path("").unwrap();
        let names: Vec<_> = root.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["a.txt", "sub"]);
        assert!(!root[0].1); // a.txt is file
        assert!(root[1].1); // sub is dir

        let sub = fs.readdir_path("sub").unwrap();
        let names: Vec<_> = sub.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["b.txt", "empty-dir", "nested"]);

        let err = fs.readdir_path("nope").unwrap_err();
        assert!(matches!(err, DirFsError::NotFound));
        let err = fs.readdir_path("a.txt").unwrap_err();
        assert!(matches!(err, DirFsError::NotADirectory));
    }

    #[test]
    fn dir_fs_read_at_path_nested_and_enoent() {
        let (arch, src, a, b) = sample_tree();
        let fs = DirFs::new(arch, src);

        assert_eq!(fs.read_at_path("a.txt", 0, 64).unwrap(), a);
        assert_eq!(fs.read_at_path("sub/b.txt", 0, 64).unwrap(), b);
        assert_eq!(fs.read_at_path("sub/b.txt", 6, 5).unwrap(), &b[6..11]);
        assert!(
            fs.read_at_path("sub/nested/c.txt", 0, 10)
                .unwrap()
                .is_empty()
        );

        // Multi-chunk splice across cuts on b.txt
        for off in [0u64, 1, 3, 7, 12] {
            for len in [1u32, 4, 17, 64] {
                let got = fs.read_at_path("sub/b.txt", off, len).unwrap();
                let end = ((off as usize) + len as usize).min(b.len());
                let start = (off as usize).min(b.len());
                assert_eq!(&got[..], &b[start..end], "off={off} len={len}");
            }
        }

        let err = fs.read_at_path("missing", 0, 10).unwrap_err();
        assert!(matches!(err, DirFsError::NotFound));
        let err = fs.read_at_path("sub/nope", 0, 10).unwrap_err();
        assert!(matches!(err, DirFsError::NotFound));
        let err = fs.read_at_path("sub", 0, 10).unwrap_err();
        assert!(matches!(err, DirFsError::IsDirectory));
    }

    #[test]
    fn dir_fs_synthesizes_parent_dirs_from_file_prefixes() {
        // Archive lists only files — no explicit Dir entries.
        let data = b"deep-file\n";
        let mut src = MemSource::default();
        let chunks = chunks_from_cuts(data, &[data.len()], &mut src);
        let arch = DirArchive::new(
            0,
            vec![DirEntry {
                path: "x/y/z.txt".into(),
                kind: DirEntryKind::File {
                    mode: 0o644,
                    size: data.len() as u64,
                    mtime_secs: 0,
                    blob_blake3: ChunkId::hash(data),
                    chunks,
                },
            }],
        )
        .unwrap();
        let fs = DirFs::new(arch, src);
        assert!(fs.is_dir_path("x"));
        assert!(fs.is_dir_path("x/y"));
        assert!(fs.is_file_path("x/y/z.txt"));
        assert_eq!(fs.read_at_path("x/y/z.txt", 0, 32).unwrap(), data);
        let kids = fs.readdir_path("x/y").unwrap();
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].0, "z.txt");
    }

    // --- Phase 10 M1: sequential PrefetchCache ---

    /// Counting [`ChunkSource`] that records every `get` id (order preserved).
    #[derive(Clone, Default)]
    struct CountingSource {
        inner: MemSource,
        gets: Arc<std::sync::Mutex<Vec<ChunkId>>>,
    }

    impl CountingSource {
        fn new(inner: MemSource) -> Self {
            Self {
                inner,
                gets: Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }

        fn clear_gets(&self) {
            self.gets.lock().unwrap().clear();
        }

        fn get_count(&self, id: &ChunkId) -> usize {
            self.gets
                .lock()
                .unwrap()
                .iter()
                .filter(|c| *c == id)
                .count()
        }
    }

    impl ChunkSource for CountingSource {
        fn has(&self, id: &ChunkId) -> Result<bool, SourceError> {
            self.inner.has(id)
        }

        fn get(&self, id: &ChunkId) -> Result<Vec<u8>, SourceError> {
            self.gets.lock().unwrap().push(*id);
            self.inner.get(id)
        }
    }

    #[test]
    fn sequential_read_prefetch_skips_reget_of_next_chunk() {
        // Three 100-byte chunks → sequential reads must not re-get a prefetched id.
        let data: Vec<u8> = (0..300).map(|i| (i % 256) as u8).collect();
        let cuts = [100usize, 200, 300];
        let (index, src) = index_from_cuts(&data, &cuts);
        assert_eq!(index.entries.len(), 3);
        let id0 = index.entries[0].chunk_id;
        let id1 = index.entries[1].chunk_id;
        let id2 = index.entries[2].chunk_id;

        let counting = CountingSource::new(src);
        let fs = BlobFs::new(index, counting.clone(), "seq");

        // Read 1: partial first chunk → get(id0) + prefetch(id1).
        let r1 = fs.read_at(0, 50).unwrap();
        assert_eq!(r1, &data[0..50]);
        assert_eq!(counting.get_count(&id0), 1);
        assert_eq!(
            counting.get_count(&id1),
            1,
            "id1 must be prefetched after read 1"
        );
        assert_eq!(counting.get_count(&id2), 0);

        counting.clear_gets();

        // Read 2: continue sequentially across chunk boundary into id1.
        // Needs remainder of id0 (re-get OK) + start of id1 (must hit prefetch).
        let r2 = fs.read_at(50, 100).unwrap();
        assert_eq!(r2, &data[50..150]);
        assert_eq!(
            counting.get_count(&id0),
            1,
            "still need on-demand get for current chunk"
        );
        assert_eq!(
            counting.get_count(&id1),
            0,
            "prefetched id1 must not be re-gotten on sequential read 2"
        );
        assert_eq!(
            counting.get_count(&id2),
            1,
            "id2 prefetched after consuming into id1"
        );
        assert_eq!(&r2[..], &data[50..150]);
    }

    #[test]
    fn prefetch_chunks_two_prefetches_deeper() {
        // Four 50-byte chunks; depth 2 → after reading into id0, id1 AND id2 prefetched.
        let data: Vec<u8> = (0..200).map(|i| (i % 256) as u8).collect();
        let cuts = [50usize, 100, 150, 200];
        let (index, src) = index_from_cuts(&data, &cuts);
        assert_eq!(index.entries.len(), 4);
        let id0 = index.entries[0].chunk_id;
        let id1 = index.entries[1].chunk_id;
        let id2 = index.entries[2].chunk_id;
        let id3 = index.entries[3].chunk_id;

        let counting = CountingSource::new(src);
        let fs = BlobFs::new(index, counting.clone(), "depth2").with_prefetch_chunks(2);

        let r1 = fs.read_at(0, 25).unwrap();
        assert_eq!(r1, &data[0..25]);
        assert_eq!(counting.get_count(&id0), 1);
        assert_eq!(counting.get_count(&id1), 1, "id1 prefetched at depth 2");
        assert_eq!(counting.get_count(&id2), 1, "id2 prefetched at depth 2");
        assert_eq!(counting.get_count(&id3), 0, "id3 beyond depth 2");

        counting.clear_gets();
        // Sequential continue into id1: hit prefetch for id1; may re-get id0; prefetch id3.
        let r2 = fs.read_at(25, 50).unwrap();
        assert_eq!(r2, &data[25..75]);
        assert_eq!(counting.get_count(&id1), 0, "id1 must hit prefetch");
        assert_eq!(counting.get_count(&id2), 0, "id2 must still be cached");
    }

    #[test]
    fn prefetch_chunks_one_equiv_default() {
        let data: Vec<u8> = (0..300).map(|i| (i % 256) as u8).collect();
        let cuts = [100usize, 200, 300];
        let (index, src) = index_from_cuts(&data, &cuts);
        let id1 = index.entries[1].chunk_id;
        let id2 = index.entries[2].chunk_id;

        let counting = CountingSource::new(src);
        let fs = BlobFs::new(index, counting.clone(), "depth1").with_prefetch_chunks(1);
        let _ = fs.read_at(0, 50).unwrap();
        assert_eq!(counting.get_count(&id1), 1);
        assert_eq!(
            counting.get_count(&id2),
            0,
            "N=1 must not prefetch beyond next chunk (≡ 1.0.0)"
        );
    }

    #[test]
    fn prefetch_chunks_clamps_above_two() {
        let data = b"abcdefghij".to_vec();
        let cuts = [data.len()];
        let (index, src) = index_from_cuts(&data, &cuts);
        // Construction must not panic; depth clamped to 2.
        let fs = BlobFs::new(index, src, "clamp").with_prefetch_chunks(99);
        let got = fs.read_at(0, 4).unwrap();
        assert_eq!(got, b"abcd");
    }

    #[test]
    fn prefetch_disabled_get_count_equals_on_demand_path() {
        // With `--no-prefetch` / with_prefetch(false), every chunk is fetched on
        // demand — get count must match the bare `read_range` (no cache) path.
        let data: Vec<u8> = (0..300).map(|i| (i % 256) as u8).collect();
        let cuts = [100usize, 200, 300];
        let (index, src) = index_from_cuts(&data, &cuts);
        let id0 = index.entries[0].chunk_id;
        let id1 = index.entries[1].chunk_id;
        let id2 = index.entries[2].chunk_id;

        // Baseline: on-demand read_range (no PrefetchCache).
        let baseline = CountingSource::new(src.clone());
        let _ = read_range(&index, &baseline, 0, 50).unwrap();
        let _ = read_range(&index, &baseline, 50, 100).unwrap();
        let _ = read_range(&index, &baseline, 150, 100).unwrap();
        let base0 = baseline.get_count(&id0);
        let base1 = baseline.get_count(&id1);
        let base2 = baseline.get_count(&id2);
        assert!(base0 >= 1 && base1 >= 1 && base2 >= 1);

        // Prefetch off via with_prefetch(false) ≡ on-demand.
        let counting = CountingSource::new(src);
        let fs = BlobFs::new(index, counting.clone(), "nopre").with_prefetch(false);
        let _ = fs.read_at(0, 50).unwrap();
        let _ = fs.read_at(50, 100).unwrap();
        let _ = fs.read_at(150, 100).unwrap();
        assert_eq!(
            counting.get_count(&id0),
            base0,
            "prefetch-off id0 gets must ≡ on-demand read_range"
        );
        assert_eq!(
            counting.get_count(&id1),
            base1,
            "prefetch-off id1 gets must ≡ on-demand read_range"
        );
        assert_eq!(
            counting.get_count(&id2),
            base2,
            "prefetch-off id2 gets must ≡ on-demand read_range"
        );
        // And must not have fewer gets than baseline (no silent prefetch hits).
        let total_off =
            counting.get_count(&id0) + counting.get_count(&id1) + counting.get_count(&id2);
        let total_base = base0 + base1 + base2;
        assert_eq!(total_off, total_base);
    }

    #[test]
    fn non_contiguous_forward_seek_cold_starts_window() {
        // Forward jump that skips the expected next_offset → invalidate.
        let data: Vec<u8> = (0..300).map(|i| (i % 256) as u8).collect();
        let cuts = [100usize, 200, 300];
        let (index, src) = index_from_cuts(&data, &cuts);
        let id1 = index.entries[1].chunk_id;
        let id2 = index.entries[2].chunk_id;

        let counting = CountingSource::new(src);
        let fs = BlobFs::new(index, counting.clone(), "gap");

        // Establish window ending at 50; prefetch id1.
        let _ = fs.read_at(0, 50).unwrap();
        assert_eq!(counting.get_count(&id1), 1);
        counting.clear_gets();

        // Non-contiguous forward seek (skip past expected offset 50 → jump to 200).
        let got = fs.read_at(200, 50).unwrap();
        assert_eq!(got, &data[200..250]);
        assert_eq!(counting.get_count(&id2), 1);
        // Stale id1 prefetch must not be reused for this discontinuous read.
        assert_eq!(
            counting.get_count(&id1),
            0,
            "non-contiguous seek must cold-start; must not touch stale prefetch"
        );
    }

    #[test]
    fn seek_backward_cold_starts_prefetch_window() {
        let data: Vec<u8> = (0..300).map(|i| (i % 256) as u8).collect();
        let cuts = [100usize, 200, 300];
        let (index, src) = index_from_cuts(&data, &cuts);
        let id1 = index.entries[1].chunk_id;
        let id2 = index.entries[2].chunk_id;

        let counting = CountingSource::new(src);
        let fs = BlobFs::new(index, counting.clone(), "seek");

        // Establish window + prefetch id1.
        let _ = fs.read_at(0, 50).unwrap();
        assert_eq!(counting.get_count(&id1), 1);
        counting.clear_gets();

        // Backward seek → invalidate; must not reuse stale prefetch for a later jump.
        let _ = fs.read_at(0, 10).unwrap(); // cold from start again
        counting.clear_gets();
        // Jump into chunk 2 without sequential advance through chunk 1.
        let got = fs.read_at(200, 50).unwrap();
        assert_eq!(got, &data[200..250]);
        assert_eq!(counting.get_count(&id2), 1);
        // id1 must not have been served from a stale prefetch for this discontinuous read.
        // (We did not need id1 at all for offset 200.)
        assert_eq!(counting.get_count(&id1), 0);
    }

    #[test]
    fn prefetch_failure_does_not_fail_satisfied_read() {
        /// Source that fails get for a designated "poison" id after serving others.
        struct PoisonAfter {
            inner: MemSource,
            poison: ChunkId,
            gets: Arc<std::sync::Mutex<Vec<ChunkId>>>,
        }

        impl ChunkSource for PoisonAfter {
            fn has(&self, id: &ChunkId) -> Result<bool, SourceError> {
                self.inner.has(id)
            }

            fn get(&self, id: &ChunkId) -> Result<Vec<u8>, SourceError> {
                self.gets.lock().unwrap().push(*id);
                if *id == self.poison {
                    return Err(SourceError::Backend("poisoned prefetch".into()));
                }
                self.inner.get(id)
            }
        }

        let data: Vec<u8> = (0..200).map(|i| (i % 256) as u8).collect();
        let cuts = [100usize, 200];
        let (index, src) = index_from_cuts(&data, &cuts);
        let id1 = index.entries[1].chunk_id;
        let poison = PoisonAfter {
            inner: src,
            poison: id1,
            gets: Arc::new(std::sync::Mutex::new(Vec::new())),
        };
        let fs = BlobFs::new(index, poison, "poison");

        // Read only chunk 0; prefetch of id1 fails silently — current read must succeed.
        let r = fs.read_at(0, 50).unwrap();
        assert_eq!(r, &data[0..50]);
    }

    #[test]
    fn dir_fs_cross_file_cold_starts_prefetch() {
        let a: Vec<u8> = (0..200).map(|i| (i % 256) as u8).collect();
        let b: Vec<u8> = (10..210).map(|i| (i % 256) as u8).collect();
        let mut src = MemSource::default();
        let a_chunks = chunks_from_cuts(&a, &[100, 200], &mut src);
        let b_chunks = chunks_from_cuts(&b, &[100, 200], &mut src);
        let a_id1 = a_chunks[1].chunk_id;

        let arch = DirArchive::new(
            0,
            vec![
                DirEntry {
                    path: "a.bin".into(),
                    kind: DirEntryKind::File {
                        mode: 0o644,
                        size: a.len() as u64,
                        mtime_secs: 0,
                        blob_blake3: ChunkId::hash(&a),
                        chunks: a_chunks,
                    },
                },
                DirEntry {
                    path: "b.bin".into(),
                    kind: DirEntryKind::File {
                        mode: 0o644,
                        size: b.len() as u64,
                        mtime_secs: 0,
                        blob_blake3: ChunkId::hash(&b),
                        chunks: b_chunks,
                    },
                },
            ],
        )
        .unwrap();

        let counting = CountingSource::new(src);
        let fs = DirFs::new(arch, counting.clone());

        // Sequential read on a.bin prefetches a_id1.
        let _ = fs.read_at_path("a.bin", 0, 50).unwrap();
        assert_eq!(counting.get_count(&a_id1), 1);
        counting.clear_gets();

        // Cross-file → cold-start; reading b.bin must not consume a.bin's prefetch.
        let got = fs.read_at_path("b.bin", 0, 50).unwrap();
        assert_eq!(got, &b[0..50]);
        assert_eq!(
            counting.get_count(&a_id1),
            0,
            "cross-file must not reuse other file's prefetched chunk"
        );
    }


    // --- Phase 21 M1: filter_dir_archive → DirFs (no real FUSE) ---

    fn filter_sample_tree() -> (DirArchive, MemSource) {
        let a = b"foo-a\n".to_vec();
        let b = b"foo-b\n".to_vec();
        let c = b"bar-c\n".to_vec();
        let d = b"other\n".to_vec();
        let mut src = MemSource::default();
        let a_chunks = chunks_from_cuts(&a, &[a.len()], &mut src);
        let b_chunks = chunks_from_cuts(&b, &[b.len()], &mut src);
        let c_chunks = chunks_from_cuts(&c, &[c.len()], &mut src);
        let d_chunks = chunks_from_cuts(&d, &[d.len()], &mut src);
        let arch = DirArchive::new(
            0,
            vec![
                DirEntry {
                    path: "pkgs".into(),
                    kind: DirEntryKind::Dir { mode: 0o755 },
                },
                DirEntry {
                    path: "pkgs/foo".into(),
                    kind: DirEntryKind::Dir { mode: 0o755 },
                },
                DirEntry {
                    path: "pkgs/foo/a.txt".into(),
                    kind: DirEntryKind::File {
                        mode: 0o644,
                        size: a.len() as u64,
                        mtime_secs: 1,
                        blob_blake3: ChunkId::hash(&a),
                        chunks: a_chunks,
                    },
                },
                DirEntry {
                    path: "pkgs/foo/b.txt".into(),
                    kind: DirEntryKind::File {
                        mode: 0o644,
                        size: b.len() as u64,
                        mtime_secs: 2,
                        blob_blake3: ChunkId::hash(&b),
                        chunks: b_chunks,
                    },
                },
                DirEntry {
                    path: "pkgs/bar".into(),
                    kind: DirEntryKind::Dir { mode: 0o700 },
                },
                DirEntry {
                    path: "pkgs/bar/c.txt".into(),
                    kind: DirEntryKind::File {
                        mode: 0o600,
                        size: c.len() as u64,
                        mtime_secs: 3,
                        blob_blake3: ChunkId::hash(&c),
                        chunks: c_chunks,
                    },
                },
                DirEntry {
                    path: "other/x.txt".into(),
                    kind: DirEntryKind::File {
                        mode: 0o644,
                        size: d.len() as u64,
                        mtime_secs: 4,
                        blob_blake3: ChunkId::hash(&d),
                        chunks: d_chunks,
                    },
                },
            ],
        )
        .unwrap();
        (arch, src)
    }

    #[test]
    fn filter_include_prefix_lookup_readdir_subset() {
        use chunkforge_index::{PathFilter, filter_dir_archive};

        let (arch, src) = filter_sample_tree();
        let filter = PathFilter::new(["pkgs/foo"], Vec::<String>::new()).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        let fs = DirFs::new(filtered, src);

        // Matching files + ancestors visible.
        assert!(fs.is_dir_path("pkgs"));
        assert!(fs.is_dir_path("pkgs/foo"));
        assert!(fs.is_file_path("pkgs/foo/a.txt"));
        assert!(fs.is_file_path("pkgs/foo/b.txt"));
        assert_eq!(fs.read_at_path("pkgs/foo/a.txt", 0, 64).unwrap(), b"foo-a\n");

        // Non-matching paths absent from lookup / readdir.
        assert!(fs.lookup_path("pkgs/bar").is_none());
        assert!(fs.lookup_path("pkgs/bar/c.txt").is_none());
        assert!(fs.lookup_path("other").is_none());
        assert!(fs.lookup_path("other/x.txt").is_none());

        let root = fs.readdir_path("").unwrap();
        let names: Vec<_> = root.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["pkgs"]);

        let pkgs = fs.readdir_path("pkgs").unwrap();
        let names: Vec<_> = pkgs.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["foo"]);

        let foo = fs.readdir_path("pkgs/foo").unwrap();
        let names: Vec<_> = foo.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["a.txt", "b.txt"]);
    }

    #[test]
    fn filter_exclude_hides_paths() {
        use chunkforge_index::{PathFilter, filter_dir_archive};

        let (arch, src) = filter_sample_tree();
        let filter = PathFilter::new(["pkgs"], ["pkgs/bar/"]).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        let fs = DirFs::new(filtered, src);

        assert!(fs.is_file_path("pkgs/foo/a.txt"));
        assert!(fs.is_file_path("pkgs/foo/b.txt"));
        assert!(fs.lookup_path("pkgs/bar").is_none());
        assert!(fs.lookup_path("pkgs/bar/c.txt").is_none());
        // other/ not under pkgs include → gone
        assert!(fs.lookup_path("other/x.txt").is_none());

        let pkgs = fs.readdir_path("pkgs").unwrap();
        let names: Vec<_> = pkgs.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["foo"]);
    }

    #[test]
    fn filter_empty_equiv_full_archive_dirfs() {
        use chunkforge_index::{PathFilter, filter_dir_archive};

        let (arch, src) = filter_sample_tree();
        let empty = PathFilter::new(Vec::<String>::new(), Vec::<String>::new()).unwrap();
        let filtered = filter_dir_archive(&arch, &empty);
        assert_eq!(filtered, arch);

        let full = DirFs::new(arch.clone(), src.clone());
        let via_filter = DirFs::new(filtered, src);

        // Same path visibility (full tree ≡ 1.10).
        for path in [
            "pkgs",
            "pkgs/foo",
            "pkgs/foo/a.txt",
            "pkgs/bar/c.txt",
            "other/x.txt",
        ] {
            assert_eq!(
                full.lookup_path(path).is_some(),
                via_filter.lookup_path(path).is_some(),
                "path {path}"
            );
            assert_eq!(full.is_dir_path(path), via_filter.is_dir_path(path));
            assert_eq!(full.is_file_path(path), via_filter.is_file_path(path));
        }

        let root_full: Vec<_> = full
            .readdir_path("")
            .unwrap()
            .into_iter()
            .map(|(n, d)| (n, d))
            .collect();
        let root_filt: Vec<_> = via_filter
            .readdir_path("")
            .unwrap()
            .into_iter()
            .map(|(n, d)| (n, d))
            .collect();
        assert_eq!(root_full, root_filt);

        assert_eq!(
            full.read_at_path("pkgs/foo/a.txt", 0, 64).unwrap(),
            via_filter.read_at_path("pkgs/foo/a.txt", 0, 64).unwrap()
        );
    }

    /// Real DirFs FUSE mount — needs fuse3 + /dev/fuse; skipped by default.
    #[test]
    #[ignore = "requires fuse3 + /dev/fuse; run with --ignored when available"]
    fn real_mount_dir_fs_cmp() {
        use std::fs;
        use std::process::Command;
        use std::thread;
        use std::time::Duration;
        use tempfile::tempdir;

        let (arch, src, a, b) = sample_tree();
        let fs = DirFs::new(arch, src);

        let dir = tempdir().unwrap();
        let mnt = dir.path().join("mnt");
        fs::create_dir(&mnt).unwrap();

        let mnt2 = mnt.clone();
        let handle = thread::spawn(move || {
            let _ = mount_ro(
                fs,
                &mnt2,
                [
                    MountOption::FSName("chunkforge-dir-test".into()),
                    MountOption::AutoUnmount,
                ],
            );
        });

        let a_path = mnt.join("a.txt");
        for _ in 0..50 {
            if a_path.is_file() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        assert!(a_path.is_file(), "mount did not appear");
        assert_eq!(fs::read(&a_path).unwrap(), a);
        assert_eq!(fs::read(mnt.join("sub/b.txt")).unwrap(), b);
        assert!(mnt.join("sub/empty-dir").is_dir());
        assert_eq!(fs::read(mnt.join("sub/nested/c.txt")).unwrap(), b"");

        let write_err = fs::write(&a_path, b"x");
        assert!(write_err.is_err(), "write should fail on RO mount");

        let _ = Command::new("fusermount3").args(["-u"]).arg(&mnt).status();
        let _ = Command::new("fusermount").args(["-u"]).arg(&mnt).status();
        let _ = handle.join();
    }
}
