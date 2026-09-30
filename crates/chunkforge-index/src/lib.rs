//! ChunkForge index / archive encode / decode.
//!
//! - [`.cfidx` v1](Index) — single-blob chunk map (Phase 1; **byte-frozen**)
//! - [`.cfdir` v1](DirArchive) — multi-file directory listing (Phase 5)
//! - [seed helpers](seed_file_map) — prior path index + content-blake3 reuse (+ optional mtime trust, Phase 7)
//! - [diff helpers](diff_dir_archives) — path/chunk set comparison of two `.cfdir` (Phase 7)
//! - [extract match](judge_extract_unchanged) — dest vs listing size+blake3 for `--skip-unchanged` (Phase 9); optional mtime trust (Phase 11)
//! - [`PathFilter`] — `--path` / `--exclude` matching for archive paths (Phase 13; library only, CLI unwired)
//!
//! Binary layouts are little-endian. See `docs/index-format.md` and
//! `docs/dir-format.md`. Seed / diff / extract-match / path-filter helpers do **not** change those layouts.
//!
//! This crate does **not** implement CLI archive/extract, FUSE, or store I/O.

mod diff;
mod dir;
mod error;
mod extract_match;
mod index;
mod path;
mod path_filter;
mod seed;

pub use diff::{DiffReport, diff_dir_archives};
pub use dir::{
    DIR_FORMAT_VERSION_V1, DIR_HEADER_SIZE, DIR_MAGIC_PREFIX, DIR_MAGIC_V1, DIR_MAJOR_V1,
    DirArchive, DirEntry, DirEntryKind, KIND_DIR, KIND_FILE,
};
pub use error::{Error, IndexError};
pub use extract_match::{UnchangedVerdict, judge_extract_unchanged, judge_extract_unchanged_opts};
pub use index::{
    ENTRY_SIZE, FLAG_CHUNKS_COMPRESSED_IN_STORE, FORMAT_VERSION_V1, HEADER_SIZE, Index, IndexEntry,
    MAGIC_PREFIX, MAGIC_V1, MAJOR_V1, TRAILER_SIZE, entry_length,
};
pub use path::validate_archive_path;
pub use path_filter::{ExcludePat, PathFilter};
pub use seed::{
    SeedDecision, decide_seed, decide_seed_for_entry, decide_seed_for_entry_ex,
    decide_seed_trust_mtime, hash_reader, seed_file_map,
};

#[cfg(test)]
mod tests {
    use super::*;
    use chunkforge_chunk::{ChunkId, ChunkParams};

    fn sample_index() -> Index {
        let params = ChunkParams::default();
        let id0 = ChunkId::hash(b"chunk-zero");
        let id1 = ChunkId::hash(b"chunk-one");
        let blob = ChunkId::hash(b"whole-blob-placeholder");
        Index::new(
            0,
            params,
            100,
            blob,
            vec![
                IndexEntry {
                    end_offset: 40,
                    chunk_id: id0,
                },
                IndexEntry {
                    end_offset: 100,
                    chunk_id: id1,
                },
            ],
        )
        .unwrap()
    }

    #[test]
    fn encode_decode_roundtrip_equal() {
        let idx = sample_index();
        let bytes = idx.encode().unwrap();
        let decoded = Index::decode(&bytes).unwrap();
        assert_eq!(idx, decoded);
        // Bitwise: re-encode must match.
        assert_eq!(bytes, decoded.encode().unwrap());
    }

    #[test]
    fn empty_file_roundtrip() {
        let idx = Index::empty(ChunkParams::default());
        assert_eq!(idx.chunk_count(), 0);
        assert_eq!(idx.total_size, 0);
        assert_eq!(idx.blob_blake3, ChunkId::hash(b""));

        let bytes = idx.encode().unwrap();
        assert_eq!(bytes.len(), HEADER_SIZE + TRAILER_SIZE);
        assert_eq!(&bytes[0..8], &MAGIC_V1);

        let decoded = Index::decode(&bytes).unwrap();
        assert_eq!(idx, decoded);
        assert_eq!(bytes, decoded.encode().unwrap());
    }

    #[test]
    fn header_layout_offsets() {
        let idx = sample_index();
        let bytes = idx.encode().unwrap();
        assert_eq!(&bytes[0..8], b"CFIDX\0\0\x01");
        assert_eq!(u16::from_le_bytes(bytes[8..10].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(bytes[10..12].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 0);
        assert_eq!(
            u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            16 * 1024
        );
        assert_eq!(
            u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
            64 * 1024
        );
        assert_eq!(
            u64::from_le_bytes(bytes[32..40].try_into().unwrap()),
            256 * 1024
        );
        assert_eq!(u64::from_le_bytes(bytes[40..48].try_into().unwrap()), 100);
        assert_eq!(u64::from_le_bytes(bytes[48..56].try_into().unwrap()), 2);
        assert_eq!(&bytes[56..88], idx.blob_blake3.as_bytes());
        // First entry
        assert_eq!(u64::from_le_bytes(bytes[88..96].try_into().unwrap()), 40);
        assert_eq!(&bytes[96..128], idx.entries[0].chunk_id.as_bytes());
        assert_eq!(bytes.len(), HEADER_SIZE + 2 * ENTRY_SIZE + TRAILER_SIZE);
    }

    #[test]
    fn bad_major_hard_fails_with_upgrade_message() {
        let mut bytes = Index::empty(ChunkParams::default()).encode().unwrap();
        bytes[7] = 2; // major=2
        // Trailer will also be wrong, but major must be checked first.
        let err = Index::decode(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnsupportedMajor {
                found: 2,
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
    fn bad_magic_prefix() {
        let mut bytes = Index::empty(ChunkParams::default()).encode().unwrap();
        bytes[0] = b'X';
        let err = Index::decode(&bytes).unwrap_err();
        assert!(matches!(err, Error::BadMagic));
    }

    #[test]
    fn truncated_too_short() {
        let bytes = Index::empty(ChunkParams::default()).encode().unwrap();
        let err = Index::decode(&bytes[..bytes.len() - 1]).unwrap_err();
        assert!(matches!(err, Error::Truncated(_)), "{err:?}");
    }

    #[test]
    fn truncated_missing_entries() {
        let idx = sample_index();
        let bytes = idx.encode().unwrap();
        // Claim 2 chunks but cut body mid-entry: rebuild header with count=2 and short payload.
        let short = &bytes[..HEADER_SIZE + 10]; // incomplete
        let err = Index::decode(short).unwrap_err();
        assert!(matches!(err, Error::Truncated(_)), "{err:?}");
    }

    #[test]
    fn bad_trailer() {
        let mut bytes = sample_index().encode().unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        let err = Index::decode(&bytes).unwrap_err();
        assert!(matches!(err, Error::TrailerMismatch));
    }

    #[test]
    fn flip_header_byte_fails_trailer() {
        let mut bytes = sample_index().encode().unwrap();
        bytes[40] ^= 0x01; // total_size low byte
        let err = Index::decode(&bytes).unwrap_err();
        // Length still matches chunk_count, so we get trailer mismatch (or structure
        // error if parsing still succeeds checksum — checksum covers header so trailer).
        assert!(matches!(err, Error::TrailerMismatch), "{err:?}");
    }

    #[test]
    fn reject_empty_with_nonzero_total() {
        let err =
            Index::new(0, ChunkParams::default(), 10, ChunkId::hash(b""), vec![]).unwrap_err();
        assert!(matches!(err, Error::InvalidStructure(_)));
    }

    #[test]
    fn reject_last_end_offset_mismatch() {
        let err = Index::new(
            0,
            ChunkParams::default(),
            100,
            ChunkId::hash(b"x"),
            vec![IndexEntry {
                end_offset: 50,
                chunk_id: ChunkId::hash(b"a"),
            }],
        )
        .unwrap_err();
        assert!(matches!(err, Error::InvalidStructure(_)));
        let msg = err.to_string();
        assert!(
            msg.contains("total_size") || msg.contains("end_offset"),
            "{msg}"
        );
    }

    #[test]
    fn entry_length_helper() {
        let idx = sample_index();
        assert_eq!(entry_length(&idx.entries, 0), Some(40));
        assert_eq!(entry_length(&idx.entries, 1), Some(60));
        assert_eq!(entry_length(&idx.entries, 2), None);
    }

    #[test]
    fn flags_compressed_bit_roundtrips() {
        let mut idx = sample_index();
        idx.flags = FLAG_CHUNKS_COMPRESSED_IN_STORE;
        let decoded = Index::decode(&idx.encode().unwrap()).unwrap();
        assert_eq!(decoded.flags, FLAG_CHUNKS_COMPRESSED_IN_STORE);
    }

    #[test]
    fn unsupported_format_version() {
        let mut bytes = Index::empty(ChunkParams::default()).encode().unwrap();
        // format_version at offset 8; set to 99, then fix trailer so we pass checksum
        // only if we recompute — actually decode checks format_version before trailer
        // after magic. Order in decode: magic → format_version → … → length → trailer.
        // So bumping format_version without fixing trailer still hits UnsupportedFormatVersion
        // first (before trailer check). Confirm that.
        bytes[8] = 99;
        bytes[9] = 0;
        let err = Index::decode(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnsupportedFormatVersion {
                found: 99,
                supported: 1
            }
        ));
    }
}
