//! Content-defined chunking (FastCDC v2020) and BLAKE3 content addressing.
//!
//! This crate is the data-plane foundation for ChunkForge Phase 1: given a
//! byte stream, it yields deterministic cut points and a [`ChunkId`] (BLAKE3
//! of the **plaintext** chunk bytes) for each chunk. It does **not** touch a
//! filesystem store or index format.

mod chunk_id;
mod error;
mod params;

pub use chunk_id::ChunkId;
pub use error::{ChunkError, Error};
pub use params::ChunkParams;

use fastcdc::v2020::{FastCDC, StreamCDC};
use std::io::Read;

/// One content-defined chunk: byte range plus BLAKE3 content id.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct ChunkInfo {
    /// Starting offset within the original blob (bytes).
    pub offset: u64,
    /// Length of this chunk in bytes.
    pub length: u64,
    /// BLAKE3 of the plaintext chunk bytes.
    pub id: ChunkId,
}

/// Chunk an in-memory byte slice with FastCDC v2020 + default-or-custom params.
///
/// Same input + same params always yields the same cut points and ids.
/// Empty input yields an empty list (no chunks).
pub fn chunk_bytes(data: &[u8], params: &ChunkParams) -> Vec<ChunkInfo> {
    let chunker = FastCDC::new(
        data,
        params.min as usize,
        params.avg as usize,
        params.max as usize,
    );
    chunker
        .map(|c| {
            let slice = &data[c.offset..c.offset + c.length];
            ChunkInfo {
                offset: c.offset as u64,
                length: c.length as u64,
                id: ChunkId::hash(slice),
            }
        })
        .collect()
}

/// Chunk a [`Read`] source, yielding `(offset, length, ChunkId)` for each cut.
///
/// Uses FastCDC v2020 [`StreamCDC`]. I/O errors are returned as [`Error::Io`].
pub fn chunk_reader<R: Read>(reader: R, params: &ChunkParams) -> Result<Vec<ChunkInfo>, Error> {
    let chunker = StreamCDC::new(
        reader,
        params.min as usize,
        params.avg as usize,
        params.max as usize,
    );
    let mut out = Vec::new();
    for result in chunker {
        let chunk = result.map_err(|e| match e {
            fastcdc::v2020::Error::IoError(io) => Error::Io(io),
            other => Error::Chunker(other.to_string()),
        })?;
        let id = ChunkId::hash(&chunk.data);
        out.push(ChunkInfo {
            offset: chunk.offset,
            length: chunk.length as u64,
            id,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;

    fn fixtures_dir() -> PathBuf {
        // Crate CARGO_MANIFEST_DIR = crates/chunkforge-chunk → repo root is ../..
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures")
    }

    #[test]
    fn default_params_are_even_and_ordered() {
        let p = ChunkParams::default();
        assert_eq!(p.min, 16 * 1024);
        assert_eq!(p.avg, 64 * 1024);
        assert_eq!(p.max, 256 * 1024);
        assert_eq!(p.min % 2, 0);
        assert_eq!(p.avg % 2, 0);
        assert_eq!(p.max % 2, 0);
        assert!(p.min <= p.avg && p.avg <= p.max);
    }

    #[test]
    fn params_reject_odd_sizes() {
        let err = ChunkParams::new(16385, 65536, 262144).unwrap_err();
        assert!(matches!(err, Error::InvalidParams(_)));
        let msg = err.to_string();
        assert!(msg.contains("even") || msg.contains("odd"), "{msg}");
    }

    #[test]
    fn params_reject_min_gt_avg() {
        let err = ChunkParams::new(65536, 16384, 262144).unwrap_err();
        assert!(matches!(err, Error::InvalidParams(_)));
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("min") && (msg.contains("avg") || msg.contains("≤") || msg.contains("<=")),
            "{msg}"
        );
    }

    #[test]
    fn params_reject_avg_gt_max() {
        let err = ChunkParams::new(16384, 262144, 65536).unwrap_err();
        assert!(matches!(err, Error::InvalidParams(_)));
    }

    #[test]
    fn chunk_id_hex_roundtrip() {
        let id = ChunkId::hash(b"hello chunkforge\n");
        let hex = id.to_hex();
        assert_eq!(hex.len(), 64);
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        let parsed = ChunkId::from_hex(&hex).unwrap();
        assert_eq!(id, parsed);
        assert_eq!(format!("{id}"), hex);
    }

    #[test]
    fn chunk_id_rejects_bad_hex() {
        assert!(ChunkId::from_hex("gg").is_err());
        assert!(ChunkId::from_hex(&"ab".repeat(31)).is_err()); // 62 chars
        assert!(ChunkId::from_hex(&"AB".repeat(32)).is_err()); // uppercase
    }

    #[test]
    fn empty_input_yields_no_chunks() {
        let params = ChunkParams::default();
        let chunks = chunk_bytes(&[], &params);
        assert!(chunks.is_empty());
        let from_reader = chunk_reader(Cursor::new(Vec::<u8>::new()), &params).unwrap();
        assert!(from_reader.is_empty());
    }

    #[test]
    fn same_input_same_cuts() {
        let data = fs::read(fixtures_dir().join("zeros-64k.bin")).unwrap();
        let params = ChunkParams::default();
        let a = chunk_bytes(&data, &params);
        let b = chunk_bytes(&data, &params);
        assert_eq!(a, b);
        let c = chunk_reader(Cursor::new(data.clone()), &params).unwrap();
        assert_eq!(a, c);
    }

    #[test]
    fn hello_golden_vector() {
        let data = fs::read(fixtures_dir().join("hello.txt")).unwrap();
        assert_eq!(data, b"hello chunkforge\n");
        let params = ChunkParams::default();
        let chunks = chunk_bytes(&data, &params);

        // Tiny file < min → single chunk covering the whole blob.
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].offset, 0);
        assert_eq!(chunks[0].length, 17);
        let expected_hex = ChunkId::hash(b"hello chunkforge\n").to_hex();
        assert_eq!(chunks[0].id.to_hex(), expected_hex);

        let golden_path = fixtures_dir().join("golden/hello.chunklist.json");
        let golden: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&golden_path).unwrap()).unwrap();
        assert_eq!(golden["algorithm"], "fastcdc-v2020");
        assert_eq!(golden["params"]["min"], 16384);
        assert_eq!(golden["params"]["avg"], 65536);
        assert_eq!(golden["params"]["max"], 262144);
        let list = golden["chunks"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["offset"], 0);
        assert_eq!(list[0]["length"], 17);
        assert_eq!(list[0]["blake3_hex"], expected_hex);
    }

    #[test]
    fn binary_256_single_chunk_under_min() {
        let data = fs::read(fixtures_dir().join("binary-256.bin")).unwrap();
        assert_eq!(data.len(), 256);
        let params = ChunkParams::default();
        let chunks = chunk_bytes(&data, &params);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].offset, 0);
        assert_eq!(chunks[0].length, 256);
        assert_eq!(chunks[0].id, ChunkId::hash(&data));
    }

    #[test]
    fn zeros_64k_covers_full_length() {
        let data = fs::read(fixtures_dir().join("zeros-64k.bin")).unwrap();
        let params = ChunkParams::default();
        let chunks = chunk_bytes(&data, &params);
        let total: u64 = chunks.iter().map(|c| c.length).sum();
        assert_eq!(total, data.len() as u64);
        assert_eq!(chunks.first().map(|c| c.offset), Some(0));
        // Continuity: each chunk starts where the previous ended.
        for w in chunks.windows(2) {
            assert_eq!(w[0].offset + w[0].length, w[1].offset);
        }
    }
}
