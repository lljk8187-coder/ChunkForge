//! Materialize-side **chunk-level** seed plan (Phase 27 M1).
//!
//! Given a *prior* `.cfdir` listing that describes a tree already present on
//! local disk (the "seed root"), compute where each chunk's plaintext bytes live
//! inside that tree: `chunk_id ↦ [(path, offset, len), …]`.
//!
//! - **Pure / zero IO**: offsets are derived from [`IndexEntry::end_offset`]
//!   (`offset_i = end_offset_{i-1}`, `0` for the first chunk; `len_i = end_offset_i - offset_i`).
//! - Only [`DirEntryKind::File`] entries contribute; `Dir` / `Symlink` are skipped.
//! - Paths that fail [`validate_archive_path`] (absolute, `..`, empty segments, …)
//!   are skipped, as are files whose chunk table is structurally invalid
//!   (defensive: [`DirArchive`] fields are public and may be hand-built).
//! - Duplicate chunk ids keep **all** locations in listing order (callers try
//!   them in order; the first verified read wins).
//! - Optional `wanted` filter keeps only chunks the *target* listing needs, so
//!   memory is proportional to the needed set.
//!
//! This is distinct from the archive-side whole-file [`seed_file_map`](crate::seed_file_map)
//! (`archive --seed`). The IO + BLAKE3-verified reader lives in
//! `chunkforge-store::SeedSource`; this crate does **not** depend on the store.
//! Listing bytes (`.cfidx` / `.cfdir`) are not touched.

use crate::dir::validate_file_chunks;
use crate::path::validate_archive_path;
use crate::{DirArchive, DirEntryKind};
use chunkforge_chunk::ChunkId;
use std::collections::{HashMap, HashSet};

/// One place inside the seed root where a chunk's plaintext is expected to live.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct SeedLocation<'a> {
    /// Validated relative, `/`-separated archive path (relative to the seed root).
    pub path: &'a str,
    /// Byte offset of the chunk inside the file.
    pub offset: u64,
    /// Chunk plaintext length in bytes (> 0).
    pub len: u64,
}

/// Compute `chunk_id ↦ locations` from a prior listing. Pure; no IO.
///
/// See the [module docs](self) for the exact rules. An empty listing (or a
/// listing with no File chunks / no wanted chunks) yields an empty map.
pub fn chunk_locations<'a>(
    prior: &'a DirArchive,
    wanted: Option<&HashSet<ChunkId>>,
) -> HashMap<ChunkId, Vec<SeedLocation<'a>>> {
    let mut out: HashMap<ChunkId, Vec<SeedLocation<'a>>> = HashMap::new();
    for e in &prior.entries {
        let DirEntryKind::File { size, chunks, .. } = &e.kind else {
            continue;
        };
        if validate_archive_path(&e.path).is_err() || validate_file_chunks(*size, chunks).is_err() {
            continue;
        }
        let mut start = 0u64;
        for c in chunks {
            let end = c.end_offset;
            if wanted.is_none_or(|w| w.contains(&c.chunk_id)) {
                out.entry(c.chunk_id).or_default().push(SeedLocation {
                    path: &e.path,
                    offset: start,
                    len: end - start,
                });
            }
            start = end;
        }
    }
    out
}

/// Alias of [`chunk_locations`] (Phase 27 dispatch name: "seed plan").
pub fn seed_plan<'a>(
    prior: &'a DirArchive,
    wanted: Option<&HashSet<ChunkId>>,
) -> HashMap<ChunkId, Vec<SeedLocation<'a>>> {
    chunk_locations(prior, wanted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DirEntry, IndexEntry};

    fn id(s: &str) -> ChunkId {
        ChunkId::hash(s.as_bytes())
    }

    fn file(path: &str, chunks: &[(u64, ChunkId)]) -> DirEntry {
        let size = chunks.last().map(|c| c.0).unwrap_or(0);
        DirEntry {
            path: path.into(),
            kind: DirEntryKind::File {
                mode: 0o644,
                size,
                mtime_secs: 0,
                blob_blake3: ChunkId::hash(path.as_bytes()),
                chunks: chunks
                    .iter()
                    .map(|&(end_offset, chunk_id)| IndexEntry {
                        end_offset,
                        chunk_id,
                    })
                    .collect(),
            },
        }
    }

    fn loc(path: &str, offset: u64, len: u64) -> SeedLocation<'_> {
        SeedLocation { path, offset, len }
    }

    #[test]
    fn seed_plan_empty_listing_is_empty() {
        let a = DirArchive::empty();
        assert!(chunk_locations(&a, None).is_empty());
        assert!(seed_plan(&a, None).is_empty());
        let w: HashSet<ChunkId> = [id("x")].into_iter().collect();
        assert!(chunk_locations(&a, Some(&w)).is_empty());
    }

    #[test]
    fn seed_plan_offsets_across_multiple_chunks() {
        let (a, b, c) = (id("a"), id("b"), id("c"));
        let arch =
            DirArchive::new(0, vec![file("d/f.bin", &[(10, a), (25, b), (100, c)])]).unwrap();
        let m = chunk_locations(&arch, None);
        assert_eq!(m.len(), 3);
        assert_eq!(m[&a], vec![loc("d/f.bin", 0, 10)]); // first chunk at 0
        assert_eq!(m[&b], vec![loc("d/f.bin", 10, 15)]);
        assert_eq!(m[&c], vec![loc("d/f.bin", 25, 75)]);
    }

    #[test]
    fn seed_plan_duplicate_ids_keep_all_locations_in_listing_order() {
        let (a, b) = (id("a"), id("b"));
        let arch = DirArchive::new(
            0,
            vec![
                file("one", &[(4, a), (8, b), (12, a)]),
                file("two", &[(5, b)]),
            ],
        )
        .unwrap();
        let m = chunk_locations(&arch, None);
        assert_eq!(m[&a], vec![loc("one", 0, 4), loc("one", 8, 4)]);
        assert_eq!(m[&b], vec![loc("one", 4, 4), loc("two", 0, 5)]);
    }

    #[test]
    fn seed_plan_skips_dir_symlink_and_empty_file() {
        let a = id("a");
        let arch = DirArchive::new(
            0,
            vec![
                DirEntry {
                    path: "d".into(),
                    kind: DirEntryKind::Dir { mode: 0o755 },
                },
                DirEntry {
                    path: "ln".into(),
                    kind: DirEntryKind::Symlink {
                        mode: 0o777,
                        target: "f".into(),
                    },
                },
                file("empty", &[]),
                file("f", &[(3, a)]),
            ],
        )
        .unwrap();
        let m = chunk_locations(&arch, None);
        assert_eq!(m.len(), 1);
        assert_eq!(m[&a], vec![loc("f", 0, 3)]);
        assert!(m.values().flatten().all(|l| l.path == "f"));
    }

    #[test]
    fn seed_plan_wanted_filter() {
        let (a, b, c) = (id("a"), id("b"), id("c"));
        let arch = DirArchive::new(0, vec![file("f", &[(1, a), (2, b), (3, c)])]).unwrap();
        let w: HashSet<ChunkId> = [b, id("not-in-listing")].into_iter().collect();
        let m = chunk_locations(&arch, Some(&w));
        assert_eq!(m.len(), 1);
        assert_eq!(m[&b], vec![loc("f", 1, 1)]);
        let none: HashSet<ChunkId> = HashSet::new();
        assert!(chunk_locations(&arch, Some(&none)).is_empty());
    }

    #[test]
    fn seed_plan_rejects_escaping_paths() {
        let (a, b, c, d) = (id("a"), id("b"), id("c"), id("d"));
        // Bypass DirArchive::new validation: fields are public.
        let arch = DirArchive {
            format_version: 1,
            flags: 0,
            entries: vec![
                file("../escape", &[(1, a)]),
                file("/abs", &[(1, b)]),
                file("x/../../y", &[(1, c)]),
                file("ok", &[(1, d)]),
            ],
        };
        let m = chunk_locations(&arch, None);
        assert_eq!(m.len(), 1);
        assert_eq!(m[&d], vec![loc("ok", 0, 1)]);
    }

    #[test]
    fn seed_plan_skips_malformed_chunk_table() {
        let (a, b) = (id("a"), id("b"));
        let mut bad = file("bad", &[(5, a), (5, b)]); // non-increasing end_offset
        if let DirEntryKind::File { size, .. } = &mut bad.kind {
            *size = 5;
        }
        let arch = DirArchive {
            format_version: 1,
            flags: 0,
            entries: vec![bad, file("good", &[(2, b)])],
        };
        let m = chunk_locations(&arch, None);
        assert!(!m.contains_key(&a));
        assert_eq!(m[&b], vec![loc("good", 0, 2)]);
    }

    #[test]
    fn seed_plan_is_deterministic() {
        let (a, b) = (id("a"), id("b"));
        let arch =
            DirArchive::new(0, vec![file("p", &[(3, a), (6, b)]), file("q", &[(3, a)])]).unwrap();
        assert_eq!(chunk_locations(&arch, None), chunk_locations(&arch, None));
        assert_eq!(chunk_locations(&arch, None), seed_plan(&arch, None));
    }
}
