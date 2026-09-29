//! HTTP static chunk-directory [`ChunkSource`].

use crate::layout::chunk_url;
use chunkforge_store::{ChunkId, ChunkSource, SourceError};
use std::time::Duration;
use ureq::Agent;

/// Fetch chunks over HTTP(S) from a static CAS directory.
///
/// URL layout matches the local store:
/// `GET {base}/chunks/<2hex>/<62hex>.cnk`.
///
/// Phase 2 keeps this simple: the response body is treated as **plaintext**
/// chunk bytes (same as an uncompressed `.cnk`). BLAKE3 is verified against
/// [`ChunkId`] when [`verify_hash`](Self::verify_hash) is true (default).
#[derive(Debug, Clone)]
pub struct HttpChunkSource {
    base: String,
    verify_hash: bool,
    agent: Agent,
}

impl HttpChunkSource {
    /// Create a source with default settings (`verify_hash = true`, 30s timeout).
    pub fn new(base: impl Into<String>) -> Self {
        Self::builder(base).build()
    }

    /// Start a builder for custom options.
    pub fn builder(base: impl Into<String>) -> HttpChunkSourceBuilder {
        HttpChunkSourceBuilder {
            base: base.into(),
            verify_hash: true,
            timeout: Some(Duration::from_secs(30)),
        }
    }

    /// HTTP(S) base URL (no trailing slash required).
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Whether `get` verifies `blake3(body) == id` (default true).
    pub fn verify_hash(&self) -> bool {
        self.verify_hash
    }

    /// Absolute URL for `id` under this base.
    pub fn url_for(&self, id: &ChunkId) -> String {
        chunk_url(&self.base, id)
    }

    fn map_ureq_err(id: &ChunkId, err: ureq::Error) -> SourceError {
        match err {
            ureq::Error::StatusCode(404) | ureq::Error::StatusCode(410) => {
                SourceError::NotFound(*id)
            }
            ureq::Error::StatusCode(code) => {
                SourceError::Backend(format!("HTTP {code} fetching chunk {id}"))
            }
            other => SourceError::Backend(format!("HTTP error fetching chunk {id}: {other}")),
        }
    }
}

/// Builder for [`HttpChunkSource`].
#[derive(Debug)]
pub struct HttpChunkSourceBuilder {
    base: String,
    verify_hash: bool,
    timeout: Option<Duration>,
}

impl HttpChunkSourceBuilder {
    /// Enable or disable BLAKE3 verification on `get` (default: true).
    pub fn verify_hash(mut self, verify: bool) -> Self {
        self.verify_hash = verify;
        self
    }

    /// Global request timeout (default: 30s). `None` disables.
    pub fn timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }

    /// Build the source.
    pub fn build(self) -> HttpChunkSource {
        let mut config = Agent::config_builder();
        if let Some(t) = self.timeout {
            config = config.timeout_global(Some(t));
        }
        let agent: Agent = config.build().into();
        HttpChunkSource {
            base: self.base.trim_end_matches('/').to_string(),
            verify_hash: self.verify_hash,
            agent,
        }
    }
}

impl ChunkSource for HttpChunkSource {
    fn has(&self, id: &ChunkId) -> Result<bool, SourceError> {
        let url = self.url_for(id);
        match self.agent.head(&url).call() {
            Ok(_resp) => Ok(true),
            Err(ureq::Error::StatusCode(404) | ureq::Error::StatusCode(410)) => Ok(false),
            // Some static servers reject HEAD; fall back to a GET and discard the body.
            Err(ureq::Error::StatusCode(405) | ureq::Error::StatusCode(501)) => {
                match self.agent.get(&url).call() {
                    Ok(mut resp) => {
                        // Drain / discard body.
                        let _ = resp.body_mut().read_to_vec();
                        Ok(true)
                    }
                    Err(ureq::Error::StatusCode(404) | ureq::Error::StatusCode(410)) => Ok(false),
                    Err(e) => Err(Self::map_ureq_err(id, e)),
                }
            }
            Err(e) => Err(Self::map_ureq_err(id, e)),
        }
    }

    fn get(&self, id: &ChunkId) -> Result<Vec<u8>, SourceError> {
        let url = self.url_for(id);
        let mut resp = self
            .agent
            .get(&url)
            .call()
            .map_err(|e| Self::map_ureq_err(id, e))?;

        let status = resp.status();
        if !(200..300).contains(&status.as_u16()) {
            // Defensive: with default ureq config non-2xx is already an error.
            return Err(SourceError::Backend(format!(
                "HTTP {} fetching chunk {id}",
                status.as_u16()
            )));
        }

        let body = resp
            .body_mut()
            .read_to_vec()
            .map_err(|e| SourceError::Backend(format!("HTTP body read for {id}: {e}")))?;

        if body.is_empty() {
            return Err(SourceError::Backend(format!(
                "empty HTTP body for chunk {id}"
            )));
        }

        if self.verify_hash {
            let actual = ChunkId::hash(&body);
            if actual != *id {
                return Err(SourceError::Corrupt(*id));
            }
        }

        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chunkforge_store::{Compression, Store};
    use std::fs;
    use std::sync::Arc;
    use std::thread;
    use tempfile::tempdir;
    use tiny_http::{Header, Method, Response, Server, StatusCode};

    fn spawn_static_store_server(
        store_root: std::path::PathBuf,
    ) -> (String, thread::JoinHandle<()>) {
        let server = Server::http("127.0.0.1:0").expect("bind");
        let port = server.server_addr().to_ip().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let handle = thread::spawn(move || {
            for request in server.incoming_requests() {
                let url = request.url().to_string();
                // Strip query if any.
                let path = url.split('?').next().unwrap_or(&url);
                let rel = path.trim_start_matches('/');
                let file_path = store_root.join(rel);

                if request.method() == &Method::Head || request.method() == &Method::Get {
                    if file_path.is_file() {
                        let data = fs::read(&file_path).unwrap_or_default();
                        if request.method() == &Method::Head {
                            let response = Response::empty(200).with_header(
                                Header::from_bytes(&b"Content-Length"[..], data.len().to_string())
                                    .unwrap(),
                            );
                            let _ = request.respond(response);
                        } else {
                            let response = Response::from_data(data);
                            let _ = request.respond(response);
                        }
                    } else {
                        let _ = request.respond(Response::empty(StatusCode(404)));
                    }
                } else {
                    let _ = request.respond(Response::empty(StatusCode(405)));
                }
            }
        });
        // Give the server a moment (incoming_requests blocks until accept).
        thread::sleep(Duration::from_millis(20));
        (base, handle)
    }

    #[test]
    fn http_get_has_roundtrip_against_local_store_layout() {
        let dir = tempdir().unwrap();
        let store = Store::create(dir.path(), Compression::None).unwrap();
        let data = b"phase2-m2-http-chunk";
        let (id, _) = store.put(data).unwrap();
        let root = dir.path().to_path_buf();
        drop(store);

        let (base, _handle) = spawn_static_store_server(root);
        let src = HttpChunkSource::builder(&base)
            .timeout(Some(Duration::from_secs(5)))
            .build();

        assert_eq!(
            src.url_for(&id),
            format!(
                "{}/{}",
                base.trim_end_matches('/'),
                crate::layout::chunk_http_path(&id)
            )
        );
        assert!(src.has(&id).unwrap());
        assert_eq!(src.get(&id).unwrap(), data);

        let missing = ChunkId::hash(b"no-such-chunk-in-http-test");
        assert!(!src.has(&missing).unwrap());
        assert!(matches!(
            src.get(&missing),
            Err(SourceError::NotFound(c)) if c == missing
        ));
    }

    #[test]
    fn http_empty_body_is_error() {
        let server = Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let _handle = thread::spawn(move || {
            if let Ok(request) = server.recv() {
                let _ = request.respond(Response::from_data(Vec::<u8>::new()));
            }
        });
        thread::sleep(Duration::from_millis(20));

        let id = ChunkId::hash(b"anything");
        let src = HttpChunkSource::builder(&base)
            .timeout(Some(Duration::from_secs(5)))
            .build();
        // Server replies 200 with empty body for any path.
        let err = src.get(&id).unwrap_err();
        assert!(
            matches!(err, SourceError::Backend(ref s) if s.contains("empty")),
            "{err:?}"
        );
    }

    #[test]
    fn http_corrupt_body_fails_verify() {
        let server = Arc::new(Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let server2 = Arc::clone(&server);
        let _handle = thread::spawn(move || {
            while let Ok(request) = server2.recv() {
                let _ = request.respond(Response::from_data(b"not-the-right-bytes".to_vec()));
            }
        });
        thread::sleep(Duration::from_millis(20));

        let id = ChunkId::hash(b"expected-plaintext");
        let src = HttpChunkSource::builder(&base)
            .timeout(Some(Duration::from_secs(5)))
            .build();
        assert!(matches!(
            src.get(&id),
            Err(SourceError::Corrupt(c)) if c == id
        ));

        let no_verify = HttpChunkSource::builder(&base)
            .verify_hash(false)
            .timeout(Some(Duration::from_secs(5)))
            .build();
        assert_eq!(no_verify.get(&id).unwrap(), b"not-the-right-bytes");
    }

    #[test]
    fn http_non_2xx_maps_to_source_error() {
        let server = Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let _handle = thread::spawn(move || {
            if let Ok(request) = server.recv() {
                let _ = request.respond(Response::empty(StatusCode(500)));
            }
        });
        thread::sleep(Duration::from_millis(20));

        let id = ChunkId::hash(b"x");
        let src = HttpChunkSource::builder(&base)
            .timeout(Some(Duration::from_secs(5)))
            .build();
        let err = src.get(&id).unwrap_err();
        assert!(
            matches!(err, SourceError::Backend(ref s) if s.contains("500")),
            "{err:?}"
        );
    }
}
