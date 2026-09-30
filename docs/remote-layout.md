# ChunkForge remote chunk layout (HTTP / `file://`)

Remote backends reuse the **same on-disk layout** as the local CAS store.
A static HTTP directory of an existing store root is a valid chunk source; so is a
`file://` URL (or plain path) pointing at another store tree.

Phase 3 extends `HttpChunkSource` with **URL / header templates** and an optional
**key prefix** so object-store–friendly read paths (MinIO, R2, S3 public/CDN,
path-style endpoints) work **without** an AWS SDK. Phase 8 adds optional
**in-process SigV4** (`--aws-sigv4`; default off) — see [sigv4.md](sigv4.md).

Phase 4 adds the symmetric write face: `HttpChunkSink` issues **single-object PUT**
(optional POST) to the **same expanded URL** a GET would use, so
`chunkforge push` followed by `verify --source` hits identical keys. See
[push.md](push.md) for CLI semantics.

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
are the plaintext. With create-time **`--compression zstd`** (Phase 17 / 1.7.0
opt-in; default **none** ≡ 1.6), local `.cnk` files may hold zstd-encoded
payloads, but **`Store::get` / `ChunkSource::get` still return plaintext**, and
HTTP GET/PUT bodies remain **plaintext** (push decodes before PUT). Disk zstd
≠ wire Content-Encoding ≠ packfile. Serve any store over HTTP via the same
plaintext body contract.

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

> **CLI note:** `cat` / `verify` / `mount` / `doctor` / `push` / `pull` / `extract`
> accept `--url-template` / `--prefix` / `--header` for `http(s)://` sources
> (Phase 3+), plus `--http-retries` / `--http-retry-backoff-ms` (Phase 8; default
> retries **0**) and optional `--aws-sigv4` (Phase 8 P1; default **off**).
> Non-HTTP sources reject template / SigV4 flags; retry flags are ignored for local origins.

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

## S3-compatible path conventions (GET + PUT)

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

### Auth patterns

1. **Public read / CDN / static site** — no headers (same as Phase 2).
2. **Fixed header auth** — e.g. `Authorization: Bearer {env:TOKEN}` or a
   gateway-issued static token (value template expanded per request).
3. **Presigned query in the URL template** — you may embed query parameters in
   `url_template`; they are preserved literally aside from placeholder
   expansion. **Per-object signatures that differ for every key are not
   auto-generated** by the template engine alone.
4. **In-process SigV4** (Phase 8 P1) — `--aws-sigv4` + env credentials; signs
   GET/HEAD/PUT with AWS4-HMAC-SHA256. Default **off**. Conflicts with
   `--header Authorization:…`. Details: [sigv4.md](sigv4.md).

## HTTP chunk PUT (Phase 4)

`HttpChunkSink` shares base / `url_template` / `prefix` / header templates with
`HttpChunkSource`. For a given id and identical builder options:

```text
PUT  url_for(id)   ≡   GET url_for(id)
body               =   plaintext chunk bytes (BLAKE3 == id)
Content-Type       =   application/octet-stream   (default)
```

Default key under the document root / bucket:

```text
{prefix}chunks/{2hex}/{62hex}.cnk
```

Same examples as the GET section apply with `PUT` instead of `GET` — e.g.
`base = http://127.0.0.1:8766`, default template:

```text
PUT http://127.0.0.1:8766/chunks/af/1349…3262.cnk
```

### Semantics (`HttpChunkSink`)

| Case | Behaviour |
|---|---|
| `has` → true (HEAD, GET fallback) | Skip PUT → `PutOutcome::SkippedExists` (default `skip_if_exists`) |
| PUT/POST **2xx** | `PutOutcome::Written` |
| PUT/POST **409 Conflict** | Treated as skip when `accept_conflict` (default true) |
| Other non-2xx / transport error | `SinkError::Backend` |
| Hash mismatch before send | `SinkError::Corrupt` when `verify_hash` (default true) |
| Bad template / missing `{env:…}` | Fail at **builder** (`build()`), not mid-request |

CLI `chunkforge push` always builds an HTTP sink; non-`http(s)://` `--dest` is
rejected. Body is always **plaintext** even if the local store used zstd
(push decodes before PUT — remote layout = plaintext, matching GET assumptions).

### Local PUT stub (demo)

```bash
# Accepts PUT/HEAD/GET; writes under ./mirror/chunks/...
python3 scripts/put_stub.py --root ./mirror --port 8766
chunkforge push --store ./store --dest http://127.0.0.1:8766 hello.cfidx
chunkforge verify --source http://127.0.0.1:8766 hello.cfidx
```

Full smoke: [`scripts/demo_push.sh`](../scripts/demo_push.sh) / `make demo-push`.
Details: [push.md](push.md).

## Read failover (`--fallback`, Phase 16)

CLI read commands may repeat **`--fallback <PATH|URL>`** behind a primary
`--source` / `--store`. Failover is **Missing-only** (Transient/Corrupt fail
fast). Recommended composition: outer **`--cache`** wraps the whole fallback
chain. **`--fallback` ≠ cache fill ≠ sync ≠ prune ≠ write-back**. Templates /
retries / SigV4 apply isomorphically to each `http(s)://` origin in the chain.
Zero fallbacks ≡ 1.5 single-origin read path. See [mount.md](mount.md) /
[extract.md](extract.md) / [pull.md](pull.md).

## Cache observation vs failover vs sync (Phase 18 / 1.8.0; still holds in Phase 19 / 1.9 target)

| Flag / layer | Role | Not |
|---|---|---|
| **`--cache`** | Optional local fill-on-miss store (outer wrap) | Not sync / not write-back |
| **`--cache-stats`** | Opt-in stderr / ops-json observation of hits / miss_fills / miss_refused | **≠ LRU ≠ trim ≠ eviction** |
| **`--fallback`** | Missing-only ordered failover origins | **≠ cache ≠ sync ≠ prune** |
| Soft `--cache-max-bytes` | Refuse-fill budget | Never evicts |

**`--cache-stats` ≠ LRU ≠ sync ≠ fallback.** Observation only; default quiet ≡ 1.7.

## Disk zstd ≠ wire compression ≠ pack (Phase 17–19 / toward 1.9.0)

| Layer | Contract |
|---|---|
| Local store create | Opt-in `--compression none\|zstd` on `make` / `archive` / **`store create`** / **`pull`** (omit ≡ **none** ≡ 1.6/1.8); **`store create` ≠ recompress ≠ default zstd ≠ pack** |
| On-disk `.cnk` | May be zstd-encoded when meta says `zstd` |
| `get` / HTTP PUT·GET | Always **plaintext** body (BLAKE3 == id) |
| Packfile | **Not** implemented — see [perf.md](perf.md); Phase19 / 1.9.0 still does not implement pack |

`--progress` smoke (archive/extract/make): [`scripts/demo_zstd_progress.sh`](../scripts/demo_zstd_progress.sh).
Store lifecycle + pull compression smoke: [`scripts/demo_store_create_pull_compression.sh`](../scripts/demo_store_create_pull_compression.sh).

## Explicit non-goals (this layout / Phase 3–19 / through 1.9 target)

| Non-goal | Status |
|---|---|
| ❌ **`aws-sdk-*` / `aws-config` / `aws-smithy-*` / ListObjects** | Forbidden — HTTP stays **ureq**; SigV4 is a small HMAC helper |
| ❌ **Full credential provider chain** | Env vars only (`AWS_ACCESS_KEY_ID` / …); no IMDS / SSO / shared files |
| ❌ **S3 multipart upload API** | No Initiate/UploadPart/Complete; **per-chunk single PUT** only |
| ❌ **Remote GC / bucket lifecycle** | Local `chunkforge gc` only touches a local store |
| ❌ **Auto batch-presign every chunk** | Static URL template only; or use `--aws-sigv4` for in-process signing |
| ❌ **`UNSIGNED-PAYLOAD` / chunked streaming signing** | Always `hex(SHA256(body))` |

## `file://` (`FileUrlSource`)

Resolves to a filesystem path and opens it with `Store::open` (requires `meta.toml`).
Only empty / `localhost` / `127.0.0.1` authorities are accepted. Templates /
`prefix` do **not** apply to `file://`.

```rust
use chunkforge_remote::{ChunkSource, FileUrlSource};

let src = FileUrlSource::open("file:///tmp/cf-store")?;
let bytes = src.get(&chunk_id)?;
```

## Related CLI

- CLI `--url-template` / `--prefix` / `--header` on `cat` / `verify` / `mount` / `doctor` (Phase 3) and **`push`** (Phase 4); `pull` / `extract` share the same template flags
- `--http-retries N` / `--http-retry-backoff-ms` (Phase 8): bounded retries for transient HTTP failures (default **0** ≡ single attempt); local `--store` / `file://` ignore them
- `--aws-sigv4` (Phase 8 P1): optional AWS4-HMAC-SHA256; default **off**; see [sigv4.md](sigv4.md)
- Error classification + push/pull `failed_transient=` / `failed_permanent=` — see [http-retry.md](http-retry.md)
- `chunkforge push` — per-chunk PUT with optional `--jobs` concurrency; see [push.md](push.md)
- `chunkforge doctor` — presence check; optional `--progress` / `--cache-stats` (Phase 18; default off); see [doctor-gc.md](doctor-gc.md)
- `chunkforge gc` — **local** dry-run / `--apply` only; see [doctor-gc.md](doctor-gc.md)
- `--cache-stats` on read paths (Phase 18): observation only — **≠ LRU ≠ sync ≠ fallback**

## Related docs

- [http-retry.md](http-retry.md) — `--http-retries`, error classes, demo
- [sigv4.md](sigv4.md) — optional minimal `--aws-sigv4`
- [diff.md](diff.md) — `diff --format json`
- [push.md](push.md) / [pull.md](pull.md) — summaries with `retries=`

## Still out of scope

- HTTP Content-Encoding / mixed compression over HTTP (disk zstd ≠ wire); byte-range / partial-chunk retries (Phase 8 retries **whole chunks** only); packfile
- `aws-sdk-*` / full credential chain / S3 multipart / ListObjects / remote GC (see non-goals above)
- Uploading `.cfidx` into the chunk object layout (indexes stay out-of-band)

Minimal **in-process SigV4** is delivered as Phase 8 P1 (`--aws-sigv4`, default off) — see [sigv4.md](sigv4.md).
