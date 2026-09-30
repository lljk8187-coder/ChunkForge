//! Directory-archive path/chunk diff (Phase 7 M1; Phase22-M3 Symlink).
//!
//! Compare two [`.cfdir`](crate::DirArchive) listings by relative **File** and
//! **Symlink** path (Dir-only entries are ignored). Reports path-level added /
//! removed / content-changed / meta-changed, plus unique chunk-id set
//! statistics. Symlink paths participate in added/removed/changed/meta_changed
//! (target change → `changed`; mode-only → `meta_changed`). Seed maps remain
//! File-only ([`crate::seed_file_map`]); Symlink is never reused via seed.
//!
//! Does **not** change `.cfdir` / `.cfidx` v1 byte layouts.

use crate::{DirArchive, DirEntry, DirEntryKind};
use chunkforge_chunk::ChunkId;
use std::collections::{HashMap, HashSet};

/// Path-keyed comparison of two directory archives (File + Symlink entries).
#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub struct DiffReport {
    /// Paths present in `right` but not in `left`.
    pub added: Vec<String>,
    /// Paths present in `left` but not in `right`.
    pub removed: Vec<String>,
    /// Same path, content differs (File: `blob_blake3`/size; Symlink: target).
    pub changed: Vec<String>,
    /// Same path, same content identity, but mode (and for File, `mtime_secs`) differ.
    /// Never also listed in [`Self::changed`].
    pub meta_changed: Vec<String>,
    /// Unique chunk ids present in both archives.
    pub chunks_shared: usize,
    /// Unique chunk ids only in `left`.
    pub chunks_only_left: usize,
    /// Unique chunk ids only in `right`.
    pub chunks_only_right: usize,
}

/// Compare two directory archives by File and Symlink relative path.
///
/// - **added** / **removed** / **changed** / **meta_changed** cover File and
///   Symlink paths (Dir-only ignored).
/// - File content change: `blob_blake3` differs (size mismatch is content change).
/// - Symlink content change: `target` differs → `changed`; mode-only → `meta_changed`.
/// - Kind mismatch at the same path (File vs Symlink) → `changed`.
/// - File meta-only: same `blob_blake3`, but `mode` and/or `mtime_secs` differ.
/// - Chunk stats: unique chunk ids from all File entries on each side (Symlink = 0).
///
/// Path vectors are sorted for determinism.
///
/// Equivalent to [`diff_dir_archives_with_progress`] with a no-op progress hook.
pub fn diff_dir_archives(left: &DirArchive, right: &DirArchive) -> DiffReport {
    diff_dir_archives_with_progress(left, right, || {})
}

/// Like [`diff_dir_archives`], but invokes `on_path` once per filtered **File**
/// or **Symlink** path in the union of both sides (Dir-only entries are
/// ignored, matching [`leaf_map`]). Tick count equals `|left∪right|` leaf
/// paths — never a post-hoc empty spin over TOTAL.
pub fn diff_dir_archives_with_progress(
    left: &DirArchive,
    right: &DirArchive,
    mut on_path: impl FnMut(),
) -> DiffReport {
    let left_leaves = leaf_map(left);
    let right_leaves = leaf_map(right);

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    let mut meta_changed = Vec::new();

    // Union of File + Symlink paths (Dir-only already excluded by leaf_map).
    let mut union: HashSet<&str> = left_leaves.keys().copied().collect();
    union.extend(right_leaves.keys().copied());

    for path in &union {
        on_path();
        match (left_leaves.get(path), right_leaves.get(path)) {
            (Some(_), None) => removed.push((*path).to_owned()),
            (None, Some(_)) => added.push((*path).to_owned()),
            (Some(left_entry), Some(right_entry)) => {
                match compare_leaves(left_entry, right_entry) {
                    LeafCompare::Changed => changed.push((*path).to_owned()),
                    LeafCompare::MetaChanged => meta_changed.push((*path).to_owned()),
                    LeafCompare::Same => {}
                }
            }
            (None, None) => unreachable!("union path must exist on at least one side"),
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

/// File + Symlink path map (Dir-only excluded). Used by listing↔listing diff.
fn leaf_map(arch: &DirArchive) -> HashMap<&str, &DirEntry> {
    arch.entries
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                DirEntryKind::File { .. } | DirEntryKind::Symlink { .. }
            )
        })
        .map(|e| (e.path.as_str(), e))
        .collect()
}

enum LeafCompare {
    Same,
    Changed,
    MetaChanged,
}

fn compare_leaves(left: &DirEntry, right: &DirEntry) -> LeafCompare {
    match (&left.kind, &right.kind) {
        (
            DirEntryKind::File {
                mode: l_mode,
                size: l_size,
                mtime_secs: l_mtime,
                blob_blake3: l_blake3,
                ..
            },
            DirEntryKind::File {
                mode: r_mode,
                size: r_size,
                mtime_secs: r_mtime,
                blob_blake3: r_blake3,
                ..
            },
        ) => {
            if l_blake3 != r_blake3 || l_size != r_size {
                // Content differs (blake3 and/or size). Size mismatch without blake3
                // change is still treated as content change per Phase7 §3.1.
                LeafCompare::Changed
            } else if l_mode != r_mode || l_mtime != r_mtime {
                LeafCompare::MetaChanged
            } else {
                LeafCompare::Same
            }
        }
        (
            DirEntryKind::Symlink {
                mode: l_mode,
                target: l_target,
            },
            DirEntryKind::Symlink {
                mode: r_mode,
                target: r_target,
            },
        ) => {
            if l_target != r_target {
                LeafCompare::Changed
            } else if l_mode != r_mode {
                LeafCompare::MetaChanged
            } else {
                LeafCompare::Same
            }
        }
        // File vs Symlink (or any other kind mismatch) at the same path → changed.
        _ => LeafCompare::Changed,
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

    #[test]
    fn with_progress_ticks_once_per_union_file_path() {
        let left = base_archive(); // a.txt, sub/b.txt, c.txt (+ Dir "sub" ignored)
        let right = DirArchive::new(
            0,
            vec![
                single_chunk_file("a.txt", 0o644, 1_700_000_000, b"hello-v1"),
                DirEntry {
                    path: "sub".into(),
                    kind: DirEntryKind::Dir { mode: 0o755 },
                },
                single_chunk_file("sub/b.txt", 0o644, 1_700_000_001, b"beta"),
                // c.txt removed; new.txt added
                single_chunk_file("new.txt", 0o644, 1, b"fresh"),
            ],
        )
        .unwrap();

        let mut ticks = 0usize;
        let report = diff_dir_archives_with_progress(&left, &right, || ticks += 1);
        // Union leaf paths: a.txt, sub/b.txt, c.txt, new.txt → 4
        assert_eq!(ticks, 4, "tick once per File/Symlink path in left∪right");
        assert_eq!(report.removed, vec!["c.txt".to_string()]);
        assert_eq!(report.added, vec!["new.txt".to_string()]);

        // Empty hook must match plain diff_dir_archives.
        let plain = diff_dir_archives(&left, &right);
        assert_eq!(report, plain);
    }

    #[test]
    fn with_progress_ignores_dir_only_in_total() {
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
        let mut ticks = 0usize;
        let _ = diff_dir_archives_with_progress(&left, &right, || ticks += 1);
        assert_eq!(ticks, 1, "Dir-only paths must not contribute to TOTAL");
    }

    fn symlink_entry(path: &str, mode: u32, target: &str) -> DirEntry {
        DirEntry {
            path: path.into(),
            kind: DirEntryKind::Symlink {
                mode,
                target: target.into(),
            },
        }
    }

    #[test]
    fn symlink_added_removed_target_changed() {
        let left = DirArchive::new(
            0,
            vec![
                single_chunk_file("a.txt", 0o644, 1, b"a"),
                symlink_entry("link", 0o777, "a.txt"),
                symlink_entry("gone", 0o777, "x"),
            ],
        )
        .unwrap();
        let right = DirArchive::new(
            0,
            vec![
                single_chunk_file("a.txt", 0o644, 1, b"a"),
                symlink_entry("link", 0o777, "b.txt"), // target changed
                symlink_entry("newlink", 0o777, "a.txt"), // added
                                                       // gone removed
            ],
        )
        .unwrap();

        let report = diff_dir_archives(&left, &right);
        assert_eq!(report.added, vec!["newlink".to_string()]);
        assert_eq!(report.removed, vec!["gone".to_string()]);
        assert_eq!(report.changed, vec!["link".to_string()]);
        assert!(report.meta_changed.is_empty(), "{report:?}");
        // Symlinks contribute no chunks; only a.txt shared.
        assert_eq!(report.chunks_shared, 1);
        assert_eq!(report.chunks_only_left, 0);
        assert_eq!(report.chunks_only_right, 0);
    }

    #[test]
    fn symlink_mode_only_is_meta_changed() {
        let left = DirArchive::new(0, vec![symlink_entry("link", 0o777, "a.txt")]).unwrap();
        let right = DirArchive::new(0, vec![symlink_entry("link", 0o755, "a.txt")]).unwrap();
        let report = diff_dir_archives(&left, &right);
        assert_eq!(report.meta_changed, vec!["link".to_string()]);
        assert!(report.changed.is_empty(), "{report:?}");
        assert!(report.added.is_empty());
        assert!(report.removed.is_empty());
    }

    #[test]
    fn file_vs_symlink_same_path_is_changed() {
        let left = DirArchive::new(0, vec![single_chunk_file("x", 0o644, 1, b"data")]).unwrap();
        let right = DirArchive::new(0, vec![symlink_entry("x", 0o777, "elsewhere")]).unwrap();
        let report = diff_dir_archives(&left, &right);
        assert_eq!(report.changed, vec!["x".to_string()]);
        assert!(report.added.is_empty());
        assert!(report.removed.is_empty());
    }

    #[test]
    fn with_progress_ticks_symlink_paths() {
        let left = DirArchive::new(0, vec![symlink_entry("a", 0o777, "t")]).unwrap();
        let right = DirArchive::new(
            0,
            vec![
                symlink_entry("a", 0o777, "t"),
                symlink_entry("b", 0o777, "u"),
            ],
        )
        .unwrap();
        let mut ticks = 0usize;
        let report = diff_dir_archives_with_progress(&left, &right, || ticks += 1);
        assert_eq!(ticks, 2);
        assert_eq!(report.added, vec!["b".to_string()]);
        assert_eq!(report, diff_dir_archives(&left, &right));
    }
}
