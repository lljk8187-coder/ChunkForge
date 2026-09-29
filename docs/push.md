# `chunkforge push`

Phase 4 write path: upload **plaintext chunks** referenced by one or more
`.cfidx` files from a local CAS `--store` to an HTTP(S) destination whose URL
layout matches the Phase 3 read path. After a successful push, the same base URL
works with existing `verify` / `cat` / `doctor` / `mount --source`.

## Usage

```bash
chunkforge push \
  --store <local-cas> \
  --dest 'https://example/mybucket' \
  [--url-template '{base}/{path}'] \
  [--prefix 'data/'] \
  [--header 'Authorization: Bearer {env:TOKEN}'] \
  [--dry-run] \
  index1.cfidx [index2.cfidx ...]
```

| Flag | Meaning |
|---|---|
| `--store` | Local CAS providing plaintext chunk bytes (`Store::get`) |
| `--dest` | HTTP(S) base URL (required shape for the remote write face) |
| `--url-template` / `--prefix` / `--header` | Same closed placeholders as read-side `HttpChunkSource` (see [remote-layout.md](remote-layout.md)) |
| `--dry-run` | Probe + count only; **no** PUT |
| indexes | One or more `.cfidx` files; chunk id set is the **union** |

Default URL template is `{base}/{path}` ≡ `{base}/chunks/<2hex>/<62hex>.cnk`,
identical to Phase 2/3 GET layout.

### What push does

1. Load and validate every index; merge referenced `ChunkId`s.
2. For each id (sorted): read plaintext from the local store; remote `has`
   (HEAD, GET fallback) → skip; otherwise `PUT` the body.
3. Print a summary on stderr:
   `push: skipped=… uploaded=… failed=… (N unique chunk ids, M indexes, dry_run=…)`.
4. Exit **non-zero** if `failed > 0`.

### What push does **not** do

| Non-goal | Detail |
|---|---|
| ❌ Upload `.cfidx` | Indexes stay local / are published by the user separately |
| ❌ Remote GC / delete | Extra remote objects are left alone |
| ❌ Bidirectional sync | Explicit one-way publish only |
| ❌ S3 multipart API | Chunks are ≤256KiB; single-object PUT is enough |
| ❌ `aws-sdk-*` / in-process SigV4 | ureq + template headers / external presign only |
| ❌ `--jobs` (Phase 4 P1 / M5) | Serial upload in this milestone |

## Auth headers

Auth is the same posture as Phase 3 reads:

1. **Public / open write endpoint** (demo stub, MinIO with open PUT) — no headers.
2. **Fixed header templates** — e.g.
   `--header 'Authorization: Bearer {env:CF_TOKEN}'`.
   `{env:NAME}` is expanded at sink build time; a missing env var fails early.
3. **Presigned query in `url_template`** — query params are preserved literally
   aside from placeholder expansion. Per-object signatures that differ for every
   key are **not** auto-generated.

Headers apply to HEAD/GET probes and to PUT. Do not log expanded
`Authorization` / signature segments at default tracing levels.

```bash
export CF_TOKEN=demo
chunkforge push \
  --store ./store \
  --dest 'https://gateway.example/bucket' \
  --url-template '{base}/{prefix}{path}' \
  --prefix 'data/' \
  --header 'Authorization: Bearer {env:CF_TOKEN}' \
  hello.cfidx
```

## Failure semantics

| Situation | Behaviour |
|---|---|
| Index unreadable / invalid | Abort before uploads (non-zero) |
| Local store missing a referenced chunk | Count as **failed**; print `push: fail <id>: …`; continue other ids; exit non-zero |
| Remote `has` transport / non-skip error | **failed** for that id; continue; exit non-zero |
| PUT 2xx | **uploaded** (`Written`) |
| PUT 409 Conflict (default) | Treated as success / **skipped** (`SkippedExists`) |
| PUT other 4xx / 5xx / network error | **failed**; continue; exit non-zero |
| Non-`http(s)://` `--dest` | Immediate readable error (templates rejected too) |
| `--dry-run` | Counts would-be uploads in `uploaded=`; **zero** PUT requests |

Push does not panic on remote rejection. Partial progress may leave some chunks
on the remote; re-run is safe (see idempotency).

## No `.cfidx` upload

The object layout under `--dest` holds **chunks only**
(`chunks/<2hex>/<62hex>.cnk`, optionally under `{prefix}`). The `.cfidx` file is
an out-of-band artifact: copy it next to releases, store it in git, or serve it
from a different URL. `verify --source <dest> hello.cfidx` still needs the index
**locally** (or wherever you pass the path); only chunk bodies are fetched from
the remote.

## Idempotency

Safe to re-run:

1. Default sink probes with `has` first → already-present ids become `skipped`.
2. Optional `409 Conflict` on PUT also counts as skip.
3. Hash is verified locally (`blake3(plain) == id`) before PUT when the sink’s
   `verify_hash` is on (CLI default).

Expect a second push with the same store + indexes against an unchanged remote:

```text
push: skipped=N uploaded=0 failed=0 …
```

## Local demo (no internet)

```bash
# From the repo root (~10 minutes including first cargo build)
./scripts/demo_push.sh
# or: make demo-push
```

The script builds the CLI, `make`s `fixtures/hello.txt`, starts
[`scripts/put_stub.py`](../scripts/put_stub.py) on `127.0.0.1:8766`, runs
`push` → `verify --source` → `cat` + `cmp`, then a second push asserting
`uploaded=0`. See also the Phase 4 §6.2 command sketch in the research notes.

Manual equivalent:

```bash
cargo build -p chunkforge-cli
mkdir -p /tmp/cf-p4/{store,mirror}
./target/debug/chunkforge make --store /tmp/cf-p4/store -o /tmp/cf-p4/hello.cfidx \
  ./fixtures/hello.txt

# terminal 1
python3 ./scripts/put_stub.py --root /tmp/cf-p4/mirror --port 8766

# terminal 2
./target/debug/chunkforge push \
  --store /tmp/cf-p4/store \
  --dest http://127.0.0.1:8766 \
  --url-template '{base}/{path}' \
  /tmp/cf-p4/hello.cfidx
./target/debug/chunkforge verify --source http://127.0.0.1:8766 /tmp/cf-p4/hello.cfidx
```

## Related

- Layout + PUT key conventions: [remote-layout.md](remote-layout.md)
- Read-side templates / auth: same document, HTTP GET section
- CLI library face: `HttpChunkSink` in `chunkforge-remote` (isomorphic with `HttpChunkSource`)
