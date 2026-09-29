//! Minimal URL / header template expansion (no handlebars/tera).
//!
//! Closed placeholder set — unknown names are hard errors.

use crate::layout::chunk_http_path;
use chunkforge_store::ChunkId;
use thiserror::Error;

/// Errors from [`expand_template`] / builder validation.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TemplateError {
    /// A `{name}` that is not in the closed placeholder set.
    #[error("unknown placeholder: {{{0}}}")]
    UnknownPlaceholder(String),

    /// `{}` with nothing between the braces.
    #[error("empty placeholder in template")]
    EmptyPlaceholder,

    /// A `{` with no matching `}`.
    #[error("unclosed placeholder in template")]
    UnclosedPlaceholder,

    /// `{env:NAME}` but `NAME` is not set in the process environment.
    #[error("environment variable not set: {0}")]
    MissingEnv(String),

    /// `{env:}` with an empty name.
    #[error("invalid env placeholder (expected {{env:NAME}}): {{{0}}}")]
    InvalidEnv(String),
}

/// Context for expanding a template against a single chunk.
#[derive(Debug, Clone, Copy)]
pub struct TemplateCtx<'a> {
    /// HTTP(S) base with trailing `/` already stripped.
    pub base: &'a str,
    /// Chunk id whose hex segments fill `{2hex}` / `{62hex}` / `{id}` / `{path}`.
    pub id: &'a ChunkId,
    /// Key prefix for `{prefix}` (preferably already [`normalize_prefix`]'d).
    pub prefix: &'a str,
}

/// Normalize a key prefix for `{prefix}`:
/// - empty / only-slashes → `""`
/// - otherwise strip leading `/` and ensure exactly one trailing `/` (`foo` → `foo/`)
pub fn normalize_prefix(prefix: &str) -> String {
    let trimmed = prefix.trim_start_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.ends_with('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/")
    }
}

/// Expand `tmpl` with the closed placeholder set.
///
/// | Placeholder   | Expansion |
/// |---------------|-----------|
/// | `{base}`      | `ctx.base` |
/// | `{path}`      | `chunks/<2hex>/<62hex>.cnk` ([`chunk_http_path`]) |
/// | `{2hex}`      | first 2 hex chars of the chunk id |
/// | `{62hex}`     | remaining 62 hex chars |
/// | `{id}`/`{hex}`| full 64 lowercase hex |
/// | `{prefix}`    | normalized prefix (see [`normalize_prefix`]) |
/// | `{env:NAME}`  | `std::env::var("NAME")`; missing → [`TemplateError::MissingEnv`] |
pub fn expand_template(tmpl: &str, ctx: &TemplateCtx<'_>) -> Result<String, TemplateError> {
    let hex = ctx.id.to_hex();
    debug_assert_eq!(hex.len(), 64);
    let path = chunk_http_path(ctx.id);
    // Normalize on every expand so callers may pass raw or already-normalized prefixes.
    let prefix = normalize_prefix(ctx.prefix);

    let mut out = String::with_capacity(tmpl.len());
    let mut rest = tmpl;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        let Some(end) = rest.find('}') else {
            return Err(TemplateError::UnclosedPlaceholder);
        };
        let name = &rest[..end];
        rest = &rest[end + 1..];

        if name.is_empty() {
            return Err(TemplateError::EmptyPlaceholder);
        }

        if let Some(env_name) = name.strip_prefix("env:") {
            if env_name.is_empty() {
                return Err(TemplateError::InvalidEnv(name.to_string()));
            }
            match std::env::var(env_name) {
                Ok(val) => out.push_str(&val),
                Err(_) => return Err(TemplateError::MissingEnv(env_name.to_string())),
            }
            continue;
        }

        match name {
            "base" => out.push_str(ctx.base),
            "path" => out.push_str(&path),
            "2hex" => out.push_str(&hex[..2]),
            "62hex" => out.push_str(&hex[2..]),
            "id" | "hex" => out.push_str(&hex),
            "prefix" => out.push_str(&prefix),
            _ => return Err(TemplateError::UnknownPlaceholder(name.to_string())),
        }
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::chunk_url;

    fn ctx_for<'a>(base: &'a str, id: &'a ChunkId, prefix: &'a str) -> TemplateCtx<'a> {
        TemplateCtx { base, id, prefix }
    }

    #[test]
    fn default_template_matches_chunk_url() {
        let id = ChunkId::hash(b"template-compat");
        let base = "http://cdn.example/cf-base";
        let ctx = ctx_for(base, &id, "");
        let expanded = expand_template("{base}/{path}", &ctx).unwrap();
        assert_eq!(expanded, chunk_url(base, &id));
    }

    #[test]
    fn custom_placeholders_expand() {
        let id = ChunkId::from_bytes([0u8; 32]);
        let hex = id.to_hex();
        assert_eq!(hex, "0".repeat(64));
        let ctx = ctx_for("https://minio.example/bucket", &id, "data");
        let url = expand_template("{base}/{prefix}chunks/{2hex}/{62hex}.cnk", &ctx).unwrap();
        assert_eq!(
            url,
            format!(
                "https://minio.example/bucket/data/chunks/{}/{}.cnk",
                &hex[..2],
                &hex[2..]
            )
        );
        assert_eq!(
            expand_template("{base}/{prefix}{path}", &ctx).unwrap(),
            format!("https://minio.example/bucket/data/{}", chunk_http_path(&id))
        );
        assert_eq!(
            expand_template("id={id}", &ctx).unwrap(),
            format!("id={hex}")
        );
        assert_eq!(
            expand_template("hex={hex}", &ctx).unwrap(),
            format!("hex={hex}")
        );
    }

    #[test]
    fn unknown_placeholder_is_hard_error() {
        let id = ChunkId::from_bytes([0u8; 32]);
        let ctx = ctx_for("http://x", &id, "");
        let err = expand_template("{base}/{bucket}/{path}", &ctx).unwrap_err();
        assert_eq!(err, TemplateError::UnknownPlaceholder("bucket".into()));
    }

    #[test]
    fn unclosed_and_empty_placeholders() {
        let id = ChunkId::from_bytes([0u8; 32]);
        let ctx = ctx_for("http://x", &id, "");
        assert_eq!(
            expand_template("{base", &ctx).unwrap_err(),
            TemplateError::UnclosedPlaceholder
        );
        assert_eq!(
            expand_template("{}", &ctx).unwrap_err(),
            TemplateError::EmptyPlaceholder
        );
    }

    #[test]
    fn prefix_normalization() {
        assert_eq!(normalize_prefix(""), "");
        assert_eq!(normalize_prefix("/"), "");
        assert_eq!(normalize_prefix("data"), "data/");
        assert_eq!(normalize_prefix("data/"), "data/");
        assert_eq!(normalize_prefix("/data/"), "data/");
        assert_eq!(normalize_prefix("a/b"), "a/b/");
    }

    #[test]
    fn env_placeholder() {
        let id = ChunkId::from_bytes([0u8; 32]);
        let ctx = ctx_for("http://x", &id, "");
        // Unique name to avoid colliding with the ambient environment.
        let var = "CHUNKFORGE_TEMPLATE_TEST_TOKEN_M1";
        // Ensure clean slate.
        unsafe { std::env::remove_var(var) };
        let err =
            expand_template("Bearer {env:CHUNKFORGE_TEMPLATE_TEST_TOKEN_M1}", &ctx).unwrap_err();
        assert_eq!(
            err,
            TemplateError::MissingEnv("CHUNKFORGE_TEMPLATE_TEST_TOKEN_M1".into())
        );
        unsafe { std::env::set_var(var, "secret-value") };
        assert_eq!(
            expand_template("Bearer {env:CHUNKFORGE_TEMPLATE_TEST_TOKEN_M1}", &ctx).unwrap(),
            "Bearer secret-value"
        );
        unsafe { std::env::remove_var(var) };
        assert_eq!(
            expand_template("{env:}", &ctx).unwrap_err(),
            TemplateError::InvalidEnv("env:".into())
        );
    }
}
