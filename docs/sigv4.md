# Minimal in-process AWS SigV4 (Phase 8 P1)

ChunkForge can sign HTTP **GET / HEAD / PUT** with **AWS4-HMAC-SHA256** without
pulling in `aws-sdk-*`. Signing is **opt-in**, default **off** (≡ 0.7.0: no
SigV4 headers).

## Enable (CLI)

```bash
export AWS_ACCESS_KEY_ID=...
export AWS_SECRET_ACCESS_KEY=...
# optional:
export AWS_SESSION_TOKEN=...
export AWS_REGION=us-east-1   # default us-east-1 with a warning if unset

chunkforge push --store ./store \
  --dest 'https://minio.example:9000/mybucket' \
  --prefix 'data/' \
  --url-template '{base}/{prefix}{path}' \
  --aws-sigv4 \
  --http-retries 3 \
  release.cfdir

chunkforge verify --source 'https://minio.example:9000/mybucket' \
  --prefix 'data/' \
  --url-template '{base}/{prefix}{path}' \
  --aws-sigv4 \
  release.cfdir
```

`--aws-sigv4` applies to HTTP(S) `--source` / `--dest` on the same commands that
already accept `--url-template` / `--header` / `--http-retries`
(`cat` / `verify` / `doctor` / `push` / `pull` / `extract` / `mount`).

## Credentials (env only)

| Variable | Required when `--aws-sigv4` | Notes |
|---|---|---|
| `AWS_ACCESS_KEY_ID` | **yes** | Clear error if missing/empty |
| `AWS_SECRET_ACCESS_KEY` | **yes** | Clear error if missing/empty |
| `AWS_SESSION_TOKEN` | no | Sent as `x-amz-security-token` and included in the signature |
| `AWS_REGION` | no | Default **`us-east-1`** with a stderr warning |

**Not implemented:** shared credentials file, IMDS / instance role, SSO, ECS
task role, automatic refresh, or any other credential provider chain.

## What is signed

| Item | Behaviour |
|---|---|
| Methods | GET, HEAD, PUT (and POST if the sink is configured for POST) |
| Payload hash | `hex(SHA256(body))`; empty body = SHA256 of empty bytes (`e3b0c44298…`) — **never** `UNSIGNED-PAYLOAD` |
| Headers added | `Authorization`, `x-amz-date`, `x-amz-content-sha256`, optional `x-amz-security-token` |
| Service name | `s3` |
| URL style | **Path-style required** (bucket in path / base); virtual-host style works when the host already carries the bucket |

Chunk bodies are ≤ 256 KiB plaintext, so full-payload hashing is cheap. There is
**no** chunked / streaming signing.

## Conflict with `--header Authorization:…`

`--aws-sigv4` and `--header Authorization:…` together are a **hard error**
(library builder and CLI). Use one or the other — not both.

## Library API

```rust
use chunkforge_remote::{
    AwsCredentials, ChunkSource, HttpChunkSource, SigV4Config, SigV4Signer, SigningClock,
};

let creds = AwsCredentials {
    access_key_id: "...".into(),
    secret_access_key: "...".into(),
    session_token: None,
};
let signer = SigV4Signer::new(SigV4Config::new(creds, "us-east-1", "s3"));
// Tests may pin the clock:
// let signer = signer.with_clock(SigningClock::Fixed("20130524T000000Z".into()));

let src = HttpChunkSource::builder("https://minio.example/mybucket")
    .url_template("{base}/{prefix}{path}")
    .prefix("data/")
    .aws_sigv4(signer)
    .build()?;
```

Golden-vector unit tests live in `chunkforge-remote::sigv4` (AWS docs GET/PUT
Object examples with a fixed clock).

## Warnings

- Clocks must be roughly in sync with the server (SigV4 typically allows ~15
  minutes of skew).
- Do not log expanded `Authorization` / secret keys.
- MinIO and other S3-compatible stores often need path-style URLs and a matching
  region string (sometimes ignored by the server, but still part of the
  credential scope).

## Non-goals

| Non-goal | Status |
|---|---|
| ❌ `aws-sdk-*` / `aws-config` / `aws-smithy-*` | Forbidden |
| ❌ ListObjects / bucket listing / remote scrub | Out of scope |
| ❌ S3 multipart upload API | Out of scope (single-object PUT only) |
| ❌ `UNSIGNED-PAYLOAD` / chunked signing | Out of scope |
| ❌ Full credential provider chain (IMDS/SSO/files) | Env only |
| ❌ Presigned URL generation | Use external tools + `--url-template` if needed |

See also [remote-layout.md](remote-layout.md), [http-retry.md](http-retry.md),
and [diff.md](diff.md) (`--format json`).
