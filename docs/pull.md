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
  [--fallback <PATH|URL>]... \
  [--cache DIR] [--cache-max-bytes SIZE] [--cache-stats] \
  [--url-template '{base}/{path}'] \
  [--prefix 'data/'] \
  [--header 'Authorization: Bearer {env:TOKEN}'] \
  [--jobs N] \
  [--http-retries N] \
  [--dry-run] \
  [--verify] \
  [--progress] \
  [--format text|json] \
  [--path P]... [--exclude PAT]... [--exclude-from FILE]... \
  listing1.cfidx|.cfdir [listing2 ...]
```

| Flag | Meaning |
|---|---|
| `--store` | Local CAS to **write** (created if missing; not written in `--dry-run`) |
| `--source` | Chunk source: local store path, `file://`, or `http(s)://` |
| `--fallback` | Repeatable extra origin tried **only on Missing** (CLI order). Zero times ≡ **1.5** single origin. Fills **`--store`** from the failover chain; **≠ cache fill of a separate cache dir ≠ sync ≠ prune**. Does **not** apply to `push --dest` |
| `--url-template` / `--prefix` / `--header` | Same closed placeholders as read-side `HttpChunkSource` (HTTP sources only; see [remote-layout.md](remote-layout.md)) |
| `--http-retries N` | Extra HTTP attempts for transient failures (default **0**; HTTP sources only; ignored for local/`file://`) |
| `--jobs N` | Bounded concurrency for has/get/put (default **1** = serial) |
| `--dry-run` | Probe + count only; **no** store writes (does not create `meta.toml`) |
| `--format` | `text` (default ≡ **1.0.0** stderr summary) or `json` (one object on **stdout**; no duplicate stderr summary / per-id fail lines). Exit codes are format-independent |
| `--verify` | After a **successful** pull (and not `--dry-run`), treat local `--store` as a `ChunkSource` and run the same verify path used by `chunkforge verify` on each listing (symmetric to `push --verify`, which verifies `--dest`). Default **off** ≡ 1.7. Dry-run or pull already failed → **skip** verify with a clear stderr note. Verify failure → **non-zero** even if fetch counts were ok. Does **not** reshape `--format json` fields (verify chatter stays on stderr). Orthogonal to `--jobs` / `--progress` / path filter / `--fallback` / `--cache` |
| `--progress` | Opt-in stderr `progress: op=pull done=N/TOTAL` per chunk (default **off**) |
| `--cache` / `--cache-max-bytes` / `--cache-stats` | Optional local cache ahead of `--source` (fill on miss; soft budget refuse-fill). `--cache-stats` emits `cache: hits=…` on stderr (requires `--cache`; ≠ LRU). With `--cache` + `--format json`, additive `cache_*` fields (see [ops-json.md](ops-json.md)) |
| `--path P` | Include only `.cfdir` **File** paths under prefix `P` (repeatable; OR). With any `--path`, a candidate must match at least one before excludes. Omit all ⇒ include-all (≡ **1.2.0** full reference set). Does **not** download or alter the listing |
| `--exclude PAT` | Exclude matching File paths (repeatable): exact, trailing `/` directory prefix, or single edge `*` (`*.o`, `temp*`). Illegal middle `*` / `**` → clear error. Applied after `--path` |
| `--exclude-from FILE` | Repeatable UTF-8 file of `--exclude` patterns (blank / `#` skipped, trim). Union with CLI `--exclude` → one `PathFilter`. Missing/unreadable file or illegal line → clear non-zero |
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

### `--verify` (Phase18 / 1.8 opt-in)

Symmetric to [`push --verify`](push.md): after a **successful** fetch into
local `--store`, re-open that store as a `ChunkSource` and verify each listing
(`.cfidx` / `.cfdir`) with the same integrity path as `chunkforge verify`
(structure + referenced chunk presence/hash/length + blob_blake3).

| Rule | Detail |
|---|---|
| Default | **Off** ≡ **1.7.0** (no post-verify; no verify noise) |
| Success path | Pull `failed == 0` and not `--dry-run` → verify each listing against `--store` |
| Skip | `--dry-run`, or pull already failed (`failed > 0`) → clear stderr skip note; do **not** claim verify ok |
| Failure | Any listing verify failure → **non-zero** exit (even if fetch counters looked successful) |
| JSON | Does **not** add/rename fields; JSON object is emitted for the pull itself; verify chatter is stderr-only |
| Orthogonal | `--format` / `--jobs` / `--progress` / path filter / `--fallback` / `--cache` / `--cache-stats` |
| ≠ sync | One-way post-check only; not bidirectional sync, not prune, not extract |

```bash
# Fill empty store from local primary, then verify the listing against --store
chunkforge pull --store ./store2 --source ./store --verify ./blob.cfidx
# stderr: pull: verifying 1 listing against --store …
#         verify: ok (…)
#         pull: verify ok (1 listing)
```

### `--path` / `--exclude` / `--exclude-from` (Phase 13 M4 / Phase 14 M4)

**`path` ≠ prune ≠ sync; `fallback` ≠ cache ≠ sync:** pull path/exclude only **shrinks the fetch set**. `--fallback` only adds Missing-only read origins for the fetch (still one-way into `--store`). It does not extract a tree, does not delete local extras, and does not rewrite the listing. Extract path filtering is likewise non-prune — see [extract.md](extract.md).

Optional, repeatable, **opt-in**. Default (no `--path` / `--exclude` / `--exclude-from`) ≡ **1.2.0** full reference set.

| Rule | Detail |
|---|---|
| `--path P` | Hit iff `path == P` or `path` starts with `P/` (subtree) |
| `--exclude` | Exact; trailing `/` directory prefix; single edge `*` only (`*.o`, `temp*`) — **no** `**` / middle `*` |
| `--exclude-from` | Same patterns from a file; merged with `--exclude`. No `--path-from` |
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
| ❌ Bidirectional sync / watch | Explicit one-way fill only; `--fallback` is **not** sync |
| ❌ Extract / prune / rewrite listing | Path filter only shrinks the fetch set |
| ❌ Treating `--fallback` as a disk cache | Fallback never writes origins; pull writes only `--store` |
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
- Path-filter smoke: [`scripts/demo_path_filter.sh`](../scripts/demo_path_filter.sh)
- Fallback / suffix / `bytes_plaintext` smoke: [`scripts/demo_fallback_bytes_suffix.sh`](../scripts/demo_fallback_bytes_suffix.sh)
- Ops JSON: [ops-json.md](ops-json.md)
- Phase18 smoke (`pull --verify` / cache-stats / cat·verify `--progress`): [`scripts/demo_pull_verify_cache_stats.sh`](../scripts/demo_pull_verify_cache_stats.sh)
