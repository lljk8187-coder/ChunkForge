//! `.cfidx` v1 binary layout (little-endian).
//!
//! See `docs/index-format.md` and Phase 1 spec §3.

use crate::Error;
use chunkforge_chunk::{ChunkId, ChunkParams};
use std::io::Write;

/// Full 8-byte magic for Phase 1 writes: `b"CFIDX\0\0\x01"` (last byte = major).
pub const MAGIC_V1: [u8; 8] = *b"CFIDX\0\0\x01";
/// First 7 bytes of magic (identity); byte 7 is the major version.
pub const MAGIC_PREFIX: [u8; 7] = *b"CFIDX\0\0";
/// Phase 1 major version embedded in magic[7].
pub const MAJOR_V1: u8 = 1;
/// Phase 1 `format_version_u16`.
pub const FORMAT_VERSION_V1: u16 = 1;

/// Size of the fixed header (through `blob_blake3`).
pub const HEADER_SIZE: usize = 88;
/// Size of one entry (`end_offset` + `chunk_id`).
pub const ENTRY_SIZE: usize = 40;
/// Size of the trailer BLAKE3 checksum.
pub const TRAILER_SIZE: usize = 32;

/// `flags_u16` bit0: chunks are compressed in the store (hint only, not authoritative).
pub const FLAG_CHUNKS_COMPRESSED_IN_STORE: u16 = 1 << 0;

/// One index entry: exclusive end offset in the blob + chunk content id.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct IndexEntry {
    /// End offset (exclusive) of this chunk within the original blob.
    /// First chunk starts at 0; length = `end_offset - prev_end` (prev_end=0 for i=0).
    pub end_offset: u64,
    /// BLAKE3 of the plaintext chunk bytes (raw 32 bytes on disk).
    pub chunk_id: ChunkId,
}

/// In-memory `.cfidx` v1 index.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Index {
    /// `flags_u16` from the header.
    pub flags: u16,
    /// FastCDC parameters recorded in the header.
    pub params: ChunkParams,
    /// Original blob byte length.
    pub total_size: u64,
    /// BLAKE3 of the entire plaintext blob (end-to-end check).
    pub blob_blake3: ChunkId,
    /// Chunk table; empty iff `total_size == 0`.
    pub entries: Vec<IndexEntry>,
}

impl Index {
    /// Construct and validate a structurally legal index.
    pub fn new(
        flags: u16,
        params: ChunkParams,
        total_size: u64,
        blob_blake3: ChunkId,
        entries: Vec<IndexEntry>,
    ) -> Result<Self, Error> {
        let idx = Self {
            flags,
            params,
            total_size,
            blob_blake3,
            entries,
        };
        idx.validate()?;
        Ok(idx)
    }

    /// Empty-file index (`total_size == 0`, no entries).
    pub fn empty(params: ChunkParams) -> Self {
        Self {
            flags: 0,
            params,
            total_size: 0,
            blob_blake3: ChunkId::hash(b""),
            entries: Vec::new(),
        }
    }

    /// Number of chunks (`entries.len()` as `u64`).
    pub fn chunk_count(&self) -> u64 {
        self.entries.len() as u64
    }

    /// Validate Phase 1 structural rules (does not touch checksum / magic).
    pub fn validate(&self) -> Result<(), Error> {
        if self.entries.is_empty() {
            if self.total_size != 0 {
                return Err(Error::InvalidStructure(
                    "chunk_count == 0 requires total_size == 0".into(),
                ));
            }
            return Ok(());
        }
        if self.total_size == 0 {
            return Err(Error::InvalidStructure(
                "total_size == 0 requires chunk_count == 0".into(),
            ));
        }
        let mut prev = 0u64;
        for (i, e) in self.entries.iter().enumerate() {
            if e.end_offset <= prev {
                return Err(Error::InvalidStructure(format!(
                    "entry[{i}] end_offset {} is not strictly greater than previous {}",
                    e.end_offset, prev
                )));
            }
            prev = e.end_offset;
        }
        let last = self.entries.last().unwrap().end_offset;
        if last != self.total_size {
            return Err(Error::InvalidStructure(format!(
                "last end_offset ({last}) must equal total_size ({})",
                self.total_size
            )));
        }
        Ok(())
    }

    /// Encode to `.cfidx` v1 bytes (header || entries || trailer).
    ///
    /// Always writes major=1, `format_version=1`, `reserved=0`.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        self.validate()?;

        let n = self.entries.len();
        let mut buf = Vec::with_capacity(HEADER_SIZE + n * ENTRY_SIZE + TRAILER_SIZE);

        buf.extend_from_slice(&MAGIC_V1);
        buf.extend_from_slice(&FORMAT_VERSION_V1.to_le_bytes());
        buf.extend_from_slice(&self.flags.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // reserved
        buf.extend_from_slice(&self.params.min.to_le_bytes());
        buf.extend_from_slice(&self.params.avg.to_le_bytes());
        buf.extend_from_slice(&self.params.max.to_le_bytes());
        buf.extend_from_slice(&self.total_size.to_le_bytes());
        buf.extend_from_slice(&(n as u64).to_le_bytes());
        buf.extend_from_slice(self.blob_blake3.as_bytes());
        debug_assert_eq!(buf.len(), HEADER_SIZE);

        for e in &self.entries {
            buf.extend_from_slice(&e.end_offset.to_le_bytes());
            buf.extend_from_slice(e.chunk_id.as_bytes());
        }
        debug_assert_eq!(buf.len(), HEADER_SIZE + n * ENTRY_SIZE);

        let digest = blake3::hash(&buf);
        buf.extend_from_slice(digest.as_bytes());
        Ok(buf)
    }

    /// Decode `.cfidx` v1 bytes. Hard-fails on wrong major, truncate, or bad trailer.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < HEADER_SIZE + TRAILER_SIZE {
            return Err(Error::Truncated(format!(
                "need at least {} bytes for header+trailer, got {}",
                HEADER_SIZE + TRAILER_SIZE,
                bytes.len()
            )));
        }

        let magic = &bytes[0..8];
        if magic[..7] != MAGIC_PREFIX {
            return Err(Error::BadMagic);
        }
        let major = magic[7];
        if major != MAJOR_V1 {
            return Err(Error::UnsupportedMajor {
                found: major,
                expected: MAJOR_V1,
            });
        }

        let format_version = u16::from_le_bytes(bytes[8..10].try_into().unwrap());
        if format_version != FORMAT_VERSION_V1 {
            // Phase 1: only format_version=1 is understood; unknown minor → fail safe.
            return Err(Error::UnsupportedFormatVersion {
                found: format_version,
                supported: FORMAT_VERSION_V1,
            });
        }

        let flags = u16::from_le_bytes(bytes[10..12].try_into().unwrap());
        let _reserved = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        // Phase 1 writers set reserved=0; ignore non-zero for forward compatibility of
        // reserved bits that do not change semantics (major gates incompatible changes).

        let min = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        let avg = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        let max = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
        let params = ChunkParams::new(min, avg, max).map_err(|e| {
            Error::InvalidStructure(format!("chunk size params in header: {e}"))
        })?;

        let total_size = u64::from_le_bytes(bytes[40..48].try_into().unwrap());
        let chunk_count = u64::from_le_bytes(bytes[48..56].try_into().unwrap());
        let blob_blake3 = ChunkId::from_bytes(bytes[56..88].try_into().unwrap());

        if chunk_count > (usize::MAX / ENTRY_SIZE) as u64 {
            return Err(Error::Truncated(format!(
                "chunk_count {chunk_count} is unreasonably large"
            )));
        }
        let n = chunk_count as usize;
        let expected_len = HEADER_SIZE
            .checked_add(n.checked_mul(ENTRY_SIZE).ok_or_else(|| {
                Error::Truncated("entry table size overflow".into())
            })?)
            .and_then(|v| v.checked_add(TRAILER_SIZE))
            .ok_or_else(|| Error::Truncated("index size overflow".into()))?;

        if bytes.len() != expected_len {
            return Err(Error::Truncated(format!(
                "expected {expected_len} bytes for chunk_count={chunk_count}, got {}",
                bytes.len()
            )));
        }

        let body_end = HEADER_SIZE + n * ENTRY_SIZE;
        let body = &bytes[..body_end];
        let trailer = &bytes[body_end..body_end + TRAILER_SIZE];
        let computed = blake3::hash(body);
        if computed.as_bytes() != trailer {
            return Err(Error::TrailerMismatch);
        }

        let mut entries = Vec::with_capacity(n);
        let mut off = HEADER_SIZE;
        for _ in 0..n {
            let end_offset = u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap());
            let chunk_id = ChunkId::from_bytes(bytes[off + 8..off + 40].try_into().unwrap());
            entries.push(IndexEntry {
                end_offset,
                chunk_id,
            });
            off += ENTRY_SIZE;
        }

        Self::new(flags, params, total_size, blob_blake3, entries)
    }

    /// Write encoded bytes to `w`.
    pub fn write_to<W: Write>(&self, mut w: W) -> Result<(), Error> {
        let bytes = self.encode()?;
        w.write_all(&bytes)?;
        Ok(())
    }
}

/// Length of the i-th chunk (`end_offset[i] - end_offset[i-1]`, with prev=0 for i=0).
pub fn entry_length(entries: &[IndexEntry], i: usize) -> Option<u64> {
    let end = entries.get(i)?.end_offset;
    let start = if i == 0 {
        0
    } else {
        entries[i - 1].end_offset
    };
    Some(end - start)
}
