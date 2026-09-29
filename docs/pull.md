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
| listings | One or more `.cfidx` / `.cfdir` files; chunk id set is the **union** |

### What pull does

1. Load and validate every listing (`.cfidx` or `.cfdir`); merge referenced `ChunkId`s.
2. For each id (sorted): local `store.has` → **skip**; otherwise `source.get` →
   `store.put` (plaintext into the local CAS; hash checked on put).
3. Print a summary on stderr:
   `pull: skipped=… fetched=… failed=… retries=… (N unique chunk ids, M listings, dry_run=…)`.
4. Exit **non-zero** if `failed > 0`.

### What pull does **not** do

| Non-goal | Detail |
|---|---|
| ❌ Extract a file tree | Use `extract` after the CAS is filled |
| ❌ Delete extra local chunks | That is `gc` |
| ❌ Download / upload the listing | Listings stay local out-of-band artifacts |
| ❌ Bidirectional sync / watch | Explicit one-way fill only |
| ❌ Remote GC / packfiles / SigV4 | Same posture as `push` / `verify` |

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
