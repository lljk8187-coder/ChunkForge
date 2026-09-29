//! Remote / external [`ChunkSource`] / [`ChunkSink`] backends for ChunkForge.
//!
//! - [`HttpChunkSource`]: `GET` via URL/header templates (default ≡ Phase 2 layout)
//! - [`HttpChunkSink`]: `PUT` (default) with the same templates — isomorphic keys
//! - [`FileUrlSource`]: `file:///path/to/store` or a plain local path → [`Store::open`]
//!
//! See `docs/remote-layout.md` for URL layout and usage.

mod endpoint;
mod file_url;
mod http;
mod http_sink;
mod layout;
mod template;

pub use file_url::{FileUrlSource, parse_store_location};
pub use http::{DEFAULT_URL_TEMPLATE, HttpChunkSource, HttpChunkSourceBuilder};
pub use http_sink::{HttpChunkSink, HttpChunkSinkBuilder, HttpPutMethod};
pub use layout::{chunk_http_path, chunk_url};
pub use template::{TemplateCtx, TemplateError, expand_template, normalize_prefix};

// Re-exports for convenience when depending only on this crate.
pub use chunkforge_store::{
    CacheSource, ChunkId, ChunkSink, ChunkSource, PutOutcome, SinkError, SourceError, Store,
};
