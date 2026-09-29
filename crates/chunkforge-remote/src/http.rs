//! HTTP static chunk-directory [`ChunkSource`].

use crate::template::{TemplateCtx, TemplateError, expand_template, normalize_prefix};
use chunkforge_store::{ChunkId, ChunkSource, SourceError};
use std::time::Duration;
use ureq::Agent;

/// Default URL template — byte-compatible with Phase 2 `chunk_url` layout.
pub const DEFAULT_URL_TEMPLATE: &str = "{base}/{path}";

/// Fetch chunks over HTTP(S) from a static CAS directory (optionally templated).
///
/// Default URL layout matches the local store and Phase 2:
/// `GET {base}/chunks/<2hex>/<62hex>.cnk` via template [`DEFAULT_URL_TEMPLATE`].
///
/// Custom [`url_template`](HttpChunkSourceBuilder::url_template),
/// [`prefix`](HttpChunkSourceBuilder::prefix), and
/// [`header`](HttpChunkSourceBuilder::header) templates are expanded per request
/// with [`expand_template`]. Templates are validated once in
/// [`HttpChunkSourceBuilder::build`] against an all-zero [`ChunkId`].
///
/// The response body is treated as **plaintext** chunk bytes (same as an
/// uncompressed `.cnk`). BLAKE3 is verified against [`ChunkId`] when
/// [`verify_hash`](Self::verify_hash) is true (default).
#[derive(Debug, Clone)]
pub struct HttpChunkSource {
    base: String,
    verify_hash: bool,
    agent: Agent,
    url_template: String,
    header_templates: Vec<(String, String)>,
    prefix: String,
}

impl HttpChunkSource {
    /// Create a source with default settings (`verify_hash = true`, 30s timeout,
    /// default URL template, empty prefix, no custom headers).
    pub fn new(base: impl Into<String>) -> Self {
        Self::builder(base)
            .build()
            .expect("default HttpChunkSource template is infallible")
    }

    /// Start a builder for custom options.
    pub fn builder(base: impl Into<String>) -> HttpChunkSourceBuilder {
        HttpChunkSourceBuilder {
            base: base.into(),
            verify_hash: true,
            timeout: Some(Duration::from_secs(30)),
            url_template: DEFAULT_URL_TEMPLATE.to_string(),
            header_templates: Vec::new(),
            prefix: String::new(),
        }
    }

    /// HTTP(S) base URL (trailing `/` stripped).
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Whether `get` verifies `blake3(body) == id` (default true).
    pub fn verify_hash(&self) -> bool {
        self.verify_hash
    }

    /// URL template string (default [`DEFAULT_URL_TEMPLATE`]).
    pub fn url_template(&self) -> &str {
        &self.url_template
    }

    /// Normalized key prefix used for `{prefix}` (empty or `foo/` form).
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Header name → value-template pairs applied on every request.
    pub fn header_templates(&self) -> &[(String, String)] {
        &self.header_templates
    }

    /// Absolute URL for `id` under this source's template.
    ///
    /// Infallible after a successful [`HttpChunkSourceBuilder::build`] (templates
    /// were already validated). Panics only if the process environment lost a
    /// variable required by `{env:…}` between build and this call.
    pub fn url_for(&self, id: &ChunkId) -> String {
        self.expand_url(id)
            .expect("url_template validated at build; env vars must remain set")
    }

    fn template_ctx<'a>(&'a self, id: &'a ChunkId) -> TemplateCtx<'a> {
        TemplateCtx {
            base: &self.base,
            id,
            prefix: &self.prefix,
        }
    }

    fn expand_url(&self, id: &ChunkId) -> Result<String, TemplateError> {
        expand_template(&self.url_template, &self.template_ctx(id))
    }

    fn expand_headers(&self, id: &ChunkId) -> Result<Vec<(String, String)>, TemplateError> {
        let ctx = self.template_ctx(id);
        self.header_templates
            .iter()
            .map(|(name, tmpl)| {
                let value = expand_template(tmpl, &ctx)?;
                Ok((name.clone(), value))
            })
            .collect()
    }

    fn template_err(id: &ChunkId, err: TemplateError) -> SourceError {
        SourceError::Backend(format!("template error for chunk {id}: {err}"))
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
    url_template: String,
    header_templates: Vec<(String, String)>,
    prefix: String,
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

    /// URL template expanded per chunk (default [`DEFAULT_URL_TEMPLATE`]).
    pub fn url_template(mut self, tmpl: impl Into<String>) -> Self {
        self.url_template = tmpl.into();
        self
    }

    /// Value for `{prefix}` (normalized to empty or `foo/` form at build).
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Append a request header whose value is a template (expanded per request).
    pub fn header(mut self, name: impl Into<String>, value_template: impl Into<String>) -> Self {
        self.header_templates
            .push((name.into(), value_template.into()));
        self
    }

    /// Replace all header templates.
    pub fn header_templates(
        mut self,
        headers: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.header_templates = headers
            .into_iter()
            .map(|(n, v)| (n.into(), v.into()))
            .collect();
        self
    }

    /// Build the source.
    ///
    /// Validates `url_template` and every header value template by expanding
    /// once against an all-zero [`ChunkId`]. Unknown placeholders / missing
    /// `{env:…}` variables fail fast here.
    pub fn build(self) -> Result<HttpChunkSource, TemplateError> {
        let base = self.base.trim_end_matches('/').to_string();
        let prefix = normalize_prefix(&self.prefix);
        let fake_id = ChunkId::from_bytes([0u8; 32]);
        let ctx = TemplateCtx {
            base: &base,
            id: &fake_id,
            prefix: &prefix,
        };
        expand_template(&self.url_template, &ctx)?;
        for (_name, value_tmpl) in &self.header_templates {
            expand_template(value_tmpl, &ctx)?;
        }

        let mut config = Agent::config_builder();
        if let Some(t) = self.timeout {
            config = config.timeout_global(Some(t));
        }
        let agent: Agent = config.build().into();
        Ok(HttpChunkSource {
            base,
            verify_hash: self.verify_hash,
            agent,
            url_template: self.url_template,
            header_templates: self.header_templates,
            prefix,
        })
    }
}

impl ChunkSource for HttpChunkSource {
    fn has(&self, id: &ChunkId) -> Result<bool, SourceError> {
        let url = self.expand_url(id).map_err(|e| Self::template_err(id, e))?;
        let headers = self
            .expand_headers(id)
            .map_err(|e| Self::template_err(id, e))?;

        let mut req = self.agent.head(&url);
        for (name, value) in &headers {
            req = req.header(name.as_str(), value.as_str());
        }

        match req.call() {
            Ok(_resp) => Ok(true),
            Err(ureq::Error::StatusCode(404) | ureq::Error::StatusCode(410)) => Ok(false),
            // Some static servers reject HEAD; fall back to a GET and discard the body.
            Err(ureq::Error::StatusCode(405) | ureq::Error::StatusCode(501)) => {
                let mut req = self.agent.get(&url);
                for (name, value) in &headers {
                    req = req.header(name.as_str(), value.as_str());
                }
                match req.call() {
                    Ok(mut resp) => {
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
        let url = self.expand_url(id).map_err(|e| Self::template_err(id, e))?;
        let headers = self
            .expand_headers(id)
            .map_err(|e| Self::template_err(id, e))?;

        let mut req = self.agent.get(&url);
        for (name, value) in &headers {
            req = req.header(name.as_str(), value.as_str());
        }

        let mut resp = req.call().map_err(|e| Self::map_ureq_err(id, e))?;

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
    use crate::layout::{chunk_http_path, chunk_url};
    use chunkforge_store::{Compression, Store};
    use std::fs;
    use std::sync::Arc;
    use std::sync::Mutex;
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
            .build()
            .unwrap();

        assert_eq!(
            src.url_for(&id),
            format!(
                "{}/{}",
                base.trim_end_matches('/'),
                crate::layout::chunk_http_path(&id)
            )
        );
        // Default template ≡ legacy chunk_url.
        assert_eq!(src.url_for(&id), chunk_url(&base, &id));
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
            .build()
            .unwrap();
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
            .build()
            .unwrap();
        assert!(matches!(
            src.get(&id),
            Err(SourceError::Corrupt(c)) if c == id
        ));

        let no_verify = HttpChunkSource::builder(&base)
            .verify_hash(false)
            .timeout(Some(Duration::from_secs(5)))
            .build()
            .unwrap();
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
            .build()
            .unwrap();
        let err = src.get(&id).unwrap_err();
        assert!(
            matches!(err, SourceError::Backend(ref s) if s.contains("500")),
            "{err:?}"
        );
    }

    #[test]
    fn custom_url_template_and_prefix_expand() {
        let id = ChunkId::hash(b"phase3-m1-custom-tmpl");
        let hex = id.to_hex();
        let src = HttpChunkSource::builder("https://minio.example/mybucket/")
            .url_template("{base}/{prefix}{path}")
            .prefix("data")
            .timeout(Some(Duration::from_secs(5)))
            .build()
            .unwrap();

        assert_eq!(src.prefix(), "data/");
        assert_eq!(
            src.url_for(&id),
            format!(
                "https://minio.example/mybucket/data/chunks/{}/{}.cnk",
                &hex[..2],
                &hex[2..]
            )
        );
        assert_eq!(
            src.url_for(&id),
            format!(
                "https://minio.example/mybucket/data/{}",
                chunk_http_path(&id)
            )
        );
    }

    #[test]
    fn unknown_placeholder_fails_at_build() {
        let err = HttpChunkSource::builder("http://127.0.0.1:9")
            .url_template("{base}/{bucket}/{path}")
            .build()
            .unwrap_err();
        assert_eq!(err, TemplateError::UnknownPlaceholder("bucket".into()));
    }

    #[test]
    fn unknown_header_placeholder_fails_at_build() {
        let err = HttpChunkSource::builder("http://127.0.0.1:9")
            .header("X-Trace", "{request_id}")
            .build()
            .unwrap_err();
        assert_eq!(err, TemplateError::UnknownPlaceholder("request_id".into()));
    }

    #[test]
    fn custom_header_template_sent_on_get() {
        let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let seen2 = Arc::clone(&seen);
        let server = Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let body = b"phase3-m1-header-body".to_vec();
        let id = ChunkId::hash(&body);
        let body2 = body.clone();
        let _handle = thread::spawn(move || {
            if let Ok(request) = server.recv() {
                let auth = request
                    .headers()
                    .iter()
                    .find(|h| {
                        h.field
                            .as_str()
                            .as_str()
                            .eq_ignore_ascii_case("Authorization")
                    })
                    .map(|h| h.value.as_str().to_string());
                *seen2.lock().unwrap() = auth;
                let _ = request.respond(Response::from_data(body2));
            }
        });
        thread::sleep(Duration::from_millis(20));

        let var = "CHUNKFORGE_M1_HTTP_HEADER_TOKEN";
        unsafe { std::env::set_var(var, "tok-m1") };
        let src = HttpChunkSource::builder(&base)
            .verify_hash(true)
            .header(
                "Authorization",
                "Bearer {env:CHUNKFORGE_M1_HTTP_HEADER_TOKEN}",
            )
            .timeout(Some(Duration::from_secs(5)))
            .build()
            .unwrap();
        assert_eq!(src.get(&id).unwrap(), body);
        assert_eq!(seen.lock().unwrap().as_deref(), Some("Bearer tok-m1"));
        unsafe { std::env::remove_var(var) };
    }

    #[test]
    fn missing_env_in_header_fails_at_build() {
        let var = "CHUNKFORGE_M1_MISSING_ENV_XYZ";
        unsafe { std::env::remove_var(var) };
        let err = HttpChunkSource::builder("http://127.0.0.1:9")
            .header(
                "Authorization",
                "Bearer {env:CHUNKFORGE_M1_MISSING_ENV_XYZ}",
            )
            .build()
            .unwrap_err();
        assert_eq!(
            err,
            TemplateError::MissingEnv("CHUNKFORGE_M1_MISSING_ENV_XYZ".into())
        );
    }

    /// Phase3-M2: S3-compatible path + Authorization via tiny_http mock.
    /// Fixed ChunkId; template `{base}/{prefix}{path}` + `prefix=data/` →
    /// GET path contains `/data/chunks/…`; Authorization matches expansion.
    #[test]
    fn s3_prefix_path_and_authorization_on_mock_get() {
        #[derive(Default)]
        struct Seen {
            path: Option<String>,
            method: Option<String>,
            authorization: Option<String>,
        }
        let seen: Arc<Mutex<Seen>> = Arc::new(Mutex::new(Seen::default()));
        let seen2 = Arc::clone(&seen);

        // Fixed id so the expected key path is deterministic.
        let id = ChunkId::from_bytes([0xab; 32]);
        let hex = id.to_hex();
        assert_eq!(&hex[..2], "ab");
        let plaintext = b"phase3-m2-s3-path-body".to_vec();
        // Serve bytes whose hash is NOT id — disable verify_hash so we only
        // assert request path/header (hash verify is covered elsewhere).
        let body2 = plaintext.clone();

        let server = Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let _handle = thread::spawn(move || {
            if let Ok(request) = server.recv() {
                let mut g = seen2.lock().unwrap();
                g.method = Some(format!("{:?}", request.method()));
                g.path = Some(request.url().to_string());
                g.authorization = request
                    .headers()
                    .iter()
                    .find(|h| {
                        h.field
                            .as_str()
                            .as_str()
                            .eq_ignore_ascii_case("Authorization")
                    })
                    .map(|h| h.value.as_str().to_string());
                drop(g);
                let _ = request.respond(Response::from_data(body2));
            }
        });
        thread::sleep(Duration::from_millis(20));

        let var = "CHUNKFORGE_M2_S3_AUTH_TOKEN";
        unsafe { std::env::set_var(var, "m2-tok-fixed") };

        let src = HttpChunkSource::builder(&base)
            .url_template("{base}/{prefix}{path}")
            .prefix("data/")
            .header("Authorization", "Bearer {env:CHUNKFORGE_M2_S3_AUTH_TOKEN}")
            .verify_hash(false)
            .timeout(Some(Duration::from_secs(5)))
            .build()
            .unwrap();

        // url_for already encodes the S3-compatible key under the prefix.
        let expected_url = format!(
            "{}/data/chunks/{}/{}.cnk",
            base.trim_end_matches('/'),
            &hex[..2],
            &hex[2..]
        );
        assert_eq!(src.url_for(&id), expected_url);
        assert!(
            src.url_for(&id).contains("/data/chunks/"),
            "url must contain /data/chunks/: {}",
            src.url_for(&id)
        );

        assert_eq!(src.get(&id).unwrap(), plaintext);

        let g = seen.lock().unwrap();
        let path = g.path.as_deref().expect("mock should have seen a request");
        assert!(
            path.contains("/data/chunks/"),
            "GET path must contain /data/chunks/, got {path}"
        );
        assert!(
            path.contains(&format!("/data/chunks/{}/{}.cnk", &hex[..2], &hex[2..])),
            "GET path must be the full CAS key under prefix, got {path}"
        );
        assert_eq!(
            g.authorization.as_deref(),
            Some("Bearer m2-tok-fixed"),
            "Authorization must match template expansion"
        );

        unsafe { std::env::remove_var(var) };
    }
}
