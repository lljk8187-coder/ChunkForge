//! Directory-archive path/chunk diff (Phase 7 M1).
//!
//! Compare two [`.cfdir`](crate::DirArchive) listings by relative **File** path
//! (Dir-only entries are ignored, matching [`crate::seed_file_map`]). Reports
//! path-level added / removed / content-changed / meta-changed, plus unique
//! chunk-id set statistics.
//!
//! Does **not** change `.cfdir` / `.cfidx` v1 byte layouts.

use crate::{DirArchive, DirEntry, DirEntryKind};
use chunkforge_chunk::ChunkId;
use std::collections::{HashMap, HashSet};

/// Path-keyed comparison of two directory archives (File entries only).
#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub struct DiffReport {
    /// Paths present in `right` but not in `left`.
    pub added: Vec<String>,
    /// Paths present in `left` but not in `right`.
    pub removed: Vec<String>,
    /// Same path, content differs (`blob_blake3` different, or size mismatch).
    pub changed: Vec<String>,
    /// Same path, same `blob_blake3`, but `mode` and/or `mtime_secs` differ.
    /// Never also listed in [`Self::changed`].
    pub meta_changed: Vec<String>,
    /// Unique chunk ids present in both archives.
    pub chunks_shared: usize,
    /// Unique chunk ids only in `left`.
    pub chunks_only_left: usize,
    /// Unique chunk ids only in `right`.
    pub chunks_only_right: usize,
}

/// Compare two directory archives by File relative path.
///
/// - **added** / **removed** / **changed** / **meta_changed** cover File paths only.
/// - Content change: `blob_blake3` differs (size mismatch is treated as content change).
/// - Meta-only: same `blob_blake3`, but `mode` and/or `mtime_secs` differ.
/// - Chunk stats: unique chunk ids from all File entries on each side.
///
/// Path vectors are sorted for determinism.
pub fn diff_dir_archives(left: &DirArchive, right: &DirArchive) -> DiffReport {
    let left_files = file_map(left);
    let right_files = file_map(right);

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    let mut meta_changed = Vec::new();

    for path in left_files.keys() {
        if !right_files.contains_key(path) {
            removed.push((*path).to_owned());
        }
    }
    for path in right_files.keys() {
        if !left_files.contains_key(path) {
            added.push((*path).to_owned());
        }
    }

    for (path, left_entry) in &left_files {
        let Some(right_entry) = right_files.get(path) else {
            continue;
        };
        let (l_mode, l_size, l_mtime, l_blake3) = file_meta(left_entry);
        let (r_mode, r_size, r_mtime, r_blake3) = file_meta(right_entry);

        if l_blake3 != r_blake3 || l_size != r_size {
            // Content differs (blake3 and/or size). Size mismatch without blake3
            // change is still treated as content change per Phase7 §3.1.
            changed.push((*path).to_owned());
        } else if l_mode != r_mode || l_mtime != r_mtime {
            meta_changed.push((*path).to_owned());
        }
    }

    added.sort();
    removed.sort();
    changed.sort();
    meta_changed.sort();

    let left_chunks = unique_chunk_ids(left);
    let right_chunks = unique_chunk_ids(right);
    let chunks_shared = left_chunks.intersection(&right_chunks).count();
    let chunks_only_left = left_chunks.difference(&right_chunks).count();
    let chunks_only_right = right_chunks.difference(&left_chunks).count();

    DiffReport {
        added,
        removed,
        changed,
        meta_changed,
        chunks_shared,
        chunks_only_left,
        chunks_only_right,
    }
}

fn file_map(arch: &DirArchive) -> HashMap<&str, &DirEntry> {
    arch.entries
        .iter()
        .filter(|e| matches!(e.kind, DirEntryKind::File { .. }))
        .map(|e| (e.path.as_str(), e))
        .collect()
}

fn file_meta(entry: &DirEntry) -> (u32, u64, u64, ChunkId) {
    match &entry.kind {
        DirEntryKind::File {
            mode,
            size,
            mtime_secs,
            blob_blake3,
            ..
        } => (*mode, *size, *mtime_secs, *blob_blake3),
        DirEntryKind::Dir { .. } => unreachable!("file_map only yields File entries"),
    }
}

fn unique_chunk_ids(arch: &DirArchive) -> HashSet<ChunkId> {
    arch.all_chunk_ids().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IndexEntry;

    fn file_entry(
        path: &str,
        mode: u32,
        size: u64,
        mtime_secs: u64,
        blob: ChunkId,
        chunks: Vec<IndexEntry>,
    ) -> DirEntry {
        DirEntry {
            path: path.into(),
            kind: DirEntryKind::File {
                mode,
                size,
                mtime_secs,
                blob_blake3: blob,
                chunks,
            },
        }
    }

    fn single_chunk_file(path: &str, mode: u32, mtime_secs: u64, content: &[u8]) -> DirEntry {
        let blob = ChunkId::hash(content);
        let chunk_id = ChunkId::hash(content);
        let size = content.len() as u64;
        file_entry(
            path,
            mode,
            size,
            mtime_secs,
            blob,
            if size == 0 {
                vec![]
            } else {
                vec![IndexEntry {
                    end_offset: size,
                    chunk_id,
                }]
            },
        )
    }

    fn base_archive() -> DirArchive {
        DirArchive::new(
            0,
            vec![
                single_chunk_file("a.txt", 0o644, 1_700_000_000, b"hello-v1"),
                DirEntry {
                    path: "sub".into(),
                    kind: DirEntryKind::Dir { mode: 0o755 },
                },
                single_chunk_file("sub/b.txt", 0o644, 1_700_000_001, b"beta"),
                single_chunk_file("c.txt", 0o600, 1_700_000_002, b"gamma"),
            ],
        )
        .unwrap()
    }

    #[test]
    fn identical_archives_empty_report() {
        let left = base_archive();
        let right = base_archive();
        let report = diff_dir_archives(&left, &right);
        assert!(report.added.is_empty(), "{report:?}");
        assert!(report.removed.is_empty(), "{report:?}");
        assert!(report.changed.is_empty(), "{report:?}");
        assert!(report.meta_changed.is_empty(), "{report:?}");
        assert_eq!(report.chunks_only_left, 0);
        assert_eq!(report.chunks_only_right, 0);
        // Three unique file chunks (a, b, c) shared.
        assert_eq!(report.chunks_shared, 3);
    }

    #[test]
    fn content_change_lists_changed_not_meta() {
        let left = base_archive();
        let mut right = base_archive();
        // Replace a.txt content.
        right.entries[0] = single_chunk_file("a.txt", 0o644, 1_700_000_000, b"hello-v2");

        let report = diff_dir_archives(&left, &right);
        assert_eq!(report.changed, vec!["a.txt".to_string()]);
        assert!(!report.meta_changed.contains(&"a.txt".to_string()));
        assert!(report.added.is_empty());
        assert!(report.removed.is_empty());
        // Old a-chunk only left, new a-chunk only right; b+c shared.
        assert_eq!(report.chunks_shared, 2);
        assert_eq!(report.chunks_only_left, 1);
        assert_eq!(report.chunks_only_right, 1);
    }

    #[test]
    fn deleted_file_lists_removed() {
        let left = base_archive();
        let right = DirArchive::new(
            0,
            vec![
                single_chunk_file("a.txt", 0o644, 1_700_000_000, b"hello-v1"),
                DirEntry {
                    path: "sub".into(),
                    kind: DirEntryKind::Dir { mode: 0o755 },
                },
                single_chunk_file("sub/b.txt", 0o644, 1_700_000_001, b"beta"),
                // c.txt deleted
            ],
        )
        .unwrap();

        let report = diff_dir_archives(&left, &right);
        assert_eq!(report.removed, vec!["c.txt".to_string()]);
        assert!(report.added.is_empty());
        assert!(report.changed.is_empty());
        assert!(report.meta_changed.is_empty());
        assert_eq!(report.chunks_only_left, 1); // gamma chunk
        assert_eq!(report.chunks_only_right, 0);
        assert_eq!(report.chunks_shared, 2);
    }

    #[test]
    fn added_file_lists_added() {
        let left = base_archive();
        let mut right = base_archive();
        right
            .entries
            .push(single_chunk_file("new.txt", 0o644, 1_700_000_010, b"fresh"));
        // Re-validate via new()
        let right = DirArchive::new(0, right.entries).unwrap();

        let report = diff_dir_archives(&left, &right);
        assert_eq!(report.added, vec!["new.txt".to_string()]);
        assert!(report.removed.is_empty());
        assert!(report.changed.is_empty());
        assert!(report.meta_changed.is_empty());
        assert_eq!(report.chunks_only_right, 1);
        assert_eq!(report.chunks_only_left, 0);
        assert_eq!(report.chunks_shared, 3);
    }

    #[test]
    fn meta_only_mode_mtime_not_in_changed() {
        let left = base_archive();
        let mut right = base_archive();
        // Same content blake3/size for a.txt, but different mode + mtime.
        right.entries[0] = single_chunk_file("a.txt", 0o755, 1_800_000_000, b"hello-v1");

        let report = diff_dir_archives(&left, &right);
        assert_eq!(report.meta_changed, vec!["a.txt".to_string()]);
        assert!(
            !report.changed.contains(&"a.txt".to_string()),
            "meta-only path must not appear in changed: {report:?}"
        );
        assert!(report.added.is_empty());
        assert!(report.removed.is_empty());
        // Chunk sets identical.
        assert_eq!(report.chunks_only_left, 0);
        assert_eq!(report.chunks_only_right, 0);
        assert_eq!(report.chunks_shared, 3);
    }

    #[test]
    fn dir_only_entries_ignored_for_path_lists() {
        let left = DirArchive::new(
            0,
            vec![
                DirEntry {
                    path: "emptydir".into(),
                    kind: DirEntryKind::Dir { mode: 0o755 },
                },
                single_chunk_file("f.txt", 0o644, 1, b"x"),
            ],
        )
        .unwrap();
        let right = DirArchive::new(
            0,
            vec![
                DirEntry {
                    path: "otherdir".into(),
                    kind: DirEntryKind::Dir { mode: 0o700 },
                },
                single_chunk_file("f.txt", 0o644, 1, b"x"),
            ],
        )
        .unwrap();

        let report = diff_dir_archives(&left, &right);
        assert!(report.added.is_empty(), "{report:?}");
        assert!(report.removed.is_empty(), "{report:?}");
        assert!(report.changed.is_empty(), "{report:?}");
        assert!(report.meta_changed.is_empty(), "{report:?}");
        assert_eq!(report.chunks_shared, 1);
    }

    #[test]
    fn paths_are_sorted() {
        let left = DirArchive::empty();
        let right = DirArchive::new(
            0,
            vec![
                single_chunk_file("z.txt", 0o644, 0, b"z"),
                single_chunk_file("a.txt", 0o644, 0, b"a"),
                single_chunk_file("m.txt", 0o644, 0, b"m"),
            ],
        )
        .unwrap();
        let report = diff_dir_archives(&left, &right);
        assert_eq!(
            report.added,
            vec![
                "a.txt".to_string(),
                "m.txt".to_string(),
                "z.txt".to_string()
            ]
        );
    }
}
