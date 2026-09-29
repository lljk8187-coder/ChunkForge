//! Seed / prior-index helpers for `archive --seed` (Phase 6).
//!
//! Build a path → file-entry map from a prior [`.cfdir`](crate::DirArchive), then
//! decide [`Reuse`](SeedDecision::Reuse) vs [`Rechunk`](SeedDecision::Rechunk) by
//! **content BLAKE3** (with a size fast-reject). mtime is **never** the sole
//! reuse criterion on the default path; `--seed-trust-mtime` is out of scope here.
//!
//! Does **not** change `.cfdir` / `.cfidx` v1 byte layouts.

use crate::{DirArchive, DirEntry, DirEntryKind, Error};
use chunkforge_chunk::ChunkId;
use std::collections::HashMap;
use std::io::Read;

/// Whether a source file can reuse a prior `.cfdir` file entry's chunk table.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum SeedDecision {
    /// Copy prior `chunks` / `blob_blake3` / size / mode into the new archive;
    /// skip FastCDC for this file.
    Reuse,
    /// Path missing, size mismatch, or content blake3 differs → run FastCDC.
    Rechunk,
}

/// Build a path → file-entry map from a prior `.cfdir` (**File** kinds only).
///
/// Directory entries are skipped. Paths not present in the returned map are
/// treated as [`SeedDecision::Rechunk`] by the caller.
pub fn seed_file_map(prior: &DirArchive) -> HashMap<&str, &DirEntry> {
    prior
        .entries
        .iter()
        .filter(|e| matches!(e.kind, DirEntryKind::File { .. }))
        .map(|e| (e.path.as_str(), e))
        .collect()
}

/// Decide reuse vs rechunk given prior file size/blake3 and a source stream.
///
/// 1. **Size fast-reject**: if `source_size != prior_size` → [`Rechunk`](SeedDecision::Rechunk)
///    without reading `source`.
/// 2. Otherwise stream-hash `source` with BLAKE3; equal to `prior_blob_blake3` →
///    [`Reuse`](SeedDecision::Reuse), else [`Rechunk`](SeedDecision::Rechunk).
///
/// Does **not** consult mtime.
pub fn decide_seed(
    prior_size: u64,
    prior_blob_blake3: &ChunkId,
    source_size: u64,
    source: &mut impl Read,
) -> Result<SeedDecision, Error> {
    if source_size != prior_size {
        return Ok(SeedDecision::Rechunk);
    }
    let blob = hash_reader(source)?;
    if &blob == prior_blob_blake3 {
        Ok(SeedDecision::Reuse)
    } else {
        Ok(SeedDecision::Rechunk)
    }
}

/// Decide against a prior [`DirEntry`] (must be [`DirEntryKind::File`]).
///
/// Directory priors return [`Error::InvalidStructure`]. Missing-path handling
/// stays with the caller via [`seed_file_map`].
pub fn decide_seed_for_entry(
    prior: &DirEntry,
    source_size: u64,
    source: &mut impl Read,
) -> Result<SeedDecision, Error> {
    match &prior.kind {
        DirEntryKind::File {
            size, blob_blake3, ..
        } => decide_seed(*size, blob_blake3, source_size, source),
        DirEntryKind::Dir { .. } => Err(Error::InvalidStructure(
            "seed decision requires a File prior entry, got Dir".into(),
        )),
    }
}

/// Stream BLAKE3 over `source` (64 KiB buffer). Used by [`decide_seed`].
pub fn hash_reader(source: &mut impl Read) -> Result<ChunkId, Error> {
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = source.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(ChunkId::from_bytes(*hasher.finalize().as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IndexEntry;
    use std::io::{self, Cursor};

    /// Reader that panics if touched — proves size fast-reject skips hashing.
    struct PanicRead;
    impl Read for PanicRead {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            panic!("size fast-reject must not read source content");
        }
    }

    fn file_entry(
        path: &str,
        size: u64,
        mtime_secs: u64,
        blob: ChunkId,
        chunks: Vec<IndexEntry>,
    ) -> DirEntry {
        DirEntry {
            path: path.into(),
            kind: DirEntryKind::File {
                mode: 0o644,
                size,
                mtime_secs,
                blob_blake3: blob,
                chunks,
            },
        }
    }

    fn sample_prior() -> DirArchive {
        let content = b"hello-seed-v1";
        let blob = ChunkId::hash(content);
        let id = ChunkId::hash(content);
        DirArchive::new(
            0,
            vec![
                file_entry(
                    "a.txt",
                    content.len() as u64,
                    1_700_000_000,
                    blob,
                    vec![IndexEntry {
                        end_offset: content.len() as u64,
                        chunk_id: id,
                    }],
                ),
                DirEntry {
                    path: "sub".into(),
                    kind: DirEntryKind::Dir { mode: 0o755 },
                },
                file_entry("sub/b.txt", 0, 1_700_000_001, ChunkId::hash(b""), vec![]),
            ],
        )
        .unwrap()
    }

    #[test]
    fn seed_file_map_files_only_skips_dirs() {
        let prior = sample_prior();
        let map = seed_file_map(&prior);
        assert_eq!(map.len(), 2);
        assert!(map.contains_key("a.txt"));
        assert!(map.contains_key("sub/b.txt"));
        assert!(!map.contains_key("sub"));
        assert!(matches!(
            map.get("a.txt").unwrap().kind,
            DirEntryKind::File { .. }
        ));
    }

    #[test]
    fn same_content_reuses() {
        let content = b"hello-seed-v1";
        let blob = ChunkId::hash(content);
        let mut src = Cursor::new(content.as_slice());
        let d = decide_seed(content.len() as u64, &blob, content.len() as u64, &mut src).unwrap();
        assert_eq!(d, SeedDecision::Reuse);
    }

    #[test]
    fn one_byte_change_rechunks() {
        let prior_content = b"hello-seed-v1";
        let blob = ChunkId::hash(prior_content);
        let changed = b"hello-seed-v2"; // same length, one-byte (and more) change
        assert_eq!(prior_content.len(), changed.len());
        let mut src = Cursor::new(changed.as_slice());
        let d = decide_seed(
            prior_content.len() as u64,
            &blob,
            changed.len() as u64,
            &mut src,
        )
        .unwrap();
        assert_eq!(d, SeedDecision::Rechunk);
    }

    #[test]
    fn size_mismatch_rechunks_without_reading() {
        let blob = ChunkId::hash(b"abc");
        let mut src = PanicRead;
        let d = decide_seed(3, &blob, 4, &mut src).unwrap();
        assert_eq!(d, SeedDecision::Rechunk);
    }

    #[test]
    fn path_missing_from_prior_map_not_found() {
        let prior = sample_prior();
        let map = seed_file_map(&prior);
        assert!(!map.contains_key("renamed.txt"));
        assert!(!map.contains_key("no/such/path"));
        // Caller treats missing path as Rechunk (no decide_seed call).
    }

    #[test]
    fn path_present_then_decide_via_entry() {
        let prior = sample_prior();
        let map = seed_file_map(&prior);
        let entry = map.get("a.txt").expect("a.txt in prior");
        let content = b"hello-seed-v1";
        let mut src = Cursor::new(content.as_slice());
        let d = decide_seed_for_entry(entry, content.len() as u64, &mut src).unwrap();
        assert_eq!(d, SeedDecision::Reuse);

        let mut src = Cursor::new(b"hello-seed-v2".as_slice());
        let d = decide_seed_for_entry(entry, 13, &mut src).unwrap();
        assert_eq!(d, SeedDecision::Rechunk);
    }

    #[test]
    fn decide_seed_for_entry_rejects_dir() {
        let dir = DirEntry {
            path: "sub".into(),
            kind: DirEntryKind::Dir { mode: 0o755 },
        };
        let err = decide_seed_for_entry(&dir, 0, &mut Cursor::new(b"")).unwrap_err();
        assert!(matches!(err, Error::InvalidStructure(_)), "{err:?}");
    }

    #[test]
    fn different_mtime_same_content_still_reuse() {
        // Prior recorded mtime A; source content unchanged. Decision ignores mtime.
        let content = b"stable-bytes";
        let blob = ChunkId::hash(content);
        let prior = file_entry(
            "x.txt",
            content.len() as u64,
            100,
            blob,
            vec![IndexEntry {
                end_offset: content.len() as u64,
                chunk_id: ChunkId::hash(content),
            }],
        );
        // Even if a hypothetical source mtime were 9999, decide_seed never sees it.
        let mut src = Cursor::new(content.as_slice());
        let d = decide_seed_for_entry(&prior, content.len() as u64, &mut src).unwrap();
        assert_eq!(d, SeedDecision::Reuse);

        // Same content against a prior with a different stored mtime → still Reuse.
        let prior_other_mtime = file_entry(
            "x.txt",
            content.len() as u64,
            9_999_999_999,
            blob,
            vec![IndexEntry {
                end_offset: content.len() as u64,
                chunk_id: ChunkId::hash(content),
            }],
        );
        let mut src = Cursor::new(content.as_slice());
        let d = decide_seed_for_entry(&prior_other_mtime, content.len() as u64, &mut src).unwrap();
        assert_eq!(d, SeedDecision::Reuse);
    }

    #[test]
    fn same_mtime_different_content_rechunks() {
        // Same size + same recorded mtime, but content flipped → must Rechunk.
        let prior_bytes = b"aaaaaaaa";
        let changed = b"aaaaaaab";
        assert_eq!(prior_bytes.len(), changed.len());
        let blob = ChunkId::hash(prior_bytes);
        let prior = file_entry(
            "y.txt",
            prior_bytes.len() as u64,
            42,
            blob,
            vec![IndexEntry {
                end_offset: prior_bytes.len() as u64,
                chunk_id: ChunkId::hash(prior_bytes),
            }],
        );
        let mut src = Cursor::new(changed.as_slice());
        let d = decide_seed_for_entry(&prior, changed.len() as u64, &mut src).unwrap();
        assert_eq!(d, SeedDecision::Rechunk);
    }

    #[test]
    fn empty_file_reuse() {
        let blob = ChunkId::hash(b"");
        let mut src = Cursor::new(b"");
        let d = decide_seed(0, &blob, 0, &mut src).unwrap();
        assert_eq!(d, SeedDecision::Reuse);
    }

    #[test]
    fn hash_reader_matches_chunk_id_hash() {
        let data = b"stream-me-please";
        let mut src = Cursor::new(data.as_slice());
        assert_eq!(hash_reader(&mut src).unwrap(), ChunkId::hash(data));
    }
}
