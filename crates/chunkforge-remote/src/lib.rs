//! Remote / external [`ChunkSource`] / [`ChunkSink`] backends for ChunkForge.
//!
//! - [`HttpChunkSource`]: `GET` via URL/header templates (default ≡ Phase 2 layout)
//! - [`HttpChunkSink`]: `PUT` (default) with the same templates — isomorphic keys
//! - [`FileUrlSource`]: `file:///path/to/store` or a plain local path → [`Store::open`]
//! - [`RetryPolicy`]: bounded retries for transient HTTP failures (default 0 ≡ 0.7.0)
//! - [`SigV4Config`] / [`SigV4Signer`]: optional in-process AWS SigV4 (default off)
//!
//! See `docs/remote-layout.md` and `docs/sigv4.md` for URL layout and signing.

mod endpoint;
mod file_url;
mod http;
mod http_sink;
mod layout;
mod retry;
mod sigv4;
mod template;

pub use file_url::{FileUrlSource, parse_store_location};
pub use http::{DEFAULT_URL_TEMPLATE, HttpBuildError, HttpChunkSource, HttpChunkSourceBuilder};
pub use http_sink::{HttpChunkSink, HttpChunkSinkBuilder, HttpPutMethod};
pub use layout::{chunk_http_path, chunk_url};
pub use retry::{
    ErrorClass, RetryPolicy, SummaryFailureBucket, classify_http_status, classify_sink_error,
    classify_source_error, classify_ureq_error, http_status_is_transient, ureq_error_is_transient,
};
pub use sigv4::{
    AwsCredentials, DEFAULT_REGION, DEFAULT_SERVICE, EMPTY_PAYLOAD_HASH, SigV4Config, SigV4Error,
    SigV4Signer, SignedHeaders, SigningClock, canonical_query, canonical_uri,
    parse_url_for_signing, payload_hash, sign_request, sign_url, signing_key, uri_encode,
};
pub use template::{TemplateCtx, TemplateError, expand_template, normalize_prefix};

// Re-exports for convenience when depending only on this crate.
pub use chunkforge_store::{
    CacheSource, ChunkId, ChunkSink, ChunkSource, PutOutcome, SinkError, SourceError, Store,
};
