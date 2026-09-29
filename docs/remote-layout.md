# ChunkForge remote chunk layout (HTTP / `file://`)

Phase 2 remote backends reuse the **same on-disk layout** as the local CAS store.
A static HTTP directory of an existing store root is a valid chunk source; so is a
`file://` URL (or plain path) pointing at another store tree.

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
are the plaintext. (Compressed stores write encoded payloads; HTTP Phase 2 skeleton
treats the response body as plaintext — serve uncompressed stores for demos.)

## `--source` URL forms

```text
/path/to/store                 # plain local path
file:///path/to/store          # file URL → Store::open
file://localhost/path/to/store # same (local authority only)
http://127.0.0.1:8000/cf-base  # HTTP static base
https://cdn.example/cf-base    # HTTPS static base
```

Trailing slashes on the HTTP base are optional; join logic strips a single trailing
`/` before appending `/chunks/…`.

## HTTP chunk URL

```text
GET {base}/chunks/<2hex>/<62hex>.cnk
```

Examples (`base = http://127.0.0.1:8765`, id hex starts with `af1349…`):

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

`has` uses `HEAD` when possible; falls back to `GET` if the server returns 405/501.

### Demo: static file server

```bash
# After `chunkforge make --store ./store …`
cd ./store && python3 -m http.server 8765
# Clients use --source http://127.0.0.1:8765
```

## `file://` (`FileUrlSource`)

Resolves to a filesystem path and opens it with `Store::open` (requires `meta.toml`).
Only empty / `localhost` / `127.0.0.1` authorities are accepted.

```rust
use chunkforge_remote::{ChunkSource, FileUrlSource};

let src = FileUrlSource::open("file:///tmp/cf-store")?;
let bytes = src.get(&chunk_id)?;
```

## Out of scope (this document / M2)

- CLI `--source` / `--cache` (M3)
- FUSE mount (M4–M5)
- S3 SDK, presigned PUT, multipart
- Mixed compression over HTTP, range requests, smart retries beyond a simple timeout
