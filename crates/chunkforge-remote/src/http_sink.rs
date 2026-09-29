//! HTTP chunk-directory [`ChunkSink`] (PUT), isomorphic with [`crate::HttpChunkSource`].

use crate::endpoint::HttpEndpoint;
use crate::http::{
    DEFAULT_URL_TEMPLATE, HttpBuildError, check_sigv4_auth_conflict, headers_with_sigv4,
};
use crate::retry::{Attempt, RetryPolicy, run_with_retry, ureq_error_is_transient};
use crate::sigv4::SigV4Signer;
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
    retry_policy: RetryPolicy,
    sigv4: Option<SigV4Signer>,
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
            retry_policy: RetryPolicy::default(),
            sigv4: None,
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

    /// Retry policy for transient HTTP failures (default: 0 extra attempts).
    pub fn retry_policy(&self) -> &RetryPolicy {
        &self.retry_policy
    }

    /// Optional SigV4 signer (default `None` ≡ no SigV4 headers).
    pub fn sigv4(&self) -> Option<&SigV4Signer> {
        self.sigv4.as_ref()
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
    retry_policy: RetryPolicy,
    sigv4: Option<SigV4Signer>,
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

    /// Set the retry policy for transient HTTP failures (default: 0 extra attempts).
    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Enable AWS SigV4 signing for PUT/HEAD/GET probes (default off).
    pub fn aws_sigv4(mut self, signer: SigV4Signer) -> Self {
        self.sigv4 = Some(signer);
        self
    }

    /// Clear any SigV4 signer (explicit off).
    pub fn clear_aws_sigv4(mut self) -> Self {
        self.sigv4 = None;
        self
    }

    /// Build the sink (validates templates against an all-zero [`ChunkId`]).
    ///
    /// SigV4 + Authorization header template → [`HttpBuildError::AuthorizationConflict`].
    pub fn build(self) -> Result<HttpChunkSink, HttpBuildError> {
        check_sigv4_auth_conflict(&self.header_templates, &self.sigv4)?;
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
            retry_policy: self.retry_policy,
            sigv4: self.sigv4,
        })
    }
}

impl ChunkSink for HttpChunkSink {
    fn has(&self, id: &ChunkId) -> Result<bool, SinkError> {
        let url = self
            .endpoint
            .expand_url(id)
            .map_err(|e| Self::template_err(id, e))?;
        let user_headers = self
            .endpoint
            .expand_headers(id)
            .map_err(|e| Self::template_err(id, e))?;
        let headers =
            headers_with_sigv4(self.sigv4.as_ref(), "HEAD", &url, &user_headers, b"", &[])
                .map_err(|e| SinkError::Backend(format!("SigV4 sign HEAD for {id}: {e}")))?;

        run_with_retry(&self.retry_policy, || self.has_once(id, &url, &headers))
    }

    fn put(&self, id: &ChunkId, plain: &[u8]) -> Result<PutOutcome, SinkError> {
        // Hash mismatch is a local Corrupt-equivalent — never retried.
        if self.verify_hash {
            let actual = ChunkId::hash(plain);
            if actual != *id {
                return Err(SinkError::IdMismatch {
                    expected: *id,
                    actual,
                });
            }
        }

        let url = self
            .endpoint
            .expand_url(id)
            .map_err(|e| Self::template_err(id, e))?;
        let user_headers = self
            .endpoint
            .expand_headers(id)
            .map_err(|e| Self::template_err(id, e))?;

        // Sign PUT with body hash + Content-Type (set on the wire below).
        let headers = headers_with_sigv4(
            self.sigv4.as_ref(),
            match self.put_method {
                HttpPutMethod::Put => "PUT",
                HttpPutMethod::Post => "POST",
            },
            &url,
            &user_headers,
            plain,
            &[("Content-Type", self.content_type.as_str())],
        )
        .map_err(|e| SinkError::Backend(format!("SigV4 sign PUT for {id}: {e}")))?;

        run_with_retry(&self.retry_policy, || {
            // Re-check presence before each PUT attempt when skip_if_exists
            // (idempotent under concurrent writers / prior partial success).
            if self.skip_if_exists {
                match self.has(id) {
                    Ok(true) => return Attempt::Ok(PutOutcome::SkippedExists),
                    Ok(false) => {}
                    Err(e) => return Attempt::Fatal(e),
                }
            }
            self.put_once(id, plain, &url, &headers)
        })
    }
}

impl HttpChunkSink {
    fn classify_has(id: &ChunkId, err: ureq::Error) -> Attempt<bool, SinkError> {
        let transient = ureq_error_is_transient(&err);
        let mapped = Self::map_ureq_has_err(id, err);
        if transient {
            Attempt::Transient(mapped)
        } else {
            Attempt::Fatal(mapped)
        }
    }

    fn classify_put(id: &ChunkId, err: ureq::Error) -> Attempt<PutOutcome, SinkError> {
        let transient = ureq_error_is_transient(&err);
        let mapped = Self::map_ureq_err(id, err);
        if transient {
            Attempt::Transient(mapped)
        } else {
            Attempt::Fatal(mapped)
        }
    }

    fn has_once(
        &self,
        id: &ChunkId,
        url: &str,
        headers: &[(String, String)],
    ) -> Attempt<bool, SinkError> {
        let mut req = self.endpoint.agent.head(url);
        for (name, value) in headers {
            req = req.header(name.as_str(), value.as_str());
        }

        match req.call() {
            Ok(_resp) => Attempt::Ok(true),
            Err(ureq::Error::StatusCode(404) | ureq::Error::StatusCode(410)) => Attempt::Ok(false),
            Err(ureq::Error::StatusCode(405) | ureq::Error::StatusCode(501)) => {
                let user_only: Vec<(String, String)> = headers
                    .iter()
                    .filter(|(n, _)| {
                        let l = n.to_ascii_lowercase();
                        l != "authorization"
                            && l != "x-amz-date"
                            && l != "x-amz-content-sha256"
                            && l != "x-amz-security-token"
                    })
                    .cloned()
                    .collect();
                let get_headers =
                    match headers_with_sigv4(self.sigv4.as_ref(), "GET", url, &user_only, b"", &[])
                    {
                        Ok(h) => h,
                        Err(e) => {
                            return Attempt::Fatal(SinkError::Backend(format!(
                                "SigV4 sign GET fallback for {id}: {e}"
                            )));
                        }
                    };
                let mut req = self.endpoint.agent.get(url);
                for (name, value) in &get_headers {
                    req = req.header(name.as_str(), value.as_str());
                }
                match req.call() {
                    Ok(mut resp) => {
                        let _ = resp.body_mut().read_to_vec();
                        Attempt::Ok(true)
                    }
                    Err(ureq::Error::StatusCode(404) | ureq::Error::StatusCode(410)) => {
                        Attempt::Ok(false)
                    }
                    Err(e) => Self::classify_has(id, e),
                }
            }
            Err(e) => Self::classify_has(id, e),
        }
    }

    fn put_once(
        &self,
        id: &ChunkId,
        plain: &[u8],
        url: &str,
        headers: &[(String, String)],
    ) -> Attempt<PutOutcome, SinkError> {
        let mut req = match self.put_method {
            HttpPutMethod::Put => self.endpoint.agent.put(url),
            HttpPutMethod::Post => self.endpoint.agent.post(url),
        };
        req = req.header("Content-Type", self.content_type.as_str());
        for (name, value) in headers {
            req = req.header(name.as_str(), value.as_str());
        }

        match req.send(plain) {
            Ok(_resp) => Attempt::Ok(PutOutcome::Written),
            Err(ureq::Error::StatusCode(409)) if self.accept_conflict => {
                Attempt::Ok(PutOutcome::SkippedExists)
            }
            Err(e) => Self::classify_put(id, e),
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
        assert_eq!(
            err,
            HttpBuildError::Template(TemplateError::UnknownPlaceholder("bucket".into()))
        );
    }

    /// Phase8-M1: 2×503 then 200 on PUT → success; PUT attempts == 3 (max_retries=2).
    #[test]
    fn put_retries_transient_503_then_succeeds() {
        let attempts: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
        let attempts2 = Arc::clone(&attempts);

        let server = Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let _handle = thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let method = request.method().clone();
                if method == Method::Head {
                    // skip_if_exists probe → always missing so PUT proceeds.
                    let _ = request.respond(Response::empty(StatusCode(404)));
                    continue;
                }
                if method == Method::Put || method == Method::Post {
                    let mut body = Vec::new();
                    let _ = request.as_reader().read_to_end(&mut body);
                    let n = {
                        let mut g = attempts2.lock().unwrap();
                        *g += 1;
                        *g
                    };
                    if n <= 2 {
                        let _ = request.respond(Response::empty(StatusCode(503)));
                    } else {
                        let _ = request.respond(Response::empty(StatusCode(200)));
                    }
                } else {
                    let _ = request.respond(Response::empty(StatusCode(405)));
                }
            }
        });
        thread::sleep(Duration::from_millis(20));

        let plain = b"phase8-m1-retry-put-body";
        let id = ChunkId::hash(plain);
        let policy = RetryPolicy {
            max_retries: 2,
            base_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
        };
        let sink = HttpChunkSink::builder(&base)
            .retry_policy(policy)
            .timeout(Some(Duration::from_secs(5)))
            .build()
            .unwrap();

        assert_eq!(sink.put(&id, plain).unwrap(), PutOutcome::Written);
        assert_eq!(*attempts.lock().unwrap(), 3);
    }

    /// Phase8-M1: 404 on PUT (unexpected) is permanent-ish 4xx — but 404 is not
    /// transient per policy, so exactly 1 PUT attempt. More importantly: a GET/has
    /// 404 path is covered on Source; here we assert 403 is not retried on PUT.
    #[test]
    fn put_403_is_not_retried() {
        let attempts: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
        let attempts2 = Arc::clone(&attempts);

        let server = Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let _handle = thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let method = request.method().clone();
                if method == Method::Head {
                    let _ = request.respond(Response::empty(StatusCode(404)));
                    continue;
                }
                if method == Method::Put || method == Method::Post {
                    let mut body = Vec::new();
                    let _ = request.as_reader().read_to_end(&mut body);
                    *attempts2.lock().unwrap() += 1;
                    let _ = request.respond(Response::empty(StatusCode(403)));
                } else {
                    let _ = request.respond(Response::empty(StatusCode(405)));
                }
            }
        });
        thread::sleep(Duration::from_millis(20));

        let plain = b"phase8-m1-put-forbidden";
        let id = ChunkId::hash(plain);
        let policy = RetryPolicy {
            max_retries: 5,
            base_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
        };
        let sink = HttpChunkSink::builder(&base)
            .retry_policy(policy)
            .timeout(Some(Duration::from_secs(5)))
            .build()
            .unwrap();

        let err = sink.put(&id, plain).unwrap_err();
        assert!(
            matches!(err, SinkError::Backend(ref s) if s.contains("403")),
            "{err:?}"
        );
        assert_eq!(*attempts.lock().unwrap(), 1);
    }

    /// max_retries=0 + 503 → fail after 1 PUT attempt.
    #[test]
    fn put_503_with_zero_retries_fails_once() {
        let attempts: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
        let attempts2 = Arc::clone(&attempts);

        let server = Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let _handle = thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let method = request.method().clone();
                if method == Method::Head {
                    let _ = request.respond(Response::empty(StatusCode(404)));
                    continue;
                }
                if method == Method::Put || method == Method::Post {
                    let mut body = Vec::new();
                    let _ = request.as_reader().read_to_end(&mut body);
                    *attempts2.lock().unwrap() += 1;
                    let _ = request.respond(Response::empty(StatusCode(503)));
                } else {
                    let _ = request.respond(Response::empty(StatusCode(405)));
                }
            }
        });
        thread::sleep(Duration::from_millis(20));

        let plain = b"phase8-m1-put-no-retry";
        let id = ChunkId::hash(plain);
        let sink = HttpChunkSink::builder(&base)
            .timeout(Some(Duration::from_secs(5)))
            .build()
            .unwrap();
        assert_eq!(sink.retry_policy().max_retries, 0);

        let err = sink.put(&id, plain).unwrap_err();
        assert!(
            matches!(err, SinkError::Backend(ref s) if s.contains("503")),
            "{err:?}"
        );
        assert_eq!(*attempts.lock().unwrap(), 1);
    }

    /// Phase8-M6: PUT with SigV4 → Authorization + x-amz-date on the wire.
    #[test]
    fn put_with_sigv4_sends_authorization_and_amz_date() {
        let seen: Arc<Mutex<SeenPut>> = Arc::new(Mutex::new(SeenPut::default()));
        let store: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
        // Extend stub to capture x-amz-date
        let server = Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let seen2 = Arc::clone(&seen);
        let store2 = Arc::clone(&store);
        let amz_date: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let amz2 = Arc::clone(&amz_date);
        let _handle = thread::spawn(move || {
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
                let date = request
                    .headers()
                    .iter()
                    .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case("x-amz-date"))
                    .map(|h| h.value.as_str().to_string());
                if date.is_some() {
                    *amz2.lock().unwrap() = date;
                }
                {
                    let mut g = seen2.lock().unwrap();
                    g.method = Some(format!("{method:?}"));
                    g.path = Some(path.clone());
                    if auth.is_some() {
                        g.authorization = auth;
                    }
                    if method == Method::Head {
                        g.head_count += 1;
                    }
                }
                match method {
                    Method::Head => {
                        let exists = store2.lock().unwrap().contains_key(&path);
                        let _ = request.respond(Response::empty(StatusCode(if exists {
                            200
                        } else {
                            404
                        })));
                    }
                    Method::Put | Method::Post => {
                        let mut body = Vec::new();
                        let _ = request.as_reader().read_to_end(&mut body);
                        {
                            let mut g = seen2.lock().unwrap();
                            g.body = Some(body.clone());
                            g.put_count += 1;
                        }
                        store2.lock().unwrap().insert(path, body);
                        let _ = request.respond(Response::empty(StatusCode(200)));
                    }
                    _ => {
                        let _ = request.respond(Response::empty(StatusCode(405)));
                    }
                }
            }
        });
        thread::sleep(Duration::from_millis(20));

        let plain = b"phase8-m6-sigv4-put";
        let id = ChunkId::hash(plain);
        let creds = crate::AwsCredentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        let signer = crate::SigV4Signer::new(crate::SigV4Config::new(creds, "us-east-1", "s3"))
            .with_clock(crate::SigningClock::Fixed("20130524T000000Z".into()));
        let sink = HttpChunkSink::builder(&base)
            .aws_sigv4(signer)
            .timeout(Some(Duration::from_secs(5)))
            .build()
            .unwrap();
        assert!(sink.sigv4().is_some());
        assert_eq!(sink.put(&id, plain).unwrap(), PutOutcome::Written);

        let g = seen.lock().unwrap();
        let auth = g.authorization.as_deref().expect("Authorization on PUT");
        assert!(
            auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/"),
            "got {auth}"
        );
        assert!(auth.contains("Signature="));
        assert_eq!(
            amz_date.lock().unwrap().as_deref(),
            Some("20130524T000000Z")
        );
        assert_eq!(
            g.body.as_deref(),
            Some(plain.as_slice())
        );
    }

    /// Phase8-M6: no SigV4 → PUT has no Authorization / x-amz-date.
    #[test]
    fn put_without_sigv4_sends_no_amz_headers() {
        let seen: Arc<Mutex<SeenPut>> = Arc::new(Mutex::new(SeenPut::default()));
        let store: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
        let (base, _handle) = spawn_put_stub(Arc::clone(&seen), store, false);
        let plain = b"phase8-m6-put-plain";
        let id = ChunkId::hash(plain);
        let sink = HttpChunkSink::builder(&base)
            .timeout(Some(Duration::from_secs(5)))
            .build()
            .unwrap();
        assert!(sink.sigv4().is_none());
        assert_eq!(sink.put(&id, plain).unwrap(), PutOutcome::Written);
        assert!(seen.lock().unwrap().authorization.is_none());
    }
}
