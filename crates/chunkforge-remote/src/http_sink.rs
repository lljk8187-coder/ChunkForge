//! HTTP chunk-directory [`ChunkSink`] (PUT), isomorphic with [`crate::HttpChunkSource`].

use crate::endpoint::HttpEndpoint;
use crate::http::DEFAULT_URL_TEMPLATE;
use crate::template::TemplateError;
use chunkforge_store::{ChunkId, ChunkSink, PutOutcome, SinkError};
use std::time::Duration;

/// HTTP method used for uploading a chunk body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HttpPutMethod {
    /// `PUT` (default — object-store convention).
    #[default]
    Put,
    /// `POST` (for stubs that only accept POST).
    Post,
}

/// Upload chunks over HTTP(S) with the same URL / header templates as
/// [`crate::HttpChunkSource`].
///
/// Default URL layout is [`DEFAULT_URL_TEMPLATE`] (`{base}/{path}`), identical to
/// the GET source, so a successful push is readable with the Phase 3 verify path.
///
/// Body is **plaintext** chunk bytes (same assumption as HTTP GET). When
/// [`verify_hash`](Self::verify_hash) is true (default), `put` asserts
/// `blake3(plain) == id` before sending.
///
/// Idempotency: with [`skip_if_exists`](Self::skip_if_exists) (default true),
/// `has` (HEAD, GET fallback) is checked first and returns
/// [`PutOutcome::SkippedExists`]. A `409 Conflict` response is also treated as
/// success / skip when [`accept_conflict`](Self::accept_conflict) is true
/// (default).
#[derive(Debug, Clone)]
pub struct HttpChunkSink {
    endpoint: HttpEndpoint,
    verify_hash: bool,
    skip_if_exists: bool,
    accept_conflict: bool,
    put_method: HttpPutMethod,
    content_type: String,
}

impl HttpChunkSink {
    /// Create a sink with defaults (verify_hash, skip_if_exists, accept_conflict,
    /// PUT, `application/octet-stream`, 30s timeout, default URL template).
    pub fn new(base: impl Into<String>) -> Self {
        Self::builder(base)
            .build()
            .expect("default HttpChunkSink template is infallible")
    }

    /// Start a builder for custom options.
    pub fn builder(base: impl Into<String>) -> HttpChunkSinkBuilder {
        HttpChunkSinkBuilder {
            base: base.into(),
            verify_hash: true,
            skip_if_exists: true,
            accept_conflict: true,
            put_method: HttpPutMethod::Put,
            content_type: "application/octet-stream".to_string(),
            timeout: Some(Duration::from_secs(30)),
            url_template: DEFAULT_URL_TEMPLATE.to_string(),
            header_templates: Vec::new(),
            prefix: String::new(),
        }
    }

    /// HTTP(S) base URL (trailing `/` stripped).
    pub fn base(&self) -> &str {
        &self.endpoint.base
    }

    /// Whether `put` verifies `blake3(plain) == id` before upload (default true).
    pub fn verify_hash(&self) -> bool {
        self.verify_hash
    }

    /// Whether `put` issues HEAD (via [`ChunkSink::has`]) and skips when present.
    pub fn skip_if_exists(&self) -> bool {
        self.skip_if_exists
    }

    /// Whether HTTP 409 on PUT/POST counts as [`PutOutcome::SkippedExists`].
    pub fn accept_conflict(&self) -> bool {
        self.accept_conflict
    }

    /// Upload HTTP method (default [`HttpPutMethod::Put`]).
    pub fn put_method(&self) -> HttpPutMethod {
        self.put_method
    }

    /// `Content-Type` sent on upload (default `application/octet-stream`).
    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    /// URL template string (default [`DEFAULT_URL_TEMPLATE`]).
    pub fn url_template(&self) -> &str {
        &self.endpoint.url_template
    }

    /// Normalized key prefix used for `{prefix}`.
    pub fn prefix(&self) -> &str {
        &self.endpoint.prefix
    }

    /// Header name → value-template pairs applied on every request.
    pub fn header_templates(&self) -> &[(String, String)] {
        &self.endpoint.header_templates
    }

    /// Absolute URL for `id` — same expansion as [`crate::HttpChunkSource::url_for`]
    /// when configured identically.
    pub fn url_for(&self, id: &ChunkId) -> String {
        self.endpoint.url_for(id)
    }

    fn template_err(id: &ChunkId, err: TemplateError) -> SinkError {
        SinkError::Backend(format!("template error for chunk {id}: {err}"))
    }

    fn map_ureq_err(id: &ChunkId, err: ureq::Error) -> SinkError {
        match err {
            ureq::Error::StatusCode(code) => {
                SinkError::Backend(format!("HTTP {code} writing chunk {id}"))
            }
            other => SinkError::Backend(format!("HTTP error writing chunk {id}: {other}")),
        }
    }

    fn map_ureq_has_err(id: &ChunkId, err: ureq::Error) -> SinkError {
        match err {
            ureq::Error::StatusCode(code) => {
                SinkError::Backend(format!("HTTP {code} probing chunk {id}"))
            }
            other => SinkError::Backend(format!("HTTP error probing chunk {id}: {other}")),
        }
    }
}

/// Builder for [`HttpChunkSink`].
#[derive(Debug)]
pub struct HttpChunkSinkBuilder {
    base: String,
    verify_hash: bool,
    skip_if_exists: bool,
    accept_conflict: bool,
    put_method: HttpPutMethod,
    content_type: String,
    timeout: Option<Duration>,
    url_template: String,
    header_templates: Vec<(String, String)>,
    prefix: String,
}

impl HttpChunkSinkBuilder {
    /// Enable or disable BLAKE3 verification on `put` (default: true).
    pub fn verify_hash(mut self, verify: bool) -> Self {
        self.verify_hash = verify;
        self
    }

    /// Probe with HEAD/`has` before PUT and skip when the object exists (default: true).
    pub fn skip_if_exists(mut self, skip: bool) -> Self {
        self.skip_if_exists = skip;
        self
    }

    /// Treat HTTP 409 Conflict as [`PutOutcome::SkippedExists`] (default: true).
    pub fn accept_conflict(mut self, accept: bool) -> Self {
        self.accept_conflict = accept;
        self
    }

    /// Upload method (default [`HttpPutMethod::Put`]).
    pub fn put_method(mut self, method: HttpPutMethod) -> Self {
        self.put_method = method;
        self
    }

    /// `Content-Type` for the upload body (default `application/octet-stream`).
    pub fn content_type(mut self, ct: impl Into<String>) -> Self {
        self.content_type = ct.into();
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

    /// Build the sink (validates templates against an all-zero [`ChunkId`]).
    pub fn build(self) -> Result<HttpChunkSink, TemplateError> {
        let endpoint = HttpEndpoint::build(
            self.base,
            self.timeout,
            self.url_template,
            self.header_templates,
            self.prefix,
        )?;
        Ok(HttpChunkSink {
            endpoint,
            verify_hash: self.verify_hash,
            skip_if_exists: self.skip_if_exists,
            accept_conflict: self.accept_conflict,
            put_method: self.put_method,
            content_type: self.content_type,
        })
    }
}

impl ChunkSink for HttpChunkSink {
    fn has(&self, id: &ChunkId) -> Result<bool, SinkError> {
        let url = self
            .endpoint
            .expand_url(id)
            .map_err(|e| Self::template_err(id, e))?;
        let headers = self
            .endpoint
            .expand_headers(id)
            .map_err(|e| Self::template_err(id, e))?;

        let mut req = self.endpoint.agent.head(&url);
        for (name, value) in &headers {
            req = req.header(name.as_str(), value.as_str());
        }

        match req.call() {
            Ok(_resp) => Ok(true),
            Err(ureq::Error::StatusCode(404) | ureq::Error::StatusCode(410)) => Ok(false),
            Err(ureq::Error::StatusCode(405) | ureq::Error::StatusCode(501)) => {
                let mut req = self.endpoint.agent.get(&url);
                for (name, value) in &headers {
                    req = req.header(name.as_str(), value.as_str());
                }
                match req.call() {
                    Ok(mut resp) => {
                        let _ = resp.body_mut().read_to_vec();
                        Ok(true)
                    }
                    Err(ureq::Error::StatusCode(404) | ureq::Error::StatusCode(410)) => Ok(false),
                    Err(e) => Err(Self::map_ureq_has_err(id, e)),
                }
            }
            Err(e) => Err(Self::map_ureq_has_err(id, e)),
        }
    }

    fn put(&self, id: &ChunkId, plain: &[u8]) -> Result<PutOutcome, SinkError> {
        if self.verify_hash {
            let actual = ChunkId::hash(plain);
            if actual != *id {
                return Err(SinkError::IdMismatch {
                    expected: *id,
                    actual,
                });
            }
        }

        if self.skip_if_exists && self.has(id)? {
            return Ok(PutOutcome::SkippedExists);
        }

        let url = self
            .endpoint
            .expand_url(id)
            .map_err(|e| Self::template_err(id, e))?;
        let headers = self
            .endpoint
            .expand_headers(id)
            .map_err(|e| Self::template_err(id, e))?;

        let mut req = match self.put_method {
            HttpPutMethod::Put => self.endpoint.agent.put(&url),
            HttpPutMethod::Post => self.endpoint.agent.post(&url),
        };
        req = req.header("Content-Type", self.content_type.as_str());
        for (name, value) in &headers {
            req = req.header(name.as_str(), value.as_str());
        }

        match req.send(plain) {
            Ok(_resp) => Ok(PutOutcome::Written),
            Err(ureq::Error::StatusCode(409)) if self.accept_conflict => {
                Ok(PutOutcome::SkippedExists)
            }
            Err(e) => Err(Self::map_ureq_err(id, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HttpChunkSource;
    use crate::layout::chunk_http_path;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use tiny_http::{Header, Method, Response, Server, StatusCode};

    #[derive(Default, Clone)]
    struct SeenPut {
        method: Option<String>,
        path: Option<String>,
        authorization: Option<String>,
        content_type: Option<String>,
        body: Option<Vec<u8>>,
        put_count: usize,
        head_count: usize,
    }

    /// In-memory PUT stub: stores bodies by path; HEAD reports presence; PUT
    /// returns 200 on first write and 409 on overwrite when `conflict_on_dup`.
    fn spawn_put_stub(
        seen: Arc<Mutex<SeenPut>>,
        store: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        conflict_on_dup: bool,
    ) -> (String, thread::JoinHandle<()>) {
        let server = Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let handle = thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let method = request.method().clone();
                let url = request.url().to_string();
                let path = url.split('?').next().unwrap_or(&url).to_string();

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
                let ct = request
                    .headers()
                    .iter()
                    .find(|h| {
                        h.field
                            .as_str()
                            .as_str()
                            .eq_ignore_ascii_case("Content-Type")
                    })
                    .map(|h| h.value.as_str().to_string());

                {
                    let mut g = seen.lock().unwrap();
                    g.method = Some(format!("{method:?}"));
                    g.path = Some(path.clone());
                    if auth.is_some() {
                        g.authorization = auth.clone();
                    }
                    if ct.is_some() {
                        g.content_type = ct.clone();
                    }
                    if method == Method::Head {
                        g.head_count += 1;
                    }
                }

                match method {
                    Method::Head => {
                        let exists = store.lock().unwrap().contains_key(&path);
                        let code = if exists { 200 } else { 404 };
                        let _ = request.respond(Response::empty(StatusCode(code)));
                    }
                    Method::Get => {
                        let data = store.lock().unwrap().get(&path).cloned();
                        if let Some(data) = data {
                            let _ = request.respond(Response::from_data(data));
                        } else {
                            let _ = request.respond(Response::empty(StatusCode(404)));
                        }
                    }
                    Method::Put | Method::Post => {
                        let mut body = Vec::new();
                        let _ = request.as_reader().read_to_end(&mut body);
                        {
                            let mut g = seen.lock().unwrap();
                            g.body = Some(body.clone());
                            g.put_count += 1;
                        }
                        let mut map = store.lock().unwrap();
                        if map.contains_key(&path) && conflict_on_dup {
                            drop(map);
                            let _ = request.respond(Response::empty(StatusCode(409)));
                        } else {
                            map.insert(path, body);
                            drop(map);
                            let _ = request.respond(Response::empty(StatusCode(200)).with_header(
                                Header::from_bytes(&b"Content-Length"[..], "0").unwrap(),
                            ));
                        }
                    }
                    _ => {
                        let _ = request.respond(Response::empty(StatusCode(405)));
                    }
                }
            }
        });
        thread::sleep(Duration::from_millis(20));
        (base, handle)
    }

    #[test]
    fn put_asserts_path_header_body_and_blake3() {
        let seen: Arc<Mutex<SeenPut>> = Arc::new(Mutex::new(SeenPut::default()));
        let store: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
        let (base, _handle) = spawn_put_stub(Arc::clone(&seen), Arc::clone(&store), false);

        let plain = b"phase4-m2-put-body";
        let id = ChunkId::hash(plain);
        let hex = id.to_hex();

        let var = "CHUNKFORGE_P4_M2_PUT_TOKEN";
        unsafe { std::env::set_var(var, "put-tok-m2") };

        let sink = HttpChunkSink::builder(&base)
            .url_template("{base}/{prefix}{path}")
            .prefix("data/")
            .header("Authorization", "Bearer {env:CHUNKFORGE_P4_M2_PUT_TOKEN}")
            .timeout(Some(Duration::from_secs(5)))
            .build()
            .unwrap();

        let expected_url = format!(
            "{}/data/chunks/{}/{}.cnk",
            base.trim_end_matches('/'),
            &hex[..2],
            &hex[2..]
        );
        assert_eq!(sink.url_for(&id), expected_url);

        let outcome = sink.put(&id, plain).unwrap();
        assert_eq!(outcome, PutOutcome::Written);

        let g = seen.lock().unwrap();
        assert_eq!(g.method.as_deref(), Some("Put"));
        let path = g.path.as_deref().expect("path");
        assert!(
            path.contains(&format!("/data/chunks/{}/{}.cnk", &hex[..2], &hex[2..])),
            "PUT path mismatch: {path}"
        );
        assert_eq!(g.authorization.as_deref(), Some("Bearer put-tok-m2"));
        assert_eq!(g.content_type.as_deref(), Some("application/octet-stream"));
        let body = g.body.as_ref().expect("body");
        assert_eq!(body.as_slice(), plain);
        assert_eq!(ChunkId::hash(body), id);

        unsafe { std::env::remove_var(var) };
    }

    #[test]
    fn put_path_matches_http_chunk_source_url_for() {
        let seen: Arc<Mutex<SeenPut>> = Arc::new(Mutex::new(SeenPut::default()));
        let store: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
        let (base, _handle) = spawn_put_stub(Arc::clone(&seen), store, false);

        // Fixed id so path is deterministic; body must hash to this id for verify.
        // Use real hash id instead of forced bytes so verify_hash passes.
        let plain = b"phase4-m2-isomorphic-url";
        let id = ChunkId::hash(plain);

        let sink = HttpChunkSink::builder(&base)
            .url_template("{base}/{prefix}{path}")
            .prefix("data/")
            .timeout(Some(Duration::from_secs(5)))
            .build()
            .unwrap();
        let src = HttpChunkSource::builder(&base)
            .url_template("{base}/{prefix}{path}")
            .prefix("data/")
            .timeout(Some(Duration::from_secs(5)))
            .build()
            .unwrap();

        assert_eq!(sink.url_for(&id), src.url_for(&id));
        assert_eq!(
            sink.url_for(&id),
            format!(
                "{}/data/{}",
                base.trim_end_matches('/'),
                chunk_http_path(&id)
            )
        );

        sink.put(&id, plain).unwrap();

        let g = seen.lock().unwrap();
        let path = g.path.as_deref().expect("path");
        // Request path is the URL path component of url_for.
        let expected_path = {
            let url = src.url_for(&id);
            let after_scheme = url.split("://").nth(1).unwrap();
            let path_part = after_scheme.find('/').map(|i| &after_scheme[i..]).unwrap();
            path_part.to_string()
        };
        assert_eq!(path, expected_path.as_str());
    }

    #[test]
    fn default_template_put_path_matches_source_url_for() {
        let seen: Arc<Mutex<SeenPut>> = Arc::new(Mutex::new(SeenPut::default()));
        let store: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
        let (base, _handle) = spawn_put_stub(Arc::clone(&seen), store, false);

        let plain = b"phase4-m2-default-layout";
        let id = ChunkId::hash(plain);

        let sink = HttpChunkSink::builder(&base)
            .timeout(Some(Duration::from_secs(5)))
            .build()
            .unwrap();
        let src = HttpChunkSource::builder(&base)
            .timeout(Some(Duration::from_secs(5)))
            .build()
            .unwrap();

        assert_eq!(sink.url_for(&id), src.url_for(&id));
        assert_eq!(sink.url_template(), DEFAULT_URL_TEMPLATE);
        assert_eq!(src.url_template(), DEFAULT_URL_TEMPLATE);

        assert_eq!(sink.put(&id, plain).unwrap(), PutOutcome::Written);
        let path = seen.lock().unwrap().path.clone().unwrap();
        let expected_path = {
            let url = src.url_for(&id);
            let after_scheme = url.split("://").nth(1).unwrap();
            after_scheme
                .find('/')
                .map(|i| &after_scheme[i..])
                .unwrap()
                .to_string()
        };
        assert_eq!(path, expected_path);
    }

    #[test]
    fn repeated_put_is_idempotent_skipped_exists() {
        let seen: Arc<Mutex<SeenPut>> = Arc::new(Mutex::new(SeenPut::default()));
        let store: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
        let (base, _handle) = spawn_put_stub(Arc::clone(&seen), Arc::clone(&store), true);

        let plain = b"phase4-m2-idempotent";
        let id = ChunkId::hash(plain);

        let sink = HttpChunkSink::builder(&base)
            .timeout(Some(Duration::from_secs(5)))
            .build()
            .unwrap();

        assert_eq!(sink.put(&id, plain).unwrap(), PutOutcome::Written);
        assert_eq!(seen.lock().unwrap().put_count, 1);

        // Second put: HEAD finds object → SkippedExists, no second PUT.
        assert_eq!(sink.put(&id, plain).unwrap(), PutOutcome::SkippedExists);
        assert_eq!(seen.lock().unwrap().put_count, 1);
        assert!(seen.lock().unwrap().head_count >= 1);
        assert_eq!(store.lock().unwrap().len(), 1);
    }

    #[test]
    fn put_without_skip_treats_409_as_skipped() {
        let seen: Arc<Mutex<SeenPut>> = Arc::new(Mutex::new(SeenPut::default()));
        let store: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
        let (base, _handle) = spawn_put_stub(Arc::clone(&seen), store, true);

        let plain = b"phase4-m2-conflict-409";
        let id = ChunkId::hash(plain);

        let sink = HttpChunkSink::builder(&base)
            .skip_if_exists(false)
            .accept_conflict(true)
            .timeout(Some(Duration::from_secs(5)))
            .build()
            .unwrap();

        assert_eq!(sink.put(&id, plain).unwrap(), PutOutcome::Written);
        assert_eq!(sink.put(&id, plain).unwrap(), PutOutcome::SkippedExists);
        assert_eq!(seen.lock().unwrap().put_count, 2);
    }

    #[test]
    fn put_rejects_id_mismatch() {
        let sink = HttpChunkSink::builder("http://127.0.0.1:9")
            .timeout(Some(Duration::from_millis(50)))
            .build()
            .unwrap();
        let wrong = ChunkId::hash(b"expected");
        let err = sink.put(&wrong, b"actual-bytes").unwrap_err();
        assert!(
            matches!(
                err,
                SinkError::IdMismatch { expected, actual }
                if expected == wrong && actual == ChunkId::hash(b"actual-bytes")
            ),
            "{err:?}"
        );
    }

    #[test]
    fn unknown_placeholder_fails_at_build() {
        let err = HttpChunkSink::builder("http://127.0.0.1:9")
            .url_template("{base}/{bucket}/{path}")
            .build()
            .unwrap_err();
        assert_eq!(err, TemplateError::UnknownPlaceholder("bucket".into()));
    }
}
