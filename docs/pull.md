# `chunkforge pull`

Phase 6 P1: fill a **local CAS** `--store` with plaintext chunks referenced by
one or more `.cfidx` / `.cfdir` listings, reading missing ids from `--source`.
Symmetric to [`push`](push.md) (store → HTTP dest), but **source → store**.

`pull` only guarantees the local CAS has the referenced chunks. Materializing
a file tree still uses `extract`; mounting still uses `mount`.

## Usage

```bash
chunkforge pull \
  --store <local-cas> \
  --source <PATH|URL> \
  [--url-template '{base}/{path}'] \
  [--prefix 'data/'] \
  [--header 'Authorization: Bearer {env:TOKEN}'] \
  [--jobs N] \
  [--http-retries N] \
  [--dry-run] \
  [--format text|json] \
  [--path P]... [--exclude PAT]... \
  listing1.cfidx|.cfdir [listing2 ...]
```

| Flag | Meaning |
|---|---|
| `--store` | Local CAS to **write** (created if missing; not written in `--dry-run`) |
| `--source` | Chunk source: local store path, `file://`, or `http(s)://` |
| `--url-template` / `--prefix` / `--header` | Same closed placeholders as read-side `HttpChunkSource` (HTTP sources only; see [remote-layout.md](remote-layout.md)) |
| `--http-retries N` | Extra HTTP attempts for transient failures (default **0**; HTTP sources only; ignored for local/`file://`) |
| `--jobs N` | Bounded concurrency for has/get/put (default **1** = serial) |
| `--dry-run` | Probe + count only; **no** store writes (does not create `meta.toml`) |
| `--format` | `text` (default ≡ **1.0.0** stderr summary) or `json` (one object on **stdout**; no duplicate stderr summary / per-id fail lines). Exit codes are format-independent |
| `--path P` | Include only `.cfdir` **File** paths under prefix `P` (repeatable; OR). With any `--path`, a candidate must match at least one before excludes. Omit all ⇒ include-all (≡ **1.2.0** full reference set). Does **not** download or alter the listing |
| `--exclude PAT` | Exclude matching File paths (repeatable): exact, trailing `/` directory prefix, or single edge `*` (`*.o`, `temp*`). Illegal middle `*` / `**` → clear error. Applied after `--path` |
| listings | One or more `.cfidx` / `.cfdir` files; chunk id set is the **union** (after path filter for `.cfdir` Files; Dir entries never contribute) |

### What pull does

1. Load and validate every listing (`.cfidx` or `.cfdir`); merge referenced `ChunkId`s. With `--path`/`--exclude`, only matching `.cfdir` **File** entries contribute (Dir entries never do); empty flags ≡ full set (≡ 1.2.0). Listing files themselves are not downloaded.
2. For each id (sorted): local `store.has` → **skip**; otherwise `source.get` →
   `store.put` (plaintext into the local CAS; hash checked on put).
3. Emit a summary (`--format text`, default ≡ **1.0.0**): stderr line
   `pull: skipped=… fetched=… failed=… failed_transient=… failed_permanent=… retries=… (N unique chunk ids, M listings, dry_run=…)`. Missing/Corrupt roll into `failed_permanent`; see [http-retry.md](http-retry.md). With **`--format json`**: one JSON object on **stdout** (no duplicate stderr summary; per-id `pull: fail` lines omitted — counts are in the object).
4. Exit **non-zero** if `failed > 0` (independent of `--format`).

### `--format json` (stdout; Phase 11 M3)

One JSON **object** on stdout (emitted even when `failed > 0`, then non-zero exit). Same shape as `push` JSON except **`fetched`** replaces `uploaded`. Field rename is **breaking**.

| Field | Type | Meaning |
|---|---|---|
| `ok` | bool | `failed == 0` |
| `skipped` | number | Already present in local `--store` |
| `fetched` | number | Source get + store put (or dry-run would-fetch) |
| `failed` | number | `failed_transient + failed_permanent` |
| `failed_transient` | number | Transient HTTP class |
| `failed_permanent` | number | Permanent / missing / corrupt |
| `retries` | number | `--http-retries` value (configured max extra attempts) |
| `unique_chunks` | number | Union of listing chunk ids **after** `--path`/`--exclude` (filtered set; field name unchanged) |
| `listings` | number | Listings successfully loaded |
| `dry_run` | bool | `--dry-run` was set |

```json
{"ok":true,"skipped":0,"fetched":3,"failed":0,"failed_transient":0,"failed_permanent":0,"retries":0,"unique_chunks":3,"listings":1,"dry_run":false}
```

### `--path` / `--exclude` (Phase 13 M4)

Optional, repeatable, **opt-in**. Default (no flags) ≡ **1.2.0** full reference set.

| Rule | Detail |
|---|---|
| `--path P` | Hit iff `path == P` or `path` starts with `P/` (subtree) |
| `--exclude` | Exact; trailing `/` directory prefix; single edge `*` only (`*.o`, `temp*`) — **no** `**` / middle `*` |
| Combine | If any `--path` is given: must hit include first, then excludes reject |
| Scope | Only `.cfdir` **File** entries contribute chunk ids; **Dir** entries never do |
| Listing | Full listing is read locally; pull still does **not** download or rewrite the listing |
| Orthogonal | `--dry-run` / `--format` / `--jobs` / `--progress` / retries / SigV4 do **not** change match rules |
| JSON | Field names unchanged; `unique_chunks` = filtered unique id count |

```bash
# Only pull chunks for packages/foo (subset of a full .cfdir)
chunkforge pull --store ./store2 --source http://127.0.0.1:8766 \
  --path packages/foo --format json ./app.cfdir
```

### What pull does **not** do

| Non-goal | Detail |
|---|---|
| ❌ Extract a file tree | Use `extract` after the CAS is filled |
| ❌ Delete extra local chunks | That is `gc` |
| ❌ Download / upload the listing | Listings stay local out-of-band artifacts |
| ❌ Bidirectional sync / watch | Explicit one-way fill only |
| ❌ Extract / prune / rewrite listing | Path filter only shrinks the fetch set |
| ❌ Remote GC / packfiles | Same posture as `push` / `verify` |

## End-to-end (push stub → empty store → pull → verify)

```bash
cargo build -p chunkforge-cli
mkdir -p /tmp/cf-pull/{store,mirror,store2}
./target/debug/chunkforge make --store /tmp/cf-pull/store \
  -o /tmp/cf-pull/hello.cfidx ./fixtures/hello.txt

# terminal 1
python3 ./scripts/put_stub.py --root /tmp/cf-pull/mirror --port 8766

# terminal 2 — publish chunks (not the listing)
./target/debug/chunkforge push --store /tmp/cf-pull/store \
  --dest http://127.0.0.1:8766 --verify /tmp/cf-pull/hello.cfidx

# empty local CAS + listing only → pull missing chunks
rm -rf /tmp/cf-pull/store2 && mkdir -p /tmp/cf-pull/store2
./target/debug/chunkforge pull --store /tmp/cf-pull/store2 \
  --source http://127.0.0.1:8766 /tmp/cf-pull/hello.cfidx
./target/debug/chunkforge verify --store /tmp/cf-pull/store2 /tmp/cf-pull/hello.cfidx

# dry-run against an empty / missing store: would fetch, write nothing
./target/debug/chunkforge pull --store /tmp/cf-pull/store3 \
  --source http://127.0.0.1:8766 --dry-run /tmp/cf-pull/hello.cfidx
# stderr: … fetched=N … dry_run=true; no meta.toml / .cnk under store3

# second pull is idempotent
./target/debug/chunkforge pull --store /tmp/cf-pull/store2 \
  --source http://127.0.0.1:8766 /tmp/cf-pull/hello.cfidx
# stderr: skipped=N fetched=0 failed=0 …
```

Directory archives work the same way (pass a `.cfdir`).

## Failure semantics

| Situation | Behaviour |
|---|---|
| Listing unreadable / invalid | Abort before fetches (non-zero) |
| Source missing a referenced chunk | Count as **failed**; print `pull: fail <id>: …`; continue other ids; exit non-zero |
| Local put / hash mismatch | **failed** for that id; continue; exit non-zero |
| Already present in `--store` | **skipped** |
| `--dry-run` | Counts would-be fetches in `fetched=`; **zero** store writes |
| HTTP template flags on non-HTTP `--source` | Immediate readable error |

Partial progress may leave some chunks in `--store`; re-run is safe (existing ids skip).

## Related

- Inverse write path: [push.md](push.md)
- HTTP layout / templates: [remote-layout.md](remote-layout.md)
- Directory archive workflow: [archive.md](archive.md)
