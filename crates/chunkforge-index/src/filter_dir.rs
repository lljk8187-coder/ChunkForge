//! Filter a [`.cfdir`](crate::DirArchive) listing by [`PathFilter`] (Phase 21 M1).
//!
//! Intended for read-only consumers such as FUSE `DirFs`
//! (filter then `DirFs::new`) and, later, `mount --path` / `--exclude` wiring.
//! Does **not** change `.cfdir` / `.cfidx` v1 encode bytes, nor
//! [`PathFilter`](crate::PathFilter) matching semantics.
//!
//! # Policy (pinned)
//!
//! - **File entries** are kept iff [`PathFilter::allows`] is true for their path
//!   (same include-then-exclude rules as `archive` / `extract` / `push` / `pull`).
//! - **Dir entries** are kept only when they are **ancestors** of at least one
//!   kept File (`dir` is a proper `/`-separated prefix of the file path). This
//!   preserves explicit directory modes for traversable parents while dropping
//!   unrelated empty dirs — matching extract/path “only reach included files”
//!   mental model. Parent dirs missing as explicit entries are still synthesized
//!   by DirFs from file path prefixes.
//! - Empty filter (`PathFilter::new([], [])`) ⇒ **identity** (clone of input) ≡
//!   1.10.0 full tree. No re-encode; listing bytes of the original archive are
//!   untouched.
//! - Non-matching Files (and non-ancestor Dirs) are omitted from the returned
//!   listing; the on-disk `.cfdir` is never rewritten by this helper.

use crate::{DirArchive, DirEntryKind, PathFilter};
use std::collections::HashSet;

/// Return a [`DirArchive`] containing only Files that pass `filter`, plus Dir
/// entries that are ancestors of those Files.
///
/// See module docs for the ancestor / empty-filter policy. Does not mutate
/// `archive`; does not encode or decode `.cfdir` bytes.
pub fn filter_dir_archive(archive: &DirArchive, filter: &PathFilter) -> DirArchive {
    // Empty paths + empty excludes ⇒ identity (≡ 1.10 full tree / no mount path flags).
    if filter.paths().is_empty() && filter.excludes().is_empty() {
        return archive.clone();
    }

    let kept_file_paths: HashSet<&str> = archive
        .entries
        .iter()
        .filter(|e| matches!(e.kind, DirEntryKind::File { .. }) && filter.allows(&e.path))
        .map(|e| e.path.as_str())
        .collect();

    let entries = archive
        .entries
        .iter()
        .filter(|e| match &e.kind {
            DirEntryKind::File { .. } => kept_file_paths.contains(e.path.as_str()),
            DirEntryKind::Dir { .. } => is_ancestor_of_any(&e.path, &kept_file_paths),
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
        assert!(!paths_of(&filtered).iter().any(|p| *p == "pkgs/empty"));
        assert!(!paths_of(&filtered).iter().any(|p| *p == "pkgs/bar"));
        assert!(!paths_of(&filtered).iter().any(|p| *p == "readme.txt"));
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
        // empty dir under pkgs is not an ancestor of any kept file → dropped.
        assert!(!paths.iter().any(|p| *p == "pkgs/empty"));
        assert!(!paths.iter().any(|p| *p == "readme.txt"));
        assert!(!paths.iter().any(|p| *p == "other/x.txt"));
    }

    #[test]
    fn exclude_only_without_path_include() {
        let arch = sample_tree();
        let filter = PathFilter::new(Vec::<String>::new(), ["*.txt"]).unwrap();
        let filtered = filter_dir_archive(&arch, &filter);
        // All sample files end in .txt → none kept; dirs without kept files gone.
        assert!(filtered.entries.is_empty());
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
}
