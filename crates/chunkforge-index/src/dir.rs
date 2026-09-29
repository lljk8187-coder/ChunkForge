//! `.cfdir` v1 directory archive encode / decode.
//!
//! Parallel to [`.cfidx`](crate::Index); does **not** alter `.cfidx` v1 bytes.
//! See `docs/dir-format.md` and Phase5 §3.1 / §4.2.

use crate::path::validate_archive_path;
use crate::{ENTRY_SIZE, Error, IndexEntry, TRAILER_SIZE};
use chunkforge_chunk::ChunkId;
use std::collections::HashSet;
use std::io::Write;

/// Full 8-byte magic for `.cfdir` v1: `b"CFDIR\0\0\x01"` (last byte = major).
pub const DIR_MAGIC_V1: [u8; 8] = *b"CFDIR\0\0\x01";
/// First 7 bytes of magic (identity); byte 7 is the major version.
pub const DIR_MAGIC_PREFIX: [u8; 7] = *b"CFDIR\0\0";
/// `.cfdir` v1 major version embedded in magic[7].
pub const DIR_MAJOR_V1: u8 = 1;
/// `.cfdir` v1 `format_version_u16`.
pub const DIR_FORMAT_VERSION_V1: u16 = 1;
/// Fixed header size (through `entry_count`).
pub const DIR_HEADER_SIZE: usize = 24;

/// Kind tag: regular file.
pub const KIND_FILE: u8 = 1;
/// Kind tag: explicit directory.
pub const KIND_DIR: u8 = 2;

/// Kind of a directory-archive entry.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum DirEntryKind {
    /// Regular file with content addressed by chunk table.
    File {
        mode: u32,
        size: u64,
        mtime_secs: u64,
        blob_blake3: ChunkId,
        /// Chunk table relative to this file (same 40-byte entry shape as `.cfidx`).
        chunks: Vec<IndexEntry>,
    },
    /// Explicit directory (empty dirs); mode only.
    Dir { mode: u32 },
}

/// One path → metadata entry inside a [`DirArchive`].
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DirEntry {
    /// Relative, `/`-separated, validated path.
    pub path: String,
    pub kind: DirEntryKind,
}

/// Directory archive listing (`.cfdir` v1). Parallel to single-blob [`crate::Index`].
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DirArchive {
    /// Always `1` for v1 writers/readers of this build.
    pub format_version: u16,
    pub flags: u16,
    pub entries: Vec<DirEntry>,
}

impl DirArchive {
    /// Construct and validate a structurally legal archive.
    pub fn new(flags: u16, entries: Vec<DirEntry>) -> Result<Self, Error> {
        let arch = Self {
            format_version: DIR_FORMAT_VERSION_V1,
            flags,
            entries,
        };
        arch.validate()?;
        Ok(arch)
    }

    /// Empty archive (no entries).
    pub fn empty() -> Self {
        Self {
            format_version: DIR_FORMAT_VERSION_V1,
            flags: 0,
            entries: Vec::new(),
        }
    }

    /// Number of directory entries.
    pub fn entry_count(&self) -> u64 {
        self.entries.len() as u64
    }

    /// Iterate all chunk ids referenced by file entries (order = archive order).
    pub fn all_chunk_ids(&self) -> impl Iterator<Item = ChunkId> + '_ {
        self.entries.iter().flat_map(|e| {
            match &e.kind {
                DirEntryKind::File { chunks, .. } => chunks.as_slice(),
                DirEntryKind::Dir { .. } => &[],
            }
            .iter()
            .map(|c| c.chunk_id)
        })
    }

    /// Validate paths, uniqueness, and per-file chunk tables.
    pub fn validate(&self) -> Result<(), Error> {
        if self.format_version != DIR_FORMAT_VERSION_V1 {
            return Err(Error::UnsupportedFormatVersion {
                found: self.format_version,
                supported: DIR_FORMAT_VERSION_V1,
            });
        }
        let mut seen = HashSet::with_capacity(self.entries.len());
        for e in &self.entries {
            validate_archive_path(&e.path)?;
            if !seen.insert(e.path.clone()) {
                return Err(Error::InvalidStructure(format!(
                    "duplicate path in .cfdir: {:?}",
                    e.path
                )));
            }
            match &e.kind {
                DirEntryKind::File { size, chunks, .. } => validate_file_chunks(*size, chunks)?,
                DirEntryKind::Dir { .. } => {}
            }
        }
        Ok(())
    }

    /// Encode to `.cfdir` v1 bytes (header || body || trailer).
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        self.validate()?;

        let n = self.entries.len();
        let mut buf = Vec::with_capacity(DIR_HEADER_SIZE + n * 64 + TRAILER_SIZE);

        buf.extend_from_slice(&DIR_MAGIC_V1);
        buf.extend_from_slice(&DIR_FORMAT_VERSION_V1.to_le_bytes());
        buf.extend_from_slice(&self.flags.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // reserved
        buf.extend_from_slice(&(n as u64).to_le_bytes());
        debug_assert_eq!(buf.len(), DIR_HEADER_SIZE);

        for e in &self.entries {
            let path_bytes = e.path.as_bytes();
            if path_bytes.len() > u16::MAX as usize {
                return Err(Error::InvalidPath(format!(
                    "path longer than u16::MAX: {} bytes",
                    path_bytes.len()
                )));
            }
            buf.extend_from_slice(&(path_bytes.len() as u16).to_le_bytes());
            buf.extend_from_slice(path_bytes);

            match &e.kind {
                DirEntryKind::File {
                    mode,
                    size,
                    mtime_secs,
                    blob_blake3,
                    chunks,
                } => {
                    buf.push(KIND_FILE);
                    buf.extend_from_slice(&mode.to_le_bytes());
                    buf.extend_from_slice(&size.to_le_bytes());
                    buf.extend_from_slice(&mtime_secs.to_le_bytes());
                    buf.extend_from_slice(blob_blake3.as_bytes());
                    buf.extend_from_slice(&(chunks.len() as u64).to_le_bytes());
                    for c in chunks {
                        buf.extend_from_slice(&c.end_offset.to_le_bytes());
                        buf.extend_from_slice(c.chunk_id.as_bytes());
                    }
                }
                DirEntryKind::Dir { mode } => {
                    buf.push(KIND_DIR);
                    buf.extend_from_slice(&mode.to_le_bytes());
                }
            }
        }

        let digest = blake3::hash(&buf);
        buf.extend_from_slice(digest.as_bytes());
        Ok(buf)
    }

    /// Decode `.cfdir` v1 bytes. Hard-fails on wrong major, truncate, or bad trailer.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < DIR_HEADER_SIZE + TRAILER_SIZE {
            return Err(Error::Truncated(format!(
                "need at least {} bytes for .cfdir header+trailer, got {}",
                DIR_HEADER_SIZE + TRAILER_SIZE,
                bytes.len()
            )));
        }

        let magic = &bytes[0..8];
        if magic[..7] != DIR_MAGIC_PREFIX {
            return Err(Error::BadMagic);
        }
        let major = magic[7];
        if major != DIR_MAJOR_V1 {
            return Err(Error::UnsupportedMajor {
                found: major,
                expected: DIR_MAJOR_V1,
            });
        }

        let format_version = u16::from_le_bytes(bytes[8..10].try_into().unwrap());
        if format_version != DIR_FORMAT_VERSION_V1 {
            return Err(Error::UnsupportedFormatVersion {
                found: format_version,
                supported: DIR_FORMAT_VERSION_V1,
            });
        }

        let flags = u16::from_le_bytes(bytes[10..12].try_into().unwrap());
        let _reserved = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let entry_count = u64::from_le_bytes(bytes[16..24].try_into().unwrap());

        if entry_count > (usize::MAX / 2) as u64 {
            return Err(Error::Truncated(format!(
                "entry_count {entry_count} is unreasonably large"
            )));
        }
        let n = entry_count as usize;

        // Body ends where trailer begins; parse entries then verify length.
        if bytes.len() < DIR_HEADER_SIZE + TRAILER_SIZE {
            return Err(Error::Truncated("truncated .cfdir".into()));
        }
        let body_and_trailer = &bytes[DIR_HEADER_SIZE..];
        if body_and_trailer.len() < TRAILER_SIZE {
            return Err(Error::Truncated("missing .cfdir trailer".into()));
        }

        let mut off = DIR_HEADER_SIZE;
        let mut entries = Vec::with_capacity(n);
        for i in 0..n {
            let remaining_for_trailer = bytes.len().saturating_sub(off);
            if remaining_for_trailer < TRAILER_SIZE + 2 {
                return Err(Error::Truncated(format!(
                    "truncated while reading entry[{i}] path_len"
                )));
            }
            let path_len = u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap()) as usize;
            off += 2;
            if bytes.len() < off + path_len + 1 + TRAILER_SIZE {
                return Err(Error::Truncated(format!(
                    "truncated while reading entry[{i}] path ({path_len} bytes)"
                )));
            }
            let path_bytes = &bytes[off..off + path_len];
            let path = std::str::from_utf8(path_bytes)
                .map_err(|_| Error::InvalidPath(format!("entry[{i}] path is not UTF-8")))?
                .to_owned();
            off += path_len;

            let kind_tag = bytes[off];
            off += 1;

            let kind = match kind_tag {
                KIND_FILE => {
                    // mode(4) + size(8) + mtime(8) + blake3(32) + chunk_count(8) = 60
                    if bytes.len() < off + 60 + TRAILER_SIZE {
                        return Err(Error::Truncated(format!(
                            "truncated while reading entry[{i}] file header"
                        )));
                    }
                    let mode = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
                    off += 4;
                    let size = u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap());
                    off += 8;
                    let mtime_secs = u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap());
                    off += 8;
                    let blob_blake3 = ChunkId::from_bytes(bytes[off..off + 32].try_into().unwrap());
                    off += 32;
                    let chunk_count = u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap());
                    off += 8;
                    if chunk_count > (usize::MAX / ENTRY_SIZE) as u64 {
                        return Err(Error::Truncated(format!(
                            "entry[{i}] chunk_count {chunk_count} unreasonably large"
                        )));
                    }
                    let cn = chunk_count as usize;
                    let need = cn
                        .checked_mul(ENTRY_SIZE)
                        .ok_or_else(|| Error::Truncated("chunk table size overflow".into()))?;
                    if bytes.len() < off + need + TRAILER_SIZE {
                        return Err(Error::Truncated(format!(
                            "truncated while reading entry[{i}] chunk table"
                        )));
                    }
                    let mut chunks = Vec::with_capacity(cn);
                    for _ in 0..cn {
                        let end_offset =
                            u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap());
                        let chunk_id =
                            ChunkId::from_bytes(bytes[off + 8..off + 40].try_into().unwrap());
                        chunks.push(IndexEntry {
                            end_offset,
                            chunk_id,
                        });
                        off += ENTRY_SIZE;
                    }
                    DirEntryKind::File {
                        mode,
                        size,
                        mtime_secs,
                        blob_blake3,
                        chunks,
                    }
                }
                KIND_DIR => {
                    if bytes.len() < off + 4 + TRAILER_SIZE {
                        return Err(Error::Truncated(format!(
                            "truncated while reading entry[{i}] dir mode"
                        )));
                    }
                    let mode = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
                    off += 4;
                    DirEntryKind::Dir { mode }
                }
                other => {
                    return Err(Error::InvalidStructure(format!(
                        "entry[{i}] unknown kind tag {other}"
                    )));
                }
            };
            entries.push(DirEntry { path, kind });
        }

        let body_end = off;
        if bytes.len() != body_end + TRAILER_SIZE {
            return Err(Error::Truncated(format!(
                "expected {} bytes after parsing {n} entries (body_end={body_end}), got {}",
                body_end + TRAILER_SIZE,
                bytes.len()
            )));
        }

        let body = &bytes[..body_end];
        let trailer = &bytes[body_end..body_end + TRAILER_SIZE];
        let computed = blake3::hash(body);
        if computed.as_bytes() != trailer {
            return Err(Error::TrailerMismatch);
        }

        Self::new(flags, entries)
    }

    /// Write encoded bytes to `w`.
    pub fn write_to<W: Write>(&self, mut w: W) -> Result<(), Error> {
        let bytes = self.encode()?;
        w.write_all(&bytes)?;
        Ok(())
    }
}

fn validate_file_chunks(size: u64, chunks: &[IndexEntry]) -> Result<(), Error> {
    if chunks.is_empty() {
        if size != 0 {
            return Err(Error::InvalidStructure(
                "file chunk_count == 0 requires size == 0".into(),
            ));
        }
        return Ok(());
    }
    if size == 0 {
        return Err(Error::InvalidStructure(
            "file size == 0 requires chunk_count == 0".into(),
        ));
    }
    let mut prev = 0u64;
    for (i, e) in chunks.iter().enumerate() {
        if e.end_offset <= prev {
            return Err(Error::InvalidStructure(format!(
                "file chunk[{i}] end_offset {} is not strictly greater than previous {}",
                e.end_offset, prev
            )));
        }
        prev = e.end_offset;
    }
    let last = chunks.last().unwrap().end_offset;
    if last != size {
        return Err(Error::InvalidStructure(format!(
            "file last end_offset ({last}) must equal size ({size})"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chunkforge_chunk::ChunkId;

    fn sample_archive() -> DirArchive {
        let id0 = ChunkId::hash(b"chunk-a");
        let id1 = ChunkId::hash(b"chunk-b");
        let blob = ChunkId::hash(b"file-contents");
        DirArchive::new(
            0,
            vec![
                DirEntry {
                    path: "readme.txt".into(),
                    kind: DirEntryKind::File {
                        mode: 0o644,
                        size: 100,
                        mtime_secs: 1_700_000_000,
                        blob_blake3: blob,
                        chunks: vec![
                            IndexEntry {
                                end_offset: 40,
                                chunk_id: id0,
                            },
                            IndexEntry {
                                end_offset: 100,
                                chunk_id: id1,
                            },
                        ],
                    },
                },
                DirEntry {
                    path: "sub/empty".into(),
                    kind: DirEntryKind::Dir { mode: 0o755 },
                },
                DirEntry {
                    path: "sub/data.bin".into(),
                    kind: DirEntryKind::File {
                        mode: 0o600,
                        size: 0,
                        mtime_secs: 0,
                        blob_blake3: ChunkId::hash(b""),
                        chunks: vec![],
                    },
                },
            ],
        )
        .unwrap()
    }

    #[test]
    fn encode_decode_roundtrip_equal() {
        let arch = sample_archive();
        let bytes = arch.encode().unwrap();
        let decoded = DirArchive::decode(&bytes).unwrap();
        assert_eq!(arch, decoded);
        assert_eq!(bytes, decoded.encode().unwrap());
    }

    #[test]
    fn empty_archive_roundtrip() {
        let arch = DirArchive::empty();
        let bytes = arch.encode().unwrap();
        assert_eq!(bytes.len(), DIR_HEADER_SIZE + TRAILER_SIZE);
        assert_eq!(&bytes[0..8], &DIR_MAGIC_V1);
        let decoded = DirArchive::decode(&bytes).unwrap();
        assert_eq!(arch, decoded);
    }

    #[test]
    fn header_layout_magic_and_counts() {
        let arch = sample_archive();
        let bytes = arch.encode().unwrap();
        assert_eq!(&bytes[0..8], b"CFDIR\0\0\x01");
        assert_eq!(u16::from_le_bytes(bytes[8..10].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(bytes[10..12].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 0);
        assert_eq!(u64::from_le_bytes(bytes[16..24].try_into().unwrap()), 3);
    }

    #[test]
    fn path_containing_dotdot_rejected() {
        let err = DirArchive::new(
            0,
            vec![DirEntry {
                path: "a/../b".into(),
                kind: DirEntryKind::Dir { mode: 0o755 },
            }],
        )
        .unwrap_err();
        assert!(matches!(err, Error::InvalidPath(_)), "{err:?}");
        assert!(err.to_string().contains(".."), "{err}");

        // Also: encode path of an otherwise-built archive cannot sneak .. through
        // if we bypass new() — validate on encode.
        let mut arch = DirArchive::empty();
        arch.entries.push(DirEntry {
            path: "../escape".into(),
            kind: DirEntryKind::File {
                mode: 0o644,
                size: 0,
                mtime_secs: 0,
                blob_blake3: ChunkId::hash(b""),
                chunks: vec![],
            },
        });
        let err = arch.encode().unwrap_err();
        assert!(matches!(err, Error::InvalidPath(_)), "{err:?}");
    }

    #[test]
    fn all_chunk_ids_collects_files_only() {
        let arch = sample_archive();
        let ids: Vec<_> = arch.all_chunk_ids().collect();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], ChunkId::hash(b"chunk-a"));
        assert_eq!(ids[1], ChunkId::hash(b"chunk-b"));
    }

    #[test]
    fn bad_magic_and_major() {
        let mut bytes = DirArchive::empty().encode().unwrap();
        bytes[0] = b'X';
        assert!(matches!(
            DirArchive::decode(&bytes).unwrap_err(),
            Error::BadMagic
        ));

        let mut bytes = DirArchive::empty().encode().unwrap();
        bytes[7] = 9;
        let err = DirArchive::decode(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnsupportedMajor {
                found: 9,
                expected: 1
            }
        ));
        let msg = err.to_string();
        assert!(
            msg.contains("upgrade") && msg.contains("chunkforge"),
            "{msg}"
        );
    }

    #[test]
    fn bad_trailer() {
        let mut bytes = sample_archive().encode().unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        assert!(matches!(
            DirArchive::decode(&bytes).unwrap_err(),
            Error::TrailerMismatch
        ));
    }

    #[test]
    fn truncated() {
        let bytes = sample_archive().encode().unwrap();
        let err = DirArchive::decode(&bytes[..bytes.len() - 1]).unwrap_err();
        assert!(
            matches!(err, Error::Truncated(_) | Error::TrailerMismatch),
            "{err:?}"
        );
    }

    #[test]
    fn duplicate_path_rejected() {
        let err = DirArchive::new(
            0,
            vec![
                DirEntry {
                    path: "same".into(),
                    kind: DirEntryKind::Dir { mode: 0o755 },
                },
                DirEntry {
                    path: "same".into(),
                    kind: DirEntryKind::Dir { mode: 0o755 },
                },
            ],
        )
        .unwrap_err();
        assert!(matches!(err, Error::InvalidStructure(_)), "{err:?}");
    }

    #[test]
    fn file_chunk_size_mismatch_rejected() {
        let err = DirArchive::new(
            0,
            vec![DirEntry {
                path: "x".into(),
                kind: DirEntryKind::File {
                    mode: 0o644,
                    size: 100,
                    mtime_secs: 0,
                    blob_blake3: ChunkId::hash(b"x"),
                    chunks: vec![IndexEntry {
                        end_offset: 50,
                        chunk_id: ChunkId::hash(b"a"),
                    }],
                },
            }],
        )
        .unwrap_err();
        assert!(matches!(err, Error::InvalidStructure(_)), "{err:?}");
    }

    #[test]
    fn cfidx_bytes_unchanged_by_cfdir_presence() {
        // Sanity: Index encode still produces CFIDX magic (regression guard).
        use crate::Index;
        use chunkforge_chunk::ChunkParams;
        let idx = Index::empty(ChunkParams::default());
        let bytes = idx.encode().unwrap();
        assert_eq!(&bytes[0..8], b"CFIDX\0\0\x01");
    }
}
