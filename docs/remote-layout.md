# ChunkForge remote chunk layout (HTTP / `file://`)

Remote backends reuse the **same on-disk layout** as the local CAS store.
A static HTTP directory of an existing store root is a valid chunk source; so is a
`file://` URL (or plain path) pointing at another store tree.

Phase 3 extends `HttpChunkSource` with **URL / header templates** and an optional
**key prefix** so object-store–friendly read paths (MinIO, R2, S3 public/CDN,
path-style endpoints) work **without** an AWS SDK or in-process SigV4.

## Local store layout (unchanged)

```text
<store_root>/
  meta.toml
  chunks/
    ab/                    # first 2 hex of BLAKE3 (lowercase)
      cdef…rest.cnk        # remaining 62 hex + .cnk
```

Relative chunk path for id hex `H` (64 chars):

```text
chunks/<H[0..2]>/<H[2..]>.cnk
```

Hash is always over **plaintext**. With `Compression::None`, the `.cnk` file bytes
are the plaintext. (Compressed stores write encoded payloads; HTTP treats the
response body as plaintext — serve uncompressed stores for demos.)

## `--source` URL forms

```text
/path/to/store                 # plain local path
file:///path/to/store          # file URL → Store::open
file://localhost/path/to/store # same (local authority only)
http://127.0.0.1:8000/cf-base  # HTTP static base
https://cdn.example/cf-base    # HTTPS static base
https://minio:9000/mybucket    # path-style: bucket in base (Phase 3)
https://mybucket.s3.amazonaws.com  # virtual-host style base (Phase 3)
```

Trailing slashes on the HTTP base are optional; join / template logic strips a
trailing `/` from `{base}` before expansion.

> **CLI note:** `cat` / `verify` / `mount` / `doctor` accept `--url-template` /
> `--prefix` / `--header` for `http(s)://` sources (Phase 3 M3+). Non-HTTP
> sources reject those flags with a readable error.

## HTTP chunk URL (default ≡ Phase 2)

Default template:

```text
{base}/{path}
≡ {base}/chunks/<2hex>/<62hex>.cnk
```

Example (`base = http://127.0.0.1:8765`, id hex starts with `af1349…`):

```text
http://127.0.0.1:8765/chunks/af/1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262.cnk
```

### Semantics (`HttpChunkSource`)

| Case | Behavior |
|---|---|
| 2xx + non-empty body | Body = plaintext; default **BLAKE3 verify** vs `ChunkId` |
| 404 / 410 | `SourceError::NotFound` |
| Other non-2xx | `SourceError::Backend` |
| Empty body | `SourceError::Backend` (empty response) |
| Hash mismatch | `SourceError::Corrupt` (unless `verify_hash = false`) |
| Bad template / missing `{env:…}` | Fail at **builder** (`build()`), not mid-request |

`has` uses `HEAD` when possible; falls back to `GET` if the server returns 405/501.
Custom header templates are attached to both `HEAD` and `GET`.

### Demo: static file server

```bash
# After `chunkforge make --store ./store …`
cd ./store && python3 -m http.server 8765
# Clients use --source http://127.0.0.1:8765
```

## URL / header template placeholders

Expansion is a **closed** set of replacements (no handlebars/tera). Unknown
`{name}` is a hard error at `HttpChunkSourceBuilder::build`.

| Placeholder | Expands to |
|---|---|
| `{base}` | HTTP(S) base from the source (trailing `/` stripped) |
| `{2hex}` | chunk id hex `[0..2]` (lowercase) |
| `{62hex}` | chunk id hex `[2..]` (62 chars) |
| `{id}` / `{hex}` | full 64 lowercase hex |
| `{path}` | `chunks/{2hex}/{62hex}.cnk` (same as `chunk_http_path`) |
| `{prefix}` | optional key prefix after [`normalize_prefix`](#prefix-normalization) |
| `{env:NAME}` | `std::env::var("NAME")`; missing → build error |

**Default template (Phase 2 compatible):** `{base}/{path}`

**Recommended object-store template:** `{base}/{prefix}{path}`

`{env:NAME}` is intended for **header** (or query-in-URL) templates. Prefer not
to put secrets into path templates; never log expanded `Authorization` / signature
query segments at default tracing levels.

```rust
use chunkforge_remote::{ChunkSource, HttpChunkSource};

let src = HttpChunkSource::builder("https://minio.example/mybucket")
    .url_template("{base}/{prefix}{path}")
    .prefix("data/")
    .header("Authorization", "Bearer {env:CF_TOKEN}")
    .build()?;
let url = src.url_for(&chunk_id);
// → https://minio.example/mybucket/data/chunks/<2hex>/<62hex>.cnk
```

## Prefix normalization

`HttpChunkSourceBuilder::prefix` / `{prefix}` use `normalize_prefix`:

| Input | Normalized `{prefix}` |
|---|---|
| `""` / `"/"` / only slashes | `""` (empty — no extra path segment) |
| `"data"` | `"data/"` |
| `"data/"` | `"data/"` |
| `"/data/"` | `"data/"` |
| `"a/b"` | `"a/b/"` |

Rules:

1. Strip **leading** `/` characters.
2. If the result is empty → `""`.
3. Otherwise ensure **exactly one** trailing `/` (so `{prefix}{path}` never
   produces a double slash or a glued `datachunks/…`).

With template `{base}/{prefix}{path}` and `prefix = "data/"` the GET path always
contains `/data/chunks/…`.

## S3-compatible path conventions (read-only)

Default **object key** (relative to the bucket / document root):

```text
{prefix}chunks/{2hex}/{62hex}.cnk
```

This matches the local CAS relative path, so syncing a store tree into a bucket
needs **no key rewrite**.

The library does **not** parse a separate `bucket` field. Put the bucket in
`base` and/or `prefix` yourself.

### Path-style (preferred in docs / demos)

Bucket is part of the path under the endpoint:

```text
base     = https://minio.example:9000/mybucket
prefix   = data/          # optional subdirectory inside the bucket
template = {base}/{prefix}{path}

→ GET https://minio.example:9000/mybucket/data/chunks/<2hex>/<62hex>.cnk
```

Public / CDN static hosting is the same shape with an empty prefix:

```text
base     = https://cdn.example/cf-base
template = {base}/{path}          # default; omit to keep Phase 2 behavior
→ GET https://cdn.example/cf-base/chunks/<2hex>/<62hex>.cnk
```

### Virtual-host style

Bucket is in the hostname; key starts at the first path segment:

```text
base     = https://mybucket.s3.amazonaws.com
prefix   =                       # empty
template = {base}/{path}         # default

→ GET https://mybucket.s3.amazonaws.com/chunks/<2hex>/<62hex>.cnk
```

With a key prefix inside the bucket:

```text
base     = https://mybucket.s3.us-east-1.amazonaws.com
prefix   = releases/v1/
template = {base}/{prefix}{path}

→ GET https://mybucket.s3.us-east-1.amazonaws.com/releases/v1/chunks/<2hex>/<62hex>.cnk
```

No code branch distinguishes path-style vs virtual-host — only the strings you
pass as `base` / `prefix` / `url_template`.

### Auth patterns that Phase 3 supports

1. **Public read / CDN / static site** — no headers (same as Phase 2).
2. **Fixed header auth** — e.g. `Authorization: Bearer {env:TOKEN}` or a
   gateway-issued static token (value template expanded per request).
3. **Presigned query in the URL template** — you may embed query parameters in
   `url_template`; they are preserved literally aside from placeholder
   expansion. **Per-object signatures that differ for every key are not
   auto-generated** (out of scope).

## Explicit non-goals (this layout / Phase 3)

| Non-goal | Status |
|---|---|
| ❌ **`aws-sdk-*` / `aws-config` / ListObjects** | Forbidden — dependency surface stays `ureq` |
| ❌ **In-process SigV4** (even GET-only HMAC) | Not in Phase 3; use external presign or header templates |
| ❌ **Upload / multipart / PUT / POST** | Read path only |
| ❌ **Remote GC / bucket lifecycle** | Local `chunkforge gc` only touches a local store |
| ❌ **Auto batch-presign every chunk** | Static URL template only |

## `file://` (`FileUrlSource`)

Resolves to a filesystem path and opens it with `Store::open` (requires `meta.toml`).
Only empty / `localhost` / `127.0.0.1` authorities are accepted. Templates /
`prefix` do **not** apply to `file://`.

```rust
use chunkforge_remote::{ChunkSource, FileUrlSource};

let src = FileUrlSource::open("file:///tmp/cf-store")?;
let bytes = src.get(&chunk_id)?;
```

## Related CLI (Phase 3 delivered)

- CLI `--url-template` / `--prefix` / `--header` on `cat` / `verify` / `mount` / `doctor`
- `chunkforge doctor` — presence check; see [doctor-gc.md](doctor-gc.md)
- `chunkforge gc` — **local** dry-run / `--apply` only; see [doctor-gc.md](doctor-gc.md)

## Still out of scope

- Mixed compression over HTTP, range requests, smart retries beyond a simple timeout
- In-process SigV4, `aws-sdk-*`, upload/multipart, remote GC (see non-goals above)
