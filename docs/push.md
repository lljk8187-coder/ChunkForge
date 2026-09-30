# `chunkforge push`

Phase 4 write path (Phase 5 extended): upload **plaintext chunks** referenced
by one or more `.cfidx` **or** `.cfdir` listings from a local CAS `--store` to
an HTTP(S) destination whose URL layout matches the Phase 3 read path. After a
successful push, the same base URL works with existing `verify` / `cat` /
`doctor` / `mount --source`.

## Usage

```bash
chunkforge push \
  --store <local-cas> \
  --dest 'https://example/mybucket' \
  [--url-template '{base}/{path}'] \
  [--prefix 'data/'] \
  [--header 'Authorization: Bearer {env:TOKEN}'] \
  [--jobs N] \
  [--http-retries N] \
  [--http-retry-backoff-ms MS] \
  [--dry-run] \
  [--verify] \
  [--format text|json] \
  [--path P]... [--exclude PAT]... [--exclude-from FILE]... \
  listing1.cfidx|.cfdir [listing2 ...]
```

| Flag | Meaning |
|---|---|
| `--store` | Local CAS providing plaintext chunk bytes (`Store::get`) |
| `--dest` | HTTP(S) base URL (required shape for the remote write face) |
| `--url-template` / `--prefix` / `--header` | Same closed placeholders as read-side `HttpChunkSource` (see [remote-layout.md](remote-layout.md)) |
| `--jobs N` | Bounded concurrency for has/PUT (default **1** = serial; suggested ≤16); also used for post-push `--verify` fetches |
| `--http-retries N` | Extra attempts after the first try for transient HTTP failures (default **0** ≡ 0.7.0). Wired into `RetryPolicy` on the HTTP sink/source. |
| `--http-retry-backoff-ms MS` | Base backoff for retries (default **100**; exponential + jitter, capped at 2s) |
| `--dry-run` | Probe + count only; **no** PUT |
| `--verify` | After a successful push (`failed=0`), treat `--dest` (+ same templates) as a `ChunkSource` and run verify for **each** listing (`.cfidx` / `.cfdir`). Any verify failure → overall non-zero. Skipped on `--dry-run` (nothing uploaded) and when push already failed. |
| `--format` | `text` (default ≡ **1.0.0** stderr summary) or `json` (one object on **stdout**; no duplicate stderr summary / per-id fail lines). Exit codes are format-independent |
| `--path P` | Include only `.cfdir` **File** paths under prefix `P` (repeatable; OR). With any `--path`, a candidate must match at least one before excludes. Omit all ⇒ include-all (≡ **1.3.0** full reference set). Does **not** upload the listing. With `.cfidx` → clear non-zero error. |
| `--exclude PAT` | Exclude matching File paths (repeatable): exact, trailing `/` directory prefix, or single edge `*` (`*.o`, `temp*`). Illegal middle `*` / `**` → clear error. Applied after `--path`. With `.cfidx` → clear non-zero error. |
| `--exclude-from FILE` | Repeatable UTF-8 file of `--exclude` patterns (blank / `#` skipped, trim). Union with CLI `--exclude` → one `PathFilter`. Missing file or illegal line → clear non-zero. With `.cfidx` → clear non-zero error. |
| listings | One or more `.cfidx` / `.cfdir` files; chunk id set is the **union** (after path filter for `.cfdir` Files; Dir entries never contribute) |

Default URL template is `{base}/{path}` ≡ `{base}/chunks/<2hex>/<62hex>.cnk`,
identical to Phase 2/3 GET layout.

### What push does

1. Load and validate every listing (`.cfidx` or `.cfdir`); merge referenced `ChunkId`s. With `--path`/`--exclude`/`--exclude-from`, only matching `.cfdir` **File** entries contribute (Dir entries never do); empty flags ≡ full set (≡ 1.3.0). Listing files themselves are **not** uploaded. `.cfidx` + any path/exclude flag → clear non-zero error.
2. For each id (sorted): read plaintext from the local store; remote `has`
   (HEAD, GET fallback) → skip; otherwise `PUT` the body.
3. Emit a summary (`--format text`, default ≡ **1.0.0**): stderr line
   `push: skipped=… uploaded=… failed=… failed_transient=… failed_permanent=… retries=… (N unique chunk ids, M listings, dry_run=…)`. Missing/Corrupt roll into `failed_permanent`; see [http-retry.md](http-retry.md). With **`--format json`**: one JSON object on **stdout** (no duplicate stderr summary; per-id `push: fail` lines omitted — counts are in the object).
4. Exit **non-zero** if `failed > 0` (independent of `--format`).
5. If `--verify` and push succeeded and not `--dry-run`: build `HttpChunkSource` from
   `--dest` (same templates) and run the same verify path as `verify --source` for
   each listing arg. Verify failure → non-zero (useful error includes listing path /
   chunk id). On success the user need not run a separate `verify --source`.

### `--format json` (stdout; Phase 11 M3)

One JSON **object** on stdout (emitted even when `failed > 0`, then non-zero exit). Field rename is **breaking**.

| Field | Type | Meaning |
|---|---|---|
| `ok` | bool | `failed == 0` |
| `skipped` | number | Already present remotely (or PUT 409 → skip) |
| `uploaded` | number | PUT written (or dry-run would-upload) |
| `failed` | number | `failed_transient + failed_permanent` |
| `failed_transient` | number | Transient HTTP class |
| `failed_permanent` | number | Permanent / missing / corrupt |
| `retries` | number | `--http-retries` value (configured max extra attempts) |
| `unique_chunks` | number | Union of listing chunk ids **after** `--path`/`--exclude` (filtered set; field name unchanged) |
| `listings` | number | Listings successfully loaded |
| `dry_run` | bool | `--dry-run` was set |

```json
{"ok":true,"skipped":0,"uploaded":3,"failed":0,"failed_transient":0,"failed_permanent":0,"retries":0,"unique_chunks":3,"listings":1,"dry_run":false}
```

### `--path` / `--path-from` / `--exclude` / `--exclude-from` (Phase 14 + Phase 20)

**`path-from` ≠ listing upload ≠ prune ≠ gc-path ≠ sync ≠ pack:** push
path/exclude only **shrinks the upload set**. It does not upload or rewrite
the listing, does not delete remote extras, and does **not** shrink `gc`'s
reference set. Symmetric to [pull.md](pull.md) path filtering.

Optional, repeatable, **opt-in**. Default (no path/exclude flags) ≡ **1.9.0** /
**1.3.0** full reference set.

| Rule | Detail |
|---|---|
| `--path P` | Hit iff `path == P` or `path` starts with `P/` (subtree) |
| `--path-from FILE` | Phase 20 opt-in: UTF-8 one include prefix per line (≡ `--path`); blank/`#`/trim; merged with `--path` (OR). Missing file → non-zero |
| `--exclude` | Exact; trailing `/` directory prefix; single edge `*` only (`*.o`, `temp*`) — **no** `**` / middle `*` |
| `--exclude-from` | Same patterns from a file; merged with `--exclude`. May combine with `--path-from` |
| Combine | If any `--path` / `--path-from` is given: must hit include first, then excludes reject |
| Scope | Only `.cfdir` **File** entries contribute chunk ids; **Dir** entries never do |
| `.cfidx` | Any path/exclude flag (incl. `--path-from`) → **non-zero clear error** (no File paths; no silent full upload) |
| Listing | Full listing is read locally; push still does **not** upload the listing |
| Orthogonal | `--dry-run` / `--format` / `--jobs` / `--progress` / retries / SigV4 / `--verify` do **not** change match rules |
| JSON | Field names unchanged; `unique_chunks` = filtered unique id count |

```bash
# Only push chunks for packages/foo (subset of a full .cfdir)
chunkforge push --store ./store --dest http://127.0.0.1:8765 \
  --path packages/foo --format json ./app.cfdir
```

### Concurrency (`--jobs`)

`cat` / `verify` / `doctor` / `push` accept `--jobs N` (default **1**). `N=1`
keeps the Phase 3 serial orchestration on the calling thread. `N>1` uses a
bounded `std::thread::scope` worker pool in the CLI only — `ChunkSource` /
`ChunkSink` stay synchronous; no tokio. Failures still name the chunk id.
FUSE `mount` is unchanged (no per-read thread storm).

### What push does **not** do

| Non-goal | Detail |
|---|---|
| ❌ Upload `.cfidx` / `.cfdir` | Listings stay local / are published by the user separately |
| ❌ Silent full upload on `.cfidx` + `--path` | Clear non-zero error instead |
| ❌ `gc --path` / prune / pack | Path filter is **not** GC scope, prune, or pack |
| ❌ Remote GC / delete | Extra remote objects are left alone |
| ❌ Bidirectional sync | Explicit one-way publish only |
| ❌ S3 multipart API | Chunks are ≤256KiB; single-object PUT is enough |
| ❌ `aws-sdk-*` / in-process SigV4 | ureq + template headers / external presign only |

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
| `--verify` + `--dry-run` | Verify is **skipped** (stderr note); dry-run never pretends the remote is verified |
| `--verify` after push failures | Skipped; push already exits non-zero |
| `--verify` remote missing/corrupt chunk | Non-zero; error names the listing / chunk |

Push does not panic on remote rejection. Partial progress may leave some chunks
on the remote; re-run is safe (see idempotency).

## No listing upload (`.cfidx` / `.cfdir`)

The object layout under `--dest` holds **chunks only**
(`chunks/<2hex>/<62hex>.cnk`, optionally under `{prefix}`). The listing file
(`.cfidx` or `.cfdir`) is an out-of-band artifact: copy it next to releases,
store it in git, or serve it from a different URL. `verify --source <dest>
release.cfdir` still needs the listing **locally** (or wherever you pass the
path); only chunk bodies are fetched from the remote.

## Idempotency

Safe to re-run:

1. Default sink probes with `has` first → already-present ids become `skipped`.
2. Optional `409 Conflict` on PUT also counts as skip.
3. Hash is verified locally (`blake3(plain) == id`) before PUT when the sink’s
   `verify_hash` is on (CLI default).

Expect a second push with the same store + listings against an unchanged remote:

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
  --verify \
  /tmp/cf-p4/hello.cfidx
# Equivalent separate step (not needed when --verify succeeds):
# ./target/debug/chunkforge verify --source http://127.0.0.1:8766 /tmp/cf-p4/hello.cfidx
```


## `.cfdir` push (Phase 5)

Same flags; pass a directory archive instead of (or mixed with) `.cfidx`:

```bash
chunkforge archive --store ./store -o release.cfdir ./src
chunkforge push --store ./store --dest http://127.0.0.1:8766 --verify release.cfdir
```

Full local walkthrough: [`scripts/demo_archive.sh`](../scripts/demo_archive.sh)
(see also [archive.md](archive.md)).

## Related

- Layout + PUT key conventions: [remote-layout.md](remote-layout.md)
- Read-side templates / auth: same document, HTTP GET section
- CLI library face: `HttpChunkSink` in `chunkforge-remote` (isomorphic with `HttpChunkSource`)
- Directory archive workflow: [archive.md](archive.md)
