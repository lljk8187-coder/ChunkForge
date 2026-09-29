//! Minimal in-process AWS Signature Version 4 (AWS4-HMAC-SHA256).
//!
//! Signs GET / HEAD / PUT for S3-compatible endpoints using env-style
//! credentials. Payload hash is always `hex(SHA256(body))` (empty body =
//! SHA256 of empty bytes — never `UNSIGNED-PAYLOAD`). No chunked/streaming
//! signing, no aws-sdk, no credential provider chain beyond what the caller
//! supplies.
//!
//! See `docs/sigv4.md` and `docs/remote-layout.md`.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// Empty-payload SHA-256 (hex) — `SHA256([])`.
pub const EMPTY_PAYLOAD_HASH: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Default region when `AWS_REGION` is unset.
pub const DEFAULT_REGION: &str = "us-east-1";

/// Default service name for object-store signing.
pub const DEFAULT_SERVICE: &str = "s3";

/// Errors from SigV4 configuration or signing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigV4Error {
    /// `AWS_ACCESS_KEY_ID` missing or empty.
    MissingAccessKey,
    /// `AWS_SECRET_ACCESS_KEY` missing or empty.
    MissingSecretKey,
    /// URL could not be parsed into scheme/host/path.
    InvalidUrl(String),
    /// Internal HMAC failure (should not happen with SHA-256).
    Crypto(String),
}

impl fmt::Display for SigV4Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingAccessKey => {
                write!(
                    f,
                    "--aws-sigv4 requires AWS_ACCESS_KEY_ID (env); set it or omit --aws-sigv4"
                )
            }
            Self::MissingSecretKey => {
                write!(
                    f,
                    "--aws-sigv4 requires AWS_SECRET_ACCESS_KEY (env); set it or omit --aws-sigv4"
                )
            }
            Self::InvalidUrl(s) => write!(f, "SigV4: invalid URL: {s}"),
            Self::Crypto(s) => write!(f, "SigV4 crypto error: {s}"),
        }
    }
}

impl std::error::Error for SigV4Error {}

/// AWS access credentials (env-sourced at the CLI; constructed explicitly in lib).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

impl AwsCredentials {
    /// Load from `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` /
    /// optional `AWS_SESSION_TOKEN`. Does **not** consult shared credentials
    /// files, IMDS, SSO, or any other provider.
    pub fn from_env() -> Result<Self, SigV4Error> {
        let access_key_id = std::env::var("AWS_ACCESS_KEY_ID")
            .map_err(|_| SigV4Error::MissingAccessKey)?
            .trim()
            .to_string();
        if access_key_id.is_empty() {
            return Err(SigV4Error::MissingAccessKey);
        }
        let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY")
            .map_err(|_| SigV4Error::MissingSecretKey)?
            .trim()
            .to_string();
        if secret_access_key.is_empty() {
            return Err(SigV4Error::MissingSecretKey);
        }
        let session_token = std::env::var("AWS_SESSION_TOKEN")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Ok(Self {
            access_key_id,
            secret_access_key,
            session_token,
        })
    }
}

/// SigV4 signing configuration for S3-compatible GET/HEAD/PUT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigV4Config {
    pub credentials: AwsCredentials,
    /// AWS region (default [`DEFAULT_REGION`]).
    pub region: String,
    /// Service name (default [`DEFAULT_SERVICE`] = `"s3"`).
    pub service: String,
}

impl SigV4Config {
    /// Build config from env credentials + optional `AWS_REGION`.
    ///
    /// Returns `(config, region_was_default)` so the CLI can warn when the
    /// region fell back to [`DEFAULT_REGION`].
    pub fn from_env() -> Result<(Self, bool), SigV4Error> {
        let credentials = AwsCredentials::from_env()?;
        let (region, was_default) = match std::env::var("AWS_REGION") {
            Ok(r) => {
                let t = r.trim().to_string();
                if t.is_empty() {
                    (DEFAULT_REGION.to_string(), true)
                } else {
                    (t, false)
                }
            }
            Err(_) => (DEFAULT_REGION.to_string(), true),
        };
        Ok((
            Self {
                credentials,
                region,
                service: DEFAULT_SERVICE.to_string(),
            },
            was_default,
        ))
    }

    /// Explicit constructor (tests / library callers).
    pub fn new(
        credentials: AwsCredentials,
        region: impl Into<String>,
        service: impl Into<String>,
    ) -> Self {
        Self {
            credentials,
            region: region.into(),
            service: service.into(),
        }
    }
}

/// Clock source for `x-amz-date` / credential scope date.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SigningClock {
    /// `SystemTime::now()` formatted as `YYYYMMDD'T'HHMMSS'Z'`.
    #[default]
    System,
    /// Fixed timestamp for golden-vector tests (`20130524T000000Z`).
    Fixed(String),
}

impl SigningClock {
    pub fn amz_date(&self) -> String {
        match self {
            Self::System => system_amz_date(),
            Self::Fixed(s) => s.clone(),
        }
    }
}

fn system_amz_date() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Format UTC without external crates (strftime not in std).
    let (y, mo, d, h, mi, s) = secs_to_utc_ymdhms(secs);
    format!("{y:04}{mo:02}{d:02}T{h:02}{mi:02}{s:02}Z")
}

/// Civil UTC date/time from Unix seconds (proleptic Gregorian; adequate for signing).
fn secs_to_utc_ymdhms(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let h = rem / 3600;
    let mi = (rem % 3600) / 60;
    let s = rem % 60;
    // Days since 1970-01-01 → year/month/day (Howard Hinnant algorithm).
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as u64, m, d, h, mi, s)
}

/// Headers produced by signing (to attach to the outgoing request).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedHeaders {
    pub authorization: String,
    pub amz_date: String,
    pub content_sha256: String,
    pub session_token: Option<String>,
}

impl SignedHeaders {
    /// Name/value pairs to set on the HTTP request (Authorization last).
    pub fn as_pairs(&self) -> Vec<(String, String)> {
        let mut out = Vec::with_capacity(4);
        out.push(("x-amz-date".to_string(), self.amz_date.clone()));
        out.push((
            "x-amz-content-sha256".to_string(),
            self.content_sha256.clone(),
        ));
        if let Some(ref tok) = self.session_token {
            out.push(("x-amz-security-token".to_string(), tok.clone()));
        }
        out.push(("Authorization".to_string(), self.authorization.clone()));
        out
    }
}

/// Hex-encode SHA-256 of `data`.
pub fn payload_hash(data: &[u8]) -> String {
    let dig = Sha256::digest(data);
    hex_encode(&dig)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<Vec<u8>, SigV4Error> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|e| SigV4Error::Crypto(e.to_string()))?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn sha256_hex(data: &[u8]) -> String {
    hex_encode(&Sha256::digest(data))
}

/// AWS SigV4 URI encode (RFC 3986 unreserved stay literal).
/// When `encode_slash` is false, `/` is preserved (canonical URI path).
pub fn uri_encode(s: &str, encode_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            b'/' if !encode_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Canonical URI: encode each path segment, keep `/` separators. Empty → `/`.
pub fn canonical_uri(path: &str) -> String {
    if path.is_empty() || path == "/" {
        return "/".to_string();
    }
    let leading = path.starts_with('/');
    let encoded = path
        .split('/')
        .map(|seg| uri_encode(seg, true))
        .collect::<Vec<_>>()
        .join("/");
    if leading && !encoded.starts_with('/') {
        format!("/{encoded}")
    } else {
        encoded
    }
}

/// Canonical query string: sort by URI-encoded key, then value; `k=v` joined by `&`.
pub fn canonical_query(query: &str) -> String {
    if query.is_empty() {
        return String::new();
    }
    let mut pairs: Vec<(String, String)> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (uri_encode(k, true), uri_encode(v, true)),
            None => (uri_encode(p, true), String::new()),
        })
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// Derive the signing key for the given date (YYYYMMDD), region, service.
pub fn signing_key(
    secret: &str,
    date_stamp: &str,
    region: &str,
    service: &str,
) -> Result<Vec<u8>, SigV4Error> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date_stamp.as_bytes())?;
    let k_region = hmac_sha256(&k_date, region.as_bytes())?;
    let k_service = hmac_sha256(&k_region, service.as_bytes())?;
    hmac_sha256(&k_service, b"aws4_request")
}

/// Low-level sign: produce Authorization + amz headers.
///
/// `extra_headers` are additional headers already present on the request
/// (e.g. `Range`, `Content-Type`, `Date`) — names are matched case-insensitively;
/// `host` / `x-amz-*` signing headers are added by this function and should not
/// be duplicated in `extra_headers`.
#[allow(clippy::too_many_arguments)] // mirrors the AWS SigV4 request surface
pub fn sign_request(
    method: &str,
    host: &str,
    path: &str,
    query: &str,
    extra_headers: &[(&str, &str)],
    body: &[u8],
    config: &SigV4Config,
    amz_date: &str,
) -> Result<SignedHeaders, SigV4Error> {
    let date_stamp = if amz_date.len() >= 8 {
        &amz_date[..8]
    } else {
        amz_date
    };
    let content_sha256 = payload_hash(body);

    // Collect headers to sign: host + x-amz-* + extras.
    let mut hdrs: Vec<(String, String)> = Vec::new();
    hdrs.push(("host".to_string(), host.to_string()));
    hdrs.push(("x-amz-content-sha256".to_string(), content_sha256.clone()));
    hdrs.push(("x-amz-date".to_string(), amz_date.to_string()));
    if let Some(ref tok) = config.credentials.session_token {
        hdrs.push(("x-amz-security-token".to_string(), tok.clone()));
    }
    for (name, value) in extra_headers {
        let lower = name.to_ascii_lowercase();
        // Skip headers we already inject / Authorization (never double-sign).
        if lower == "host"
            || lower == "authorization"
            || lower == "x-amz-date"
            || lower == "x-amz-content-sha256"
            || lower == "x-amz-security-token"
        {
            continue;
        }
        hdrs.push((lower, value.trim().to_string()));
    }
    hdrs.sort_by(|a, b| a.0.cmp(&b.0));
    // Dedup by name keeping first (sorted unique).
    hdrs.dedup_by(|a, b| a.0 == b.0);

    let signed_headers = hdrs
        .iter()
        .map(|(n, _)| n.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let canonical_headers = hdrs
        .iter()
        .map(|(n, v)| format!("{n}:{}\n", v.trim()))
        .collect::<String>();

    let can_uri = canonical_uri(path);
    let can_query = canonical_query(query);
    let canonical_request = format!(
        "{method}\n{can_uri}\n{can_query}\n{canonical_headers}\n{signed_headers}\n{content_sha256}"
    );
    let hashed_request = sha256_hex(canonical_request.as_bytes());
    let scope = format!(
        "{date_stamp}/{}/{}/aws4_request",
        config.region, config.service
    );
    let string_to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{hashed_request}");

    let key = signing_key(
        &config.credentials.secret_access_key,
        date_stamp,
        &config.region,
        &config.service,
    )?;
    let sig = hex_encode(&hmac_sha256(&key, string_to_sign.as_bytes())?);

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope},SignedHeaders={signed_headers},Signature={sig}",
        config.credentials.access_key_id
    );

    Ok(SignedHeaders {
        authorization,
        amz_date: amz_date.to_string(),
        content_sha256,
        session_token: config.credentials.session_token.clone(),
    })
}

/// Parsed pieces of an HTTP(S) URL needed for signing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlParts {
    pub host_header: String,
    pub path: String,
    pub query: String,
}

/// Parse `http(s)://host[:port]/path[?query]` into signing parts.
///
/// Port is included in `host_header` only when non-default (not 80/443).
pub fn parse_url_for_signing(url: &str) -> Result<UrlParts, SigV4Error> {
    let (is_https, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(SigV4Error::InvalidUrl(format!(
            "expected http(s):// URL, got {url:?}"
        )));
    };

    let (authority, path_query) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return Err(SigV4Error::InvalidUrl(format!("missing host in {url:?}")));
    }

    let (path, query) = match path_query.split_once('?') {
        Some((p, q)) => (p, q),
        None => (path_query, ""),
    };
    let path = if path.is_empty() { "/" } else { path };

    let host_header = strip_default_port(authority, is_https);

    Ok(UrlParts {
        host_header,
        path: path.to_string(),
        query: query.to_string(),
    })
}

fn strip_default_port(authority: &str, is_https: bool) -> String {
    // IPv6 in brackets: [唯::1]:port — keep simple; our demos use IPv4/hostname.
    if let Some((host, port)) = authority.rsplit_once(':') {
        // Avoid splitting IPv6 without brackets (no colon-port form we care about
        // for hostnames / IPv4).
        if host.contains(']') {
            // [::1]:443 form
            let default = if is_https { "443" } else { "80" };
            if port == default {
                return host.to_string();
            }
            return authority.to_string();
        }
        if host.chars().all(|c| c.is_ascii_digit() || c == '.')
            || host
                .chars()
                .any(|c| c.is_ascii_alphabetic() || c == '-' || c == '.')
        {
            // hostname or IPv4 with port
            if !port.chars().all(|c| c.is_ascii_digit()) {
                return authority.to_string();
            }
            let default = if is_https { "443" } else { "80" };
            if port == default {
                return host.to_string();
            }
            return authority.to_string();
        }
    }
    authority.to_string()
}

/// Sign a full request URL with optional extra headers and body.
pub fn sign_url(
    method: &str,
    url: &str,
    extra_headers: &[(&str, &str)],
    body: &[u8],
    config: &SigV4Config,
    clock: &SigningClock,
) -> Result<SignedHeaders, SigV4Error> {
    let parts = parse_url_for_signing(url)?;
    let amz_date = clock.amz_date();
    sign_request(
        method,
        &parts.host_header,
        &parts.path,
        &parts.query,
        extra_headers,
        body,
        config,
        &amz_date,
    )
}

/// Active signer held by HttpChunkSource / HttpChunkSink when enabled.
#[derive(Debug, Clone)]
pub struct SigV4Signer {
    pub config: SigV4Config,
    pub clock: SigningClock,
}

impl SigV4Signer {
    pub fn new(config: SigV4Config) -> Self {
        Self {
            config,
            clock: SigningClock::System,
        }
    }

    pub fn with_clock(mut self, clock: SigningClock) -> Self {
        self.clock = clock;
        self
    }

    pub fn sign(
        &self,
        method: &str,
        url: &str,
        extra_headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<SignedHeaders, SigV4Error> {
        sign_url(method, url, extra_headers, body, &self.config, &self.clock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AWS docs GET Object golden vector
    /// <https://docs.aws.amazon.com/AmazonS3/latest/API/sig-v4-header-based-auth.html>
    #[test]
    fn golden_get_object_aws_docs() {
        let creds = AwsCredentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        let config = SigV4Config::new(creds, "us-east-1", "s3");
        let signed = sign_request(
            "GET",
            "examplebucket.s3.amazonaws.com",
            "/test.txt",
            "",
            &[("Range", "bytes=0-9")],
            b"",
            &config,
            "20130524T000000Z",
        )
        .unwrap();

        assert_eq!(signed.content_sha256, EMPTY_PAYLOAD_HASH);
        assert_eq!(signed.amz_date, "20130524T000000Z");
        assert_eq!(
            signed.authorization,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    /// AWS docs PUT Object golden vector (same page).
    #[test]
    fn golden_put_object_aws_docs() {
        let creds = AwsCredentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        let config = SigV4Config::new(creds, "us-east-1", "s3");
        let body = b"Welcome to Amazon S3.";
        let signed = sign_request(
            "PUT",
            "examplebucket.s3.amazonaws.com",
            "/test$file.text",
            "",
            &[
                ("Date", "Fri, 24 May 2013 00:00:00 GMT"),
                ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
            ],
            body,
            &config,
            "20130524T000000Z",
        )
        .unwrap();

        assert_eq!(
            signed.content_sha256,
            "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"
        );
        assert_eq!(
            signed.authorization,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,SignedHeaders=date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class,Signature=98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        );
    }

    #[test]
    fn empty_payload_hash_matches_constant() {
        assert_eq!(payload_hash(b""), EMPTY_PAYLOAD_HASH);
    }

    #[test]
    fn canonical_uri_encodes_dollar() {
        assert_eq!(canonical_uri("/test$file.text"), "/test%24file.text");
        assert_eq!(canonical_uri("/"), "/");
        assert_eq!(canonical_uri(""), "/");
    }

    #[test]
    fn parse_url_strips_default_https_port() {
        let p = parse_url_for_signing("https://bucket.s3.amazonaws.com:443/key").unwrap();
        assert_eq!(p.host_header, "bucket.s3.amazonaws.com");
        assert_eq!(p.path, "/key");
    }

    #[test]
    fn parse_url_keeps_nondefault_port() {
        let p = parse_url_for_signing("http://127.0.0.1:8765/chunks/ab/cd.cnk").unwrap();
        assert_eq!(p.host_header, "127.0.0.1:8765");
        assert_eq!(p.path, "/chunks/ab/cd.cnk");
    }

    #[test]
    fn path_style_url_signs_cleanly() {
        let creds = AwsCredentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        let config = SigV4Config::new(creds, "us-east-1", "s3");
        let clock = SigningClock::Fixed("20130524T000000Z".into());
        let signed = sign_url(
            "GET",
            "https://minio.example:9000/mybucket/data/chunks/ab/cd.cnk",
            &[],
            b"",
            &config,
            &clock,
        )
        .unwrap();
        assert!(
            signed
                .authorization
                .starts_with("AWS4-HMAC-SHA256 Credential=")
        );
        assert!(
            signed
                .authorization
                .contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date")
        );
        assert_eq!(signed.amz_date, "20130524T000000Z");
        assert_eq!(signed.content_sha256, EMPTY_PAYLOAD_HASH);
    }

    #[test]
    fn session_token_is_signed_and_emitted() {
        let creds = AwsCredentials {
            access_key_id: "AKIATOKEN".into(),
            secret_access_key: "secret".into(),
            session_token: Some("sess-tok".into()),
        };
        let config = SigV4Config::new(creds, "eu-west-1", "s3");
        let signed = sign_request(
            "HEAD",
            "bucket.s3.eu-west-1.amazonaws.com",
            "/obj",
            "",
            &[],
            b"",
            &config,
            "20200101T120000Z",
        )
        .unwrap();
        assert!(
            signed.authorization.contains(
                "SignedHeaders=host;x-amz-content-sha256;x-amz-date;x-amz-security-token"
            )
        );
        assert_eq!(signed.session_token.as_deref(), Some("sess-tok"));
        let pairs = signed.as_pairs();
        assert!(
            pairs
                .iter()
                .any(|(n, v)| n.eq_ignore_ascii_case("x-amz-security-token") && v == "sess-tok")
        );
    }
}
