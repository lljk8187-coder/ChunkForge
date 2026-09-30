//! Local content-addressed store: create/open, put/get/has, atomic writes.

use crate::Error;
use crate::meta::{Compression, StoreMeta};
use crate::outcome::PutOutcome;
use crate::path::chunk_abs_path;
use chunkforge_chunk::ChunkId;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Per-process counter so concurrent `put` calls never share a tmp path.
static PUT_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// A local ChunkForge CAS store rooted at a directory.
///
/// Layout:
/// ```text
/// <store_root>/
///   meta.toml
///   chunks/
///     ab/
///       cdef...rest.cnk
/// ```
#[derive(Debug)]
pub struct Store {
    root: PathBuf,
    meta: StoreMeta,
}

impl Store {
    /// Create a new store at `root` with the given compression policy.
    ///
    /// Fails if `root` already contains a `meta.toml` (use [`open`](Self::open)).
    pub fn create(root: impl AsRef<Path>, compression: Compression) -> Result<Self, Error> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("chunks"))?;
        let meta_path = root.join("meta.toml");
        if meta_path.exists() {
            return Err(Error::InvalidMeta(format!(
                "store already exists at {}",
                root.display()
            )));
        }
        let meta = StoreMeta::new(compression);
        meta.write_to(&meta_path)?;
        // Best-effort durability for the metadata file.
        if let Ok(f) = File::open(&meta_path) {
            let _ = f.sync_all();
        }
        Ok(Self { root, meta })
    }

    /// Open an existing store at `root` (must contain a valid `meta.toml`).
    pub fn open(root: impl AsRef<Path>) -> Result<Self, Error> {
        let root = root.as_ref().to_path_buf();
        let meta = StoreMeta::read_from(&root.join("meta.toml"))?;
        Ok(Self { root, meta })
    }

    /// Store root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Uniform compression policy for this store.
    pub fn compression(&self) -> Compression {
        self.meta.compression
    }

    /// Absolute path of the `.cnk` file for `id` (may or may not exist).
    pub fn chunk_path(&self, id: &ChunkId) -> PathBuf {
        chunk_abs_path(&self.root, id)
    }

    /// Whether a chunk file for `id` exists on disk.
    pub fn has(&self, id: &ChunkId) -> bool {
        self.chunk_path(id).is_file()
    }

    /// Hash `plain`, then store it (deduplicating if already present).
    ///
    /// Returns the content id (`blake3(plain)`) and whether the chunk was newly
    /// inserted or already present. Hash is always over plaintext.
    pub fn put(&self, plain: &[u8]) -> Result<(ChunkId, PutOutcome), Error> {
        let id = ChunkId::hash(plain);
        let outcome = self.put_with_id(&id, plain)?;
        Ok((id, outcome))
    }

    /// Store `plain` under the given `id`.
    ///
    /// The id must equal `blake3(plain)`; otherwise returns [`Error::IdMismatch`].
    /// If the chunk already exists (`has`), the write is skipped (dedup) and
    /// [`PutOutcome::SkippedExists`] is returned.
    pub fn put_with_id(&self, id: &ChunkId, plain: &[u8]) -> Result<PutOutcome, Error> {
        let actual = ChunkId::hash(plain);
        if actual != *id {
            return Err(Error::IdMismatch {
                expected: *id,
                actual,
            });
        }
        if self.has(id) {
            return Ok(PutOutcome::SkippedExists);
        }

        let final_path = self.chunk_path(id);
        let parent = final_path
            .parent()
            .expect("chunk path always has a parent directory");
        fs::create_dir_all(parent)?;

        let payload = encode_payload(plain, self.meta.compression)?;

        // Unique tmp name in the same directory so rename stays atomic.
        // Include a per-process sequence so concurrent puts (archive --jobs)
        // never collide on the same *.tmp path within one process.
        let tmp_name = format!(
            "{}.{:x}.{:x}.tmp",
            final_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("chunk"),
            std::process::id(),
            PUT_TMP_SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let tmp_path = parent.join(tmp_name);

        let write_result = (|| -> Result<(), Error> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp_path)?;
            file.write_all(&payload)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&tmp_path, &final_path)?;
            // Best-effort directory fsync for durability of the rename.
            if let Ok(dir) = File::open(parent) {
                let _ = dir.sync_all();
            }
            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }

        // Another writer may have won the race; treat existing final as reuse.
        match write_result {
            Ok(()) => Ok(PutOutcome::Written),
            Err(e) if self.has(id) => {
                let _ = e; // discarded: chunk is present
                Ok(PutOutcome::SkippedExists)
            }
            Err(e) => Err(e),
        }
    }

    /// Read and return plaintext bytes for `id`, verifying BLAKE3.
    ///
    /// Corrupted on-disk bytes (or a hash mismatch after decompression) fail
    /// with [`Error::Corrupt`]. Missing chunks fail with [`Error::NotFound`].
    pub fn get(&self, id: &ChunkId) -> Result<Vec<u8>, Error> {
        self.get_verify(id, true)
    }

    /// Like [`get`](Self::get), but optionally skip the BLAKE3 check.
    ///
    /// Default API path verifies; skipping is for hot-path experiments only.
    pub fn get_verify(&self, id: &ChunkId, verify: bool) -> Result<Vec<u8>, Error> {
        let path = self.chunk_path(id);
        let bytes = fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::NotFound(*id)
            } else {
                Error::Io(e)
            }
        })?;
        let plain = decode_payload(&bytes, self.meta.compression)?;
        if verify {
            let actual = ChunkId::hash(&plain);
            if actual != *id {
                return Err(Error::Corrupt(*id));
            }
        }
        Ok(plain)
    }

    /// List chunk ids present as well-formed loose `.cnk` files under `chunks/`.
    ///
    /// Only files matching the CAS layout (`chunks/<2hex>/<62hex>.cnk` with
    /// lowercase hex) are returned. Temporary / stray files are skipped.
    /// Order is unspecified.
    pub fn list_chunk_ids(&self) -> Result<Vec<ChunkId>, Error> {
        let chunks_root = self.root.join("chunks");
        if !chunks_root.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut stack = vec![chunks_root];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if let Some(id) = chunk_id_from_cnk_path(&self.root, &path) {
                    out.push(id);
                }
            }
        }
        Ok(out)
    }

    /// Delete the loose `.cnk` file for `id` if present.
    ///
    /// Returns [`Error::NotFound`] when the file does not exist. Intended for
    /// local `gc --apply`. Each id maps to an independent `.cnk` file, so
    /// concurrent removes across distinct ids are safe. Does not touch remote
    /// stores.
    pub fn remove(&self, id: &ChunkId) -> Result<(), Error> {
        let path = self.chunk_path(id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::NotFound(*id)),
            Err(e) => Err(Error::Io(e)),
        }
    }
}

/// Parse a chunk id from an on-disk `.cnk` path if it matches the CAS layout.
///
/// Expects `chunks/<2hex>/<62hex>.cnk` relative to `store_root`. Returns `None`
/// for non-conforming names (temps, wrong extension, bad hex, nested junk).
fn chunk_id_from_cnk_path(store_root: &Path, path: &Path) -> Option<ChunkId> {
    let rel = path.strip_prefix(store_root).ok()?;
    let mut comps = rel.components();
    use std::path::Component;
    let (chunks, shard, file) = match (comps.next(), comps.next(), comps.next(), comps.next()) {
        (
            Some(Component::Normal(c)),
            Some(Component::Normal(s)),
            Some(Component::Normal(f)),
            None,
        ) if c == "chunks" => (c, s, f),
        _ => return None,
    };
    let _ = chunks;
    let shard = shard.to_str()?;
    let file = file.to_str()?;
    if shard.len() != 2 || !is_lowercase_hex(shard) {
        return None;
    }
    let stem = file.strip_suffix(".cnk")?;
    if stem.len() != 62 || !is_lowercase_hex(stem) {
        return None;
    }
    let hex = format!("{shard}{stem}");
    ChunkId::from_hex(&hex).ok()
}

fn is_lowercase_hex(s: &str) -> bool {
    s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn encode_payload(plain: &[u8], compression: Compression) -> Result<Vec<u8>, Error> {
    match compression {
        Compression::None => Ok(plain.to_vec()),
        #[cfg(feature = "zstd")]
        Compression::Zstd => {
            zstd::encode_all(plain, 3).map_err(|e| Error::Io(std::io::Error::other(e)))
        }
    }
}

fn decode_payload(stored: &[u8], compression: Compression) -> Result<Vec<u8>, Error> {
    match compression {
        Compression::None => Ok(stored.to_vec()),
        #[cfg(feature = "zstd")]
        Compression::Zstd => {
            zstd::decode_all(stored).map_err(|e| Error::Io(std::io::Error::other(e)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::Compression;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn create_open_roundtrip_meta() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("s");
        let store = Store::create(&root, Compression::None).unwrap();
        assert_eq!(store.compression(), Compression::None);
        assert!(root.join("meta.toml").is_file());
        assert!(root.join("chunks").is_dir());

        let opened = Store::open(&root).unwrap();
        assert_eq!(opened.compression(), Compression::None);
        assert_eq!(opened.root(), root.as_path());
    }

    #[test]
    fn put_get_has_roundtrip() {
        let dir = tempdir().unwrap();
        let store = Store::create(dir.path(), Compression::None).unwrap();
        let data = b"hello chunkforge store";
        let (id, outcome) = store.put(data).unwrap();
        assert!(outcome.is_new());
        assert!(store.has(&id));
        assert_eq!(store.get(&id).unwrap(), data);
        assert_eq!(id, ChunkId::hash(data));
    }

    #[test]
    fn double_put_one_file() {
        let dir = tempdir().unwrap();
        let store = Store::create(dir.path(), Compression::None).unwrap();
        let data = b"dedup-me-please";
        let (id1, o1) = store.put(data).unwrap();
        let (id2, o2) = store.put(data).unwrap();
        assert_eq!(id1, id2);
        assert!(o1.is_new());
        assert!(o2.is_reused());

        let path = store.chunk_path(&id1);
        assert!(path.is_file());

        // Exactly one `.cnk` under chunks/.
        let mut cnk_count = 0usize;
        for entry in walkdir_cnk(dir.path().join("chunks")) {
            cnk_count += 1;
            assert_eq!(entry, path);
        }
        assert_eq!(cnk_count, 1);
    }

    #[test]
    fn get_verifies_blake3_corrupt_fails() {
        let dir = tempdir().unwrap();
        let store = Store::create(dir.path(), Compression::None).unwrap();
        let data = b"integrity-check";
        let (id, _) = store.put(data).unwrap();
        let path = store.chunk_path(&id);

        // Flip one byte on disk.
        let mut bytes = fs::read(&path).unwrap();
        bytes[0] ^= 0xff;
        fs::write(&path, &bytes).unwrap();

        let err = store.get(&id).unwrap_err();
        assert!(matches!(err, Error::Corrupt(c) if c == id), "{err:?}");
    }

    #[test]
    fn has_false_for_missing() {
        let dir = tempdir().unwrap();
        let store = Store::create(dir.path(), Compression::None).unwrap();
        let id = ChunkId::hash(b"never-written");
        assert!(!store.has(&id));
        assert!(matches!(store.get(&id), Err(Error::NotFound(_))));
    }

    #[test]
    fn put_with_id_rejects_mismatch() {
        let dir = tempdir().unwrap();
        let store = Store::create(dir.path(), Compression::None).unwrap();
        let wrong = ChunkId::hash(b"a");
        let err = store.put_with_id(&wrong, b"b").unwrap_err();
        assert!(matches!(err, Error::IdMismatch { .. }));
    }

    #[test]
    fn path_layout_uses_hex_prefix() {
        let dir = tempdir().unwrap();
        let store = Store::create(dir.path(), Compression::None).unwrap();
        let (id, _) = store.put(b"path-layout").unwrap();
        let hex = id.to_hex();
        let path = store.chunk_path(&id);
        let expected = dir
            .path()
            .join("chunks")
            .join(&hex[0..2])
            .join(format!("{}.cnk", &hex[2..]));
        assert_eq!(path, expected);
        assert!(path.is_file());
    }

    #[test]
    fn empty_chunk_roundtrip() {
        let dir = tempdir().unwrap();
        let store = Store::create(dir.path(), Compression::None).unwrap();
        let (id, _) = store.put(b"").unwrap();
        assert_eq!(
            id.to_hex(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert_eq!(store.get(&id).unwrap(), b"");
    }

    #[test]
    fn list_chunk_ids_finds_put_chunks_skips_junk() {
        let dir = tempdir().unwrap();
        let store = Store::create(dir.path(), Compression::None).unwrap();
        let (id_a, _) = store.put(b"list-a").unwrap();
        let (id_b, _) = store.put(b"list-b").unwrap();

        // Junk that must not appear: tmp sibling, wrong-length name, nested dir file.
        let a_path = store.chunk_path(&id_a);
        let parent = a_path.parent().unwrap();
        fs::write(parent.join("not-a-chunk.tmp"), b"x").unwrap();
        fs::write(parent.join("abcd.cnk"), b"x").unwrap(); // stem != 62 hex
        fs::create_dir_all(parent.join("nested")).unwrap();
        fs::write(
            parent
                .join("nested")
                .join(format!("{}.cnk", "f".repeat(62))),
            b"x",
        )
        .unwrap();

        let mut ids = store.list_chunk_ids().unwrap();
        ids.sort();
        let mut expect = vec![id_a, id_b];
        expect.sort();
        assert_eq!(ids, expect);
    }

    #[test]
    fn remove_deletes_chunk_file() {
        let dir = tempdir().unwrap();
        let store = Store::create(dir.path(), Compression::None).unwrap();
        let (id, _) = store.put(b"remove-me").unwrap();
        assert!(store.has(&id));
        store.remove(&id).unwrap();
        assert!(!store.has(&id));
        assert!(matches!(store.remove(&id), Err(Error::NotFound(_))));
    }

    fn walkdir_cnk(root: PathBuf) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().and_then(|e| e.to_str()) == Some("cnk") {
                    out.push(path);
                }
            }
        }
        out
    }
}

#[cfg(all(test, feature = "zstd"))]
mod zstd_tests {
    use super::*;
    use crate::meta::Compression;
    use tempfile::tempdir;

    #[test]
    fn zstd_put_get_roundtrip_and_dedup() {
        let dir = tempdir().unwrap();
        let store = Store::create(dir.path(), Compression::Zstd).unwrap();
        let data = b"zzzzzzzzzzzzzzzz compressible payload zzzzzzzzzzzz";
        let (id, o1) = store.put(data).unwrap();
        assert!(o1.is_new());
        let (id2, o2) = store.put(data).unwrap();
        assert_eq!(id2, id);
        assert!(o2.is_reused());
        assert_eq!(store.get(&id).unwrap(), data);
        // On-disk bytes should differ from plaintext when compressed.
        let on_disk = std::fs::read(store.chunk_path(&id)).unwrap();
        assert_ne!(on_disk, data);
        assert_eq!(ChunkId::hash(data), id);
    }
}
