//! Local content-addressed chunk store (CAS) for ChunkForge.
//!
//! Layout (Phase 1, loose chunks — no packfiles):
//! ```text
//! <store_root>/
//!   meta.toml          # magic/version, default compression strategy
//!   chunks/
//!     ab/              # first 2 hex of BLAKE3
//!       cdef...rest.cnk  # remaining 62 hex + .cnk
//! ```
//!
//! Hash is always over **plaintext** (before compress / after decompress).
//! Compression policy is uniform per store (`meta.toml`); Phase 1 does not
//! allow per-chunk mixed compression.
//!
//! Optional `zstd` cargo feature enables zstd on-disk encoding.

mod error;
mod meta;
mod outcome;
mod path;
mod store;

pub use error::{Error, StoreError};
pub use meta::{Compression, MAGIC, StoreMeta, VERSION};
pub use outcome::PutOutcome;
pub use path::{chunk_abs_path, chunk_rel_path};
pub use store::Store;

// Re-export ChunkId so callers can depend only on chunkforge-store when convenient.
pub use chunkforge_chunk::ChunkId;
