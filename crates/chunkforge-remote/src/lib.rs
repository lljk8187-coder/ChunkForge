//! Remote / external [`ChunkSource`] backends for ChunkForge Phase 2.
//!
//! - [`HttpChunkSource`]: `GET {base}/chunks/<2hex>/<62hex>.cnk`
//! - [`FileUrlSource`]: `file:///path/to/store` or a plain local path → [`Store::open`]
//!
//! See `docs/remote-layout.md` for URL layout and usage.

mod file_url;
mod http;
mod layout;

pub use file_url::{FileUrlSource, parse_store_location};
pub use http::HttpChunkSource;
pub use layout::chunk_http_path;

// Re-exports for convenience when depending only on this crate.
pub use chunkforge_store::{ChunkId, ChunkSource, SourceError, Store};
