//! Filter a [`.cfdir`](crate::DirArchive) listing by [`PathFilter`] (Phase 21 M1).
//!
//! Intended for read-only consumers such as FUSE `DirFs`
//! (filter then `DirFs::new`) and, later, `mount --path` / `--exclude` wiring.
//! Does **not** change `.cfdir` / `.cfidx` v1 encode bytes, nor
//! [`PathFilter`](crate::PathFilter) matching semantics.
//!
//! # Policy (pinned)
//!
//! - **File** and **Symlink** entries are kept iff [`PathFilter::allows`] is true
//!   for their path (same include-then-exclude rules as `archive` / `extract` /
//!   `push` / `pull`). Symlink paths are treated like File paths for filtering.
//! - **Dir entries** are kept when they are **ancestors** of at least one
//!   kept File or Symlink (`dir` is a proper `/`-separated prefix of the leaf
//!   path), **or** (when the filter is non-empty) when [`PathFilter::allows`]
//!   is true for the Dir path itself — so path-matched leaf/empty Dirs from
//!   `archive --empty-dirs` survive the same way extract uses `allows` on Dir
//!   entries. Unrelated empty dirs that fail `allows` are still dropped. Do
//!   **not** synthesize ghost Dirs: only entries already present in the input
//!   archive may be kept. Parent dirs missing as explicit entries are still
//!   synthesized by DirFs from path prefixes.
//! - Empty filter (`PathFilter::new([], [])`) ⇒ **identity** (clone of input) ≡
//!   1.10.0 full tree. No re-encode; listing bytes of the original archive are
//!   untouched.
//! - Non-matching Files/Symlinks and Dir entries that are neither ancestors of
//!   kept leaves nor path-matched via `allows` are omitted from the returned
//!   listing; the on-disk `.cfdir` is never rewritten by this helper.

use crate::{DirArchive, DirEntryKind, PathFilter};
use std::collections::HashSet;

/// Return a [`DirArchive`] containing only Files/Symlinks that pass `filter`,
/// plus Dir entries that are ancestors of those leaves **or** (non-empty
/// filter) path-matched leaf/empty Dirs that pass [`PathFilter::allows`].
///
/// See module docs for the ancestor / empty-filter policy. Does not mutate
/// `archive`; does not encode or decode `.cfdir` bytes.
pub fn filter_dir_archive(archive: &DirArchive, filter: &PathFilter) -> DirArchive {
    // Empty paths + empty excludes ⇒ identity (≡ 1.10 full tree / no mount path flags).
    if filter.paths().is_empty() && filter.excludes().is_empty() {
        return archive.clone();
    }

    // Keep Files and Symlinks that pass PathFilter (Symlink path treated like File).
    // Dir kept if ancestor of a kept File/Symlink leaf, OR PathFilter::allows
    // (path-matched leaf/empty Dir; filter is known non-empty here).
    let kept_leaf_paths: HashSet<&str> = archive
        .entries
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                DirEntryKind::File { .. } | DirEntryKind::Symlink { .. }
            ) && filter.allows(&e.path)
        })
        .map(|e| e.path.as_str())
        .collect();

    let entries = archive
        .entries
        .iter()
        .filter(|e| match &e.kind {
            DirEntryKind::File { .. } | DirEntryKind::Symlink { .. } => {
                kept_leaf_paths.contains(e.path.as_str())
            }
            DirEntryKind::Dir { .. } => {
                is_ancestor_of_any(&e.path, &kept_leaf_paths) || filter.allows(&e.path)
            }
        })
        .cloned()
        .collect();

    DirArchive {
        format_version: archive.format_version,
        flags: archive.flags,
        entries,
    }
}

/// `dir` is a proper ancestor of `file` when `file` is under `dir/` (not equal).
fn is_ancestor(dir: &str, file: &str) -> bool {
    file.starts_with(dir) && file.as_bytes().get(dir.len()) == Some(&b'/')
}

fn is_ancestor_of_any(dir: &str, files: &HashSet<&str>) -> bool {
    files.iter().any(|f| is_ancestor(dir, f))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DirEntry, IndexEntry};
    use chunkforge_chunk::ChunkId;

    fn file_entry(path: &str) -> DirEntry {
        DirEntry {
            path: path.into(),
            kind: DirEntryKind::File {
                mode: 0o644,
                size: 0,
                mtime_secs: 0,
                blob_blake3: ChunkId::hash(b""),
                chunks: Vec::<IndexEntry>::new(),
            },
        }
    }

    fn dir_entry(path: &str, mode: u32) -> DirEntry {
        DirEntry {
            path: path.into(),
            kind: DirEntryKind::Dir { mode },
        }
    }

    fn sample_tree() -> DirArchive {
        DirArchive::new(
            0,
            vec![
                file_entry("readme.txt"),
                dir_entry("pkgs", 0o755),
                dir_entry("pkgs/foo", 0o755),
                file_entry("pkgs/foo/a.txt"),
                file_entry("pkgs/foo/b.txt"),
                dir_entry("pkgs/bar", 0o700),
                file_entry("pkgs/bar/c.txt"),
                dir_entry("pkgs/empty", 0o755),
                file_entry("other/x.txt"),
            ],
        )
        .unwrap()
    }

    fn paths_of(arch: &DirArchive) -> Vec<&str> {
        arch.entries.iter().map(|e| e.path.as_str()).collect()
    }

    #[test]
    fn empty_filter_is_identity() {
        let arch = sample_tree();
        let filter = PathFilter::new(Vec::<String>::new(), Vec::<String>::new()).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        assert_eq!(filtered, arch);
        // Encode bytes of the *original* listing are unchanged by filtering
        // (identity returns a clone; helper never rewrites on-disk bytes).
        assert_eq!(arch.encode().unwrap(), filtered.encode().unwrap());
    }

    #[test]
    fn include_prefix_keeps_files_and_ancestor_dirs() {
        let arch = sample_tree();
        let filter = PathFilter::new(["pkgs/foo"], Vec::<String>::new()).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        assert_eq!(
            paths_of(&filtered),
            vec!["pkgs", "pkgs/foo", "pkgs/foo/a.txt", "pkgs/foo/b.txt"]
        );
        // Unrelated empty dir and other subtree dropped.
        assert!(!paths_of(&filtered).contains(&"pkgs/empty"));
        assert!(!paths_of(&filtered).contains(&"pkgs/bar"));
        assert!(!paths_of(&filtered).contains(&"readme.txt"));
        // Ancestor Dir mode preserved.
        let pkgs_foo = filtered
            .entries
            .iter()
            .find(|e| e.path == "pkgs/foo")
            .unwrap();
        assert!(matches!(pkgs_foo.kind, DirEntryKind::Dir { mode: 0o755 }));
    }

    #[test]
    fn exclude_drops_matching_files_and_orphan_dirs() {
        let arch = sample_tree();
        // Include pkgs/, exclude pkgs/bar/ → foo files + ancestors; bar gone.
        let filter = PathFilter::new(["pkgs"], ["pkgs/bar/"]).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        let paths = paths_of(&filtered);
        assert!(paths.contains(&"pkgs/foo/a.txt"));
        assert!(paths.contains(&"pkgs/foo/b.txt"));
        assert!(paths.contains(&"pkgs"));
        assert!(paths.contains(&"pkgs/foo"));
        assert!(!paths.iter().any(|p| p.starts_with("pkgs/bar")));
        // empty dir under pkgs passes PathFilter::allows → kept as leaf Dir (Phase26).
        assert!(paths.contains(&"pkgs/empty"));
        assert!(!paths.contains(&"readme.txt"));
        assert!(!paths.contains(&"other/x.txt"));
    }

    #[test]
    fn exclude_only_without_path_include() {
        let arch = sample_tree();
        let filter = PathFilter::new(Vec::<String>::new(), ["*.txt"]).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        // All sample files end in .txt → none kept as leaves.
        // Explicit Dir entries still pass allows (*.txt does not match them) → kept.
        assert_eq!(
            paths_of(&filtered),
            vec!["pkgs", "pkgs/foo", "pkgs/bar", "pkgs/empty"]
        );
        assert!(
            filtered
                .entries
                .iter()
                .all(|e| matches!(e.kind, DirEntryKind::Dir { .. }))
        );
    }

    #[test]
    fn no_matching_files_yields_empty_archive() {
        let arch = sample_tree();
        let filter = PathFilter::new(["missing/prefix"], Vec::<String>::new()).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        assert!(filtered.entries.is_empty());
        assert_eq!(filtered.format_version, arch.format_version);
        assert_eq!(filtered.flags, arch.flags);
    }

    #[test]
    fn preserves_flags_and_order_of_kept_entries() {
        let mut arch = sample_tree();
        arch.flags = 0x0001;
        let filter = PathFilter::new(["pkgs/foo", "readme.txt"], Vec::<String>::new()).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        assert_eq!(filtered.flags, 0x0001);
        // Order follows original archive order among kept entries.
        assert_eq!(
            paths_of(&filtered),
            vec![
                "readme.txt",
                "pkgs",
                "pkgs/foo",
                "pkgs/foo/a.txt",
                "pkgs/foo/b.txt"
            ]
        );
    }

    #[test]
    fn original_archive_encode_unchanged_after_filter() {
        let arch = sample_tree();
        let before = arch.encode().unwrap();
        let filter = PathFilter::new(["pkgs/foo"], Vec::<String>::new()).unwrap();
        let _ = filter_dir_archive(&arch, &filter);
        assert_eq!(arch.encode().unwrap(), before);
    }

    fn symlink_entry(path: &str, target: &str) -> DirEntry {
        DirEntry {
            path: path.into(),
            kind: DirEntryKind::Symlink {
                mode: 0o777,
                target: target.into(),
            },
        }
    }

    fn sample_tree_with_symlinks() -> DirArchive {
        DirArchive::new(
            0,
            vec![
                file_entry("readme.txt"),
                dir_entry("pkgs", 0o755),
                dir_entry("pkgs/foo", 0o755),
                file_entry("pkgs/foo/a.txt"),
                symlink_entry("pkgs/foo/link", "a.txt"),
                dir_entry("pkgs/bar", 0o700),
                symlink_entry("pkgs/bar/slink", "../foo/a.txt"),
                file_entry("other/x.txt"),
            ],
        )
        .unwrap()
    }

    #[test]
    fn empty_filter_identity_includes_symlinks() {
        let arch = sample_tree_with_symlinks();
        assert_eq!(arch.format_version, crate::DIR_FORMAT_VERSION_V2);
        let filter = PathFilter::new(Vec::<String>::new(), Vec::<String>::new()).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        assert_eq!(filtered, arch);
        assert_eq!(arch.encode().unwrap(), filtered.encode().unwrap());
        assert!(
            filtered.entries.iter().any(
                |e| matches!(e.kind, DirEntryKind::Symlink { .. }) && e.path == "pkgs/foo/link"
            )
        );
    }

    #[test]
    fn filter_keeps_symlink_path_and_ancestors() {
        let arch = sample_tree_with_symlinks();
        let filter = PathFilter::new(["pkgs/foo"], Vec::<String>::new()).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        let paths = paths_of(&filtered);
        assert_eq!(
            paths,
            vec!["pkgs", "pkgs/foo", "pkgs/foo/a.txt", "pkgs/foo/link",]
        );
        let link = filtered
            .entries
            .iter()
            .find(|e| e.path == "pkgs/foo/link")
            .unwrap();
        match &link.kind {
            DirEntryKind::Symlink { target, .. } => assert_eq!(target, "a.txt"),
            other => panic!("expected Symlink, got {other:?}"),
        }
        // bar symlink and other dropped
        assert!(!paths.iter().any(|p| p.starts_with("pkgs/bar")));
        assert!(!paths.contains(&"other/x.txt"));
        assert!(!paths.contains(&"readme.txt"));
    }

    #[test]
    fn filter_symlink_only_prefix_keeps_ancestors() {
        let arch = sample_tree_with_symlinks();
        // Exact path include of a symlink leaf
        let filter = PathFilter::new(["pkgs/bar/slink"], Vec::<String>::new()).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        let paths = paths_of(&filtered);
        assert!(paths.contains(&"pkgs"));
        assert!(paths.contains(&"pkgs/bar"));
        assert!(paths.contains(&"pkgs/bar/slink"));
        assert!(!paths.contains(&"pkgs/foo/a.txt"));
        assert!(!paths.contains(&"pkgs/foo/link"));
    }

    #[test]
    fn path_matched_empty_leaf_dir_is_kept() {
        let arch = sample_tree();
        // Exact path hit on an empty leaf Dir (no File/Symlink under it).
        let filter = PathFilter::new(["pkgs/empty"], Vec::<String>::new()).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        let paths = paths_of(&filtered);
        assert!(paths.contains(&"pkgs/empty"));
        let leaf = filtered
            .entries
            .iter()
            .find(|e| e.path == "pkgs/empty")
            .unwrap();
        assert!(matches!(leaf.kind, DirEntryKind::Dir { mode: 0o755 }));
        // No File/Symlink kept; ancestor pkgs does not allow under exact pkgs/empty.
        assert!(!paths.contains(&"pkgs"));
        assert!(!paths.contains(&"pkgs/foo"));
        assert!(!paths.contains(&"pkgs/foo/a.txt"));
        assert!(!paths.contains(&"readme.txt"));
        // Dedup: one Dir entry only.
        assert_eq!(paths.iter().filter(|p| **p == "pkgs/empty").count(), 1);
        assert_eq!(filtered.entries.len(), 1);
    }

    #[test]
    fn path_prefix_keeps_empty_leaf_dir_under_include() {
        let arch = sample_tree();
        // --path pkgs ⇒ pkgs/empty allows → kept alongside File leaves + ancestors.
        let filter = PathFilter::new(["pkgs"], Vec::<String>::new()).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        let paths = paths_of(&filtered);
        assert!(paths.contains(&"pkgs/empty"));
        assert!(paths.contains(&"pkgs/foo/a.txt"));
        assert!(paths.contains(&"pkgs/bar/c.txt"));
        assert!(!paths.contains(&"readme.txt"));
        assert!(!paths.contains(&"other/x.txt"));
    }

    #[test]
    fn unrelated_empty_dir_still_dropped() {
        let arch = sample_tree();
        let filter = PathFilter::new(["pkgs/foo"], Vec::<String>::new()).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        // pkgs/empty does not allow under pkgs/foo → still dropped (unchanged).
        assert!(!paths_of(&filtered).contains(&"pkgs/empty"));
        assert_eq!(
            paths_of(&filtered),
            vec!["pkgs", "pkgs/foo", "pkgs/foo/a.txt", "pkgs/foo/b.txt"]
        );
    }

    #[test]
    fn archive_without_explicit_dir_entries_unchanged() {
        // No Dir entries at all → filtering cannot invent ghost Dirs (≡ 1.15).
        let arch = DirArchive::new(
            0,
            vec![
                file_entry("readme.txt"),
                file_entry("pkgs/foo/a.txt"),
                file_entry("other/x.txt"),
            ],
        )
        .unwrap();
        let filter = PathFilter::new(["pkgs/foo"], Vec::<String>::new()).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        assert_eq!(paths_of(&filtered), vec!["pkgs/foo/a.txt"]);
        assert!(
            filtered
                .entries
                .iter()
                .all(|e| matches!(e.kind, DirEntryKind::File { .. }))
        );
    }

    #[test]
    fn sample_only_ancestor_dirs_equiv_old_file_symlink_behavior() {
        // Listing has ancestor Dirs but no empty leaf Dir → ≡ pre-Phase26 result
        // for a file-scoped path filter.
        let arch = DirArchive::new(
            0,
            vec![
                dir_entry("pkgs", 0o755),
                dir_entry("pkgs/foo", 0o755),
                file_entry("pkgs/foo/a.txt"),
                file_entry("other/x.txt"),
            ],
        )
        .unwrap();
        let filter = PathFilter::new(["pkgs/foo"], Vec::<String>::new()).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        assert_eq!(
            paths_of(&filtered),
            vec!["pkgs", "pkgs/foo", "pkgs/foo/a.txt"]
        );
    }

    #[test]
    fn empty_leaf_dir_kept_once_when_also_ancestor() {
        // Dir that both allows and is ancestor of a kept File is kept once.
        let arch = DirArchive::new(
            0,
            vec![
                dir_entry("pkgs", 0o755),
                dir_entry("pkgs/foo", 0o755),
                file_entry("pkgs/foo/a.txt"),
            ],
        )
        .unwrap();
        let filter = PathFilter::new(["pkgs"], Vec::<String>::new()).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        let paths = paths_of(&filtered);
        assert_eq!(paths, vec!["pkgs", "pkgs/foo", "pkgs/foo/a.txt"]);
        assert_eq!(paths.iter().filter(|p| **p == "pkgs").count(), 1);
        assert_eq!(paths.iter().filter(|p| **p == "pkgs/foo").count(), 1);
    }

    #[test]
    fn identity_keeps_all_dirs_including_empty_leaf() {
        let arch = sample_tree();
        assert!(paths_of(&arch).contains(&"pkgs/empty"));
        let filter = PathFilter::new(Vec::<String>::new(), Vec::<String>::new()).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        assert_eq!(filtered, arch);
        assert!(paths_of(&filtered).contains(&"pkgs/empty"));
    }
}
