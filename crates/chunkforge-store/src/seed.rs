//! Materialize-side chunk-level **seed source** (Phase 27 M1).
//!
//! [`SeedSource`] serves chunk plaintext from a tree that already exists on
//! local disk (the *seed root*), at locations computed from a prior `.cfdir`
//! listing (see `chunkforge_index::chunk_locations`). It implements the frozen
//! [`ChunkSource`] trait unchanged.
//!
//! Contract:
//! - **Read-only.** Never writes, deletes, or fills any cache; wrap order in the
//!   CLI is `Fallback[Seed, Cache?(…)]`, so seed hits never reach a cache.
//! - **Every byte verified.** A location is used only if `BLAKE3(bytes) == id`.
//! - **Opportunistic.** Any per-location failure (missing file, not a regular
//!   file, symlink anywhere under the root, short file / short read, IO error,
//!   BLAKE3 mismatch) counts as `stale` and the next location is tried. When no
//!   location verifies, `get` returns [`SourceError::NotFound`] so a
//!   [`FallbackSource`](crate::FallbackSource) falls through to the real chain.
//!   The seed never surfaces `Io` / `Corrupt` / `Backend`.
//! - **No symlink following.** Every path component below the root is checked
//!   with `symlink_metadata`; any symlink component rejects the location. After
//!   `open`, the handle's `(dev, ino)` must equal the checked final component.
//! - Relative paths that are absolute or contain `..` / `.` / prefix components
//!   are dropped at construction.

use crate::source::{ChunkSource, SourceError};
use chunkforge_chunk::ChunkId;
use std::collections::HashMap;
use std::fs::File;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Snapshot of [`SeedSource`] observation counters (`get` path only).
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct SeedStats {
    /// `get` calls answered from the seed tree (verified).
    pub hits: u64,
    /// Individual locations tried and rejected (missing / symlink / short / IO / BLAKE3 mismatch).
    pub stale: u64,
    /// `get` calls that returned [`SourceError::NotFound`] (no location, or all stale).
    pub misses: u64,
    /// Plaintext bytes returned by hits.
    pub bytes: u64,
}

#[derive(Debug, Clone)]
struct SeedLoc {
    rel: PathBuf,
    offset: u64,
    len: u64,
}

/// Read-only, BLAKE3-verified chunk source backed by a local seed tree.
#[derive(Debug)]
pub struct SeedSource {
    root: PathBuf,
    locs: HashMap<ChunkId, Vec<SeedLoc>>,
    hits: AtomicU64,
    stale: AtomicU64,
    misses: AtomicU64,
    bytes: AtomicU64,
}

/// Outcome of trying one location.
enum Attempt {
    Hit(Vec<u8>),
    Stale,
}

impl SeedSource {
    /// Build from a seed `root` and `(chunk_id, relative_path, offset, len)` tuples.
    ///
    /// Locations for the same id are tried in iteration order. Tuples whose
    /// relative path is empty / absolute / contains `..`, `.` or a prefix
    /// component, or whose `len == 0` / `offset + len` overflows, are dropped.
    pub fn new(
        root: PathBuf,
        locs: impl IntoIterator<Item = (ChunkId, PathBuf, u64, u64)>,
    ) -> Self {
        let mut map: HashMap<ChunkId, Vec<SeedLoc>> = HashMap::new();
        for (id, rel, offset, len) in locs {
            if len == 0 || offset.checked_add(len).is_none() || !is_safe_relative(&rel) {
                continue;
            }
            map.entry(id)
                .or_default()
                .push(SeedLoc { rel, offset, len });
        }
        Self {
            root,
            locs: map,
            hits: AtomicU64::new(0),
            stale: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
        }
    }

    /// Seed root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Number of distinct chunk ids with at least one planned location.
    pub fn planned_chunks(&self) -> usize {
        self.locs.len()
    }

    /// Verified seed hits on `get`.
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    /// Locations rejected on `get` (missing / symlink / short / IO / mismatch).
    pub fn stale(&self) -> u64 {
        self.stale.load(Ordering::Relaxed)
    }

    /// `get` calls that returned NotFound.
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }

    /// Plaintext bytes served by hits.
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// Counter snapshot.
    pub fn stats(&self) -> SeedStats {
        SeedStats {
            hits: self.hits(),
            stale: self.stale(),
            misses: self.misses(),
            bytes: self.bytes(),
        }
    }

    /// Try each location; `count` controls whether counters move (`get` only).
    fn lookup(&self, id: &ChunkId, count: bool) -> Option<Vec<u8>> {
        for loc in self.locs.get(id).into_iter().flatten() {
            match self.try_loc(id, loc) {
                Attempt::Hit(buf) => {
                    if count {
                        self.hits.fetch_add(1, Ordering::Relaxed);
                        self.bytes.fetch_add(buf.len() as u64, Ordering::Relaxed);
                    }
                    return Some(buf);
                }
                Attempt::Stale => {
                    if count {
                        self.stale.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
        if count {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
        None
    }

    fn try_loc(&self, id: &ChunkId, loc: &SeedLoc) -> Attempt {
        let Some(path) = resolve_no_symlinks(&self.root, &loc.rel) else {
            return Attempt::Stale;
        };
        let Ok(pre) = std::fs::symlink_metadata(&path) else {
            return Attempt::Stale;
        };
        // `resolve_no_symlinks` already checked; re-check right before open.
        if !pre.file_type().is_file() {
            return Attempt::Stale;
        }
        let Some(end) = loc.offset.checked_add(loc.len) else {
            return Attempt::Stale;
        };
        if pre.len() < end {
            return Attempt::Stale;
        }
        let Ok(file) = File::open(&path) else {
            return Attempt::Stale;
        };
        // Guard against a swap between the check and the open.
        let Ok(post) = file.metadata() else {
            return Attempt::Stale;
        };
        if !post.file_type().is_file() || post.dev() != pre.dev() || post.ino() != pre.ino() {
            return Attempt::Stale;
        }
        let Ok(len) = usize::try_from(loc.len) else {
            return Attempt::Stale;
        };
        let mut buf = vec![0u8; len];
        if file.read_exact_at(&mut buf, loc.offset).is_err() {
            return Attempt::Stale;
        }
        if ChunkId::hash(&buf) != *id {
            return Attempt::Stale;
        }
        Attempt::Hit(buf)
    }
}

impl ChunkSource for SeedSource {
    /// Honest, verified presence: performs the same checked read as `get`
    /// but does **not** move counters.
    fn has(&self, id: &ChunkId) -> Result<bool, SourceError> {
        Ok(self.lookup(id, false).is_some())
    }

    fn get(&self, id: &ChunkId) -> Result<Vec<u8>, SourceError> {
        self.lookup(id, true).ok_or(SourceError::NotFound(*id))
    }
}

/// Relative path made only of normal components (no root / prefix / `.` / `..`).
fn is_safe_relative(rel: &Path) -> bool {
    let mut any = false;
    for c in rel.components() {
        match c {
            Component::Normal(_) => any = true,
            _ => return false,
        }
    }
    any
}

/// Join `rel` under `root`, refusing any symlink component below the root.
/// Intermediate components must be directories; the final one a regular file.
fn resolve_no_symlinks(root: &Path, rel: &Path) -> Option<PathBuf> {
    let comps: Vec<_> = rel.components().collect();
    let mut cur = root.to_path_buf();
    for (i, c) in comps.iter().enumerate() {
        let Component::Normal(name) = c else {
            return None;
        };
        cur.push(name);
        let md = std::fs::symlink_metadata(&cur).ok()?;
        let ft = md.file_type();
        if ft.is_symlink() {
            return None;
        }
        let last = i + 1 == comps.len();
        if (last && !ft.is_file()) || (!last && !ft.is_dir()) {
            return None;
        }
    }
    if comps.is_empty() { None } else { Some(cur) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compression, FallbackSource, Store};
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    /// Tree: `d/f.bin` = A(10) || B(20) || C(5); returns ids.
    fn seed_tree(root: &Path) -> (ChunkId, ChunkId, ChunkId) {
        let a = vec![b'a'; 10];
        let b = vec![b'b'; 20];
        let c = vec![b'c'; 5];
        fs::create_dir_all(root.join("d")).unwrap();
        let mut all = a.clone();
        all.extend_from_slice(&b);
        all.extend_from_slice(&c);
        fs::write(root.join("d/f.bin"), &all).unwrap();
        (ChunkId::hash(&a), ChunkId::hash(&b), ChunkId::hash(&c))
    }

    fn plan(a: ChunkId, b: ChunkId, c: ChunkId, rel: &str) -> Vec<(ChunkId, PathBuf, u64, u64)> {
        vec![
            (a, rel.into(), 0, 10),
            (b, rel.into(), 10, 20),
            (c, rel.into(), 30, 5),
        ]
    }

    #[test]
    fn seed_source_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SeedSource>();
        let _: Box<dyn ChunkSource> = Box::new(SeedSource::new(PathBuf::new(), []));
    }

    #[test]
    fn seed_hit_returns_verified_plaintext_and_counts() {
        let dir = tempdir().unwrap();
        let (a, b, c) = seed_tree(dir.path());
        let s = SeedSource::new(dir.path().into(), plan(a, b, c, "d/f.bin"));
        assert_eq!(s.planned_chunks(), 3);
        assert_eq!(s.get(&a).unwrap(), vec![b'a'; 10]);
        assert_eq!(s.get(&b).unwrap(), vec![b'b'; 20]);
        assert_eq!(s.get(&c).unwrap(), vec![b'c'; 5]);
        assert_eq!(
            s.stats(),
            SeedStats {
                hits: 3,
                stale: 0,
                misses: 0,
                bytes: 35
            }
        );
    }

    #[test]
    fn seed_tampered_file_is_stale_notfound() {
        let dir = tempdir().unwrap();
        let (a, b, c) = seed_tree(dir.path());
        let p = dir.path().join("d/f.bin");
        let mut bytes = fs::read(&p).unwrap();
        bytes[12] ^= 0xff; // inside chunk B
        fs::write(&p, &bytes).unwrap();
        let s = SeedSource::new(dir.path().into(), plan(a, b, c, "d/f.bin"));
        assert!(matches!(s.get(&b), Err(SourceError::NotFound(x)) if x == b));
        assert_eq!(s.get(&a).unwrap(), vec![b'a'; 10]); // untouched chunk still hits
        let st = s.stats();
        assert_eq!((st.hits, st.stale, st.misses, st.bytes), (1, 1, 1, 10));
    }

    #[test]
    fn seed_second_location_used_when_first_stale() {
        let dir = tempdir().unwrap();
        let (a, _, _) = seed_tree(dir.path());
        fs::write(dir.path().join("other"), vec![b'a'; 10]).unwrap();
        let s = SeedSource::new(
            dir.path().into(),
            vec![
                (a, PathBuf::from("gone"), 0, 10),
                (a, PathBuf::from("d/f.bin"), 1, 10), // wrong offset → mismatch
                (a, PathBuf::from("other"), 0, 10),
            ],
        );
        assert_eq!(s.get(&a).unwrap(), vec![b'a'; 10]);
        let st = s.stats();
        assert_eq!((st.hits, st.stale, st.misses), (1, 2, 0));
    }

    #[test]
    fn seed_missing_file_is_notfound() {
        let dir = tempdir().unwrap();
        let (a, b, c) = seed_tree(dir.path());
        fs::remove_file(dir.path().join("d/f.bin")).unwrap();
        let s = SeedSource::new(dir.path().into(), plan(a, b, c, "d/f.bin"));
        assert!(matches!(s.get(&a), Err(SourceError::NotFound(_))));
        assert!(!s.has(&a).unwrap());
        let st = s.stats();
        assert_eq!((st.hits, st.stale, st.misses), (0, 1, 1));
    }

    #[test]
    fn seed_truncated_file_short_read_is_notfound() {
        let dir = tempdir().unwrap();
        let (a, b, c) = seed_tree(dir.path());
        let p = dir.path().join("d/f.bin");
        let bytes = fs::read(&p).unwrap();
        fs::write(&p, &bytes[..32]).unwrap(); // C (30..35) now short
        let s = SeedSource::new(dir.path().into(), plan(a, b, c, "d/f.bin"));
        assert!(matches!(s.get(&c), Err(SourceError::NotFound(x)) if x == c));
        assert_eq!(s.get(&b).unwrap(), vec![b'b'; 20]);
        let st = s.stats();
        assert_eq!((st.hits, st.stale, st.misses), (1, 1, 1));
    }

    #[test]
    fn seed_refuses_symlinked_file() {
        let dir = tempdir().unwrap();
        let (a, b, c) = seed_tree(dir.path());
        symlink(dir.path().join("d/f.bin"), dir.path().join("link.bin")).unwrap();
        let s = SeedSource::new(dir.path().into(), plan(a, b, c, "link.bin"));
        assert!(matches!(s.get(&a), Err(SourceError::NotFound(_))));
        assert!(!s.has(&a).unwrap());
        let st = s.stats();
        assert_eq!((st.hits, st.stale, st.misses), (0, 1, 1));
    }

    #[test]
    fn seed_refuses_symlinked_parent_dir() {
        let dir = tempdir().unwrap();
        let (a, b, c) = seed_tree(dir.path());
        symlink(dir.path().join("d"), dir.path().join("ld")).unwrap();
        let s = SeedSource::new(dir.path().into(), plan(a, b, c, "ld/f.bin"));
        assert!(matches!(s.get(&b), Err(SourceError::NotFound(_))));
        // Same bytes through the real dir still hit.
        let ok = SeedSource::new(dir.path().into(), plan(a, b, c, "d/f.bin"));
        assert!(ok.get(&b).is_ok());
        assert_eq!(s.stats().stale, 1);
    }

    #[test]
    fn seed_refuses_non_regular_file() {
        let dir = tempdir().unwrap();
        let (a, _, _) = seed_tree(dir.path());
        let s = SeedSource::new(dir.path().into(), vec![(a, PathBuf::from("d"), 0, 10)]);
        assert!(matches!(s.get(&a), Err(SourceError::NotFound(_))));
    }

    #[test]
    fn seed_empty_plan_is_notfound() {
        let dir = tempdir().unwrap();
        let (a, _, _) = seed_tree(dir.path());
        let s = SeedSource::new(dir.path().into(), []);
        assert_eq!(s.planned_chunks(), 0);
        assert!(matches!(s.get(&a), Err(SourceError::NotFound(x)) if x == a));
        assert!(!s.has(&a).unwrap());
        assert_eq!(
            s.stats(),
            SeedStats {
                hits: 0,
                stale: 0,
                misses: 1,
                bytes: 0
            }
        );
    }

    #[test]
    fn seed_drops_unsafe_relative_paths() {
        let dir = tempdir().unwrap();
        let inner = dir.path().join("root");
        let (a, b, c) = seed_tree(&inner);
        // A copy of the bytes outside the root must not be reachable.
        let abs = inner.join("d/f.bin");
        let s = SeedSource::new(
            inner.clone(),
            vec![
                (a, PathBuf::from("../root/d/f.bin"), 0, 10),
                (b, abs, 10, 20),
                (c, PathBuf::from("./d/f.bin"), 30, 5),
                (c, PathBuf::from(""), 30, 5),
                (a, PathBuf::from("d/f.bin"), 0, 0), // len 0 dropped
            ],
        );
        assert_eq!(s.planned_chunks(), 0);
        assert!(s.get(&a).is_err() && s.get(&b).is_err() && s.get(&c).is_err());
    }

    #[test]
    fn seed_has_is_verified_and_does_not_move_counters() {
        let dir = tempdir().unwrap();
        let (a, b, c) = seed_tree(dir.path());
        let s = SeedSource::new(dir.path().into(), plan(a, b, c, "d/f.bin"));
        assert!(s.has(&a).unwrap());
        assert!(!s.has(&ChunkId::hash(b"absent")).unwrap());
        assert_eq!(s.stats(), SeedStats::default());
    }

    #[test]
    fn seed_is_read_only_and_fallback_falls_through_on_miss() {
        let dir = tempdir().unwrap();
        let seed_root = dir.path().join("seed");
        let (a, b, c) = seed_tree(&seed_root);
        let before = fs::read(seed_root.join("d/f.bin")).unwrap();
        let store = Store::create(dir.path().join("store"), Compression::None).unwrap();
        let (fresh, _) = store.put(b"only-in-store").unwrap();
        let seed =
            std::sync::Arc::new(SeedSource::new(seed_root.clone(), plan(a, b, c, "d/f.bin")));
        let fb = FallbackSource::new(vec![
            Box::new(seed.clone()) as Box<dyn ChunkSource>,
            Box::new(store),
        ])
        .unwrap();
        assert_eq!(fb.get(&a).unwrap(), vec![b'a'; 10]); // seed hit
        assert_eq!(fb.get(&fresh).unwrap(), b"only-in-store"); // seed NotFound → store
        let st = seed.stats();
        assert_eq!((st.hits, st.misses), (1, 1));
        // Seed tree untouched; nothing new created under the seed root.
        assert_eq!(fs::read(seed_root.join("d/f.bin")).unwrap(), before);
        let names: Vec<_> = fs::read_dir(&seed_root)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("d")]);
    }
}
