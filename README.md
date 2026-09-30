# ChunkForge

**ChunkForge**: FastCDC content-defined chunking + BLAKE3 content addressing to split large files/blobs into deduplicable chunks, write them into a local CAS store, and reassemble via a custom `.cfidx` index — with on-demand fetch (`ChunkSource`), object-store–friendly HTTP templates, per-chunk HTTP **PUT** (`ChunkSink` / `push`), **directory-tree archive** (`.cfdir` / `archive` / `extract`), and **read-only** FUSE mount of a single blob **or** a directory tree (with sequential prefetch).

## Status

| Phase | Version | Delivered |
|---|---|---|
| **Phase 1** | **0.1.0** | Local chunk / store / index / CLI (`make` `cat` `verify` `chunk-id` `store has`) / fixtures + dedup demo / CI |
| **Phase 2** | **0.2.0** | `ChunkSource` + HTTP/`file://` remote + `--source`/`--cache` + read-only `mount` |
| **Phase 3** | **0.3.0** | URL/header templates + S3 path conventions; `doctor`; local `gc` dry-run / `--apply` |
| **Phase 4** | **0.4.0** | `ChunkSink` + `HttpChunkSink` per-chunk PUT; CLI `push`; `--jobs` on cat/verify/doctor/push |
| **Phase 5** | **0.5.0** | `.cfdir` v1 directory archive + `DirFs` RO mount; `archive` / `extract` / tree `verify`; `push`/`doctor`/`gc` accept `.cfdir`; `push --verify`; `archive --dry-run` |
| **Phase 6** | **0.6.0** | `archive --seed` incremental reuse; `chunkforge pull` CAS fill; `archive --jobs`; `scripts/demo_seed.sh` |
| **Phase 7** | **0.7.0** | `chunkforge diff` (+ `--tree`); `store scrub`; `archive --seed-trust-mtime`; `extract --force`; `scripts/demo_diff_scrub.sh` |
| **Phase 8** | **0.8.0** | HTTP `--http-retries` + error-class summaries; `diff --format json`; minimal `--aws-sigv4`; `scripts/demo_http_retry.sh` |
| **Phase 9** | **0.9.0** | `extract --skip-unchanged` / `--dry-run`; loose HTTP perf baseline; SigV4 shared-creds fallback; `scripts/demo_extract_skip.sh` |
| **Phase 10** | **1.0.0** | FUSE sequential prefetch (`--no-prefetch`) + 1.0 stability freeze (`docs/stability.md`); `verify`/`doctor --format json` |
| **Phase 11** | **1.1.0** | `extract --skip-trust-mtime` + `extract`/`push`/`pull --format json`; P1 `mount --prefetch-chunks N` |

## Non-goals (Phase 10 / 1.0)

| Not this | Why |
|---|---|
| ❌ **Full AWS/S3 SDK** (`aws-sdk-*` / `aws-config` / ListObjects / IMDS / SSO) | HTTP stays **ureq**; optional minimal SigV4 via `--aws-sigv4` (env + shared credentials file; see `docs/sigv4.md`) |
| ❌ **Complete S3 multipart upload API** | Chunks ≤256KiB; **single-object PUT** only |
| ❌ **Packfile / multi-chunk single object** | Loose `.cnk` layout unchanged; [docs/perf.md](docs/perf.md) measures only — pack **not** implemented |
| ❌ **Write mount / COW / writable FUSE** | FUSE stays `RO` (blob **and** tree); prefetch is **not** write-back; writes return `EROFS` / `EACCES` |
| ❌ **Bidirectional sync / watch dirs / conflict resolution** | `diff` is **not** sync; `extract --skip-unchanged` is **not** sync |
| ❌ **Extract prune / `--delete`** | `extract` never removes extra files under `-o` |
| ❌ **Remote scrub / remote GC / bucket lifecycle** | Referenced remote integrity → `verify --source`; presence → `doctor`; `gc` / `store scrub` stay **local** `--store` only |
| ❌ **Byte-range / partial-chunk HTTP resume** | Retries **whole chunks** only (chunks ≤256KiB) |
| ❌ **`push` uploads listings** | Chunks only; `.cfdir` / `.cfidx` stay out-of-band (git / release artifact) |
| ❌ **Change default `--jobs` / `--http-retries`** | Stay **jobs=1**, **retries=0** (≡ 0.9.0) |
| ❌ **Tokio as default runtime** | Keep `std::thread` + ureq; mount prefetch may sync-get the next chunk on the call thread |
| ❌ **Rewrite / abandon `.cfidx` v1 or `.cfdir` v1** | Prefetch / stability / optional JSON do **not** bump magic |
| ❌ **Full POSIX fidelity / symlink recording** | Symlinks still skipped + warned |
| ❌ **casync `.catar` / `.caibx` bit-compat** | Semantic alignment only; native `.cfdir` / `.cfidx` |
| ❌ **P2P** / **GPU / LLM** / video analysis | Pure CPU data plane; no device discovery |
| ❌ Not a restic/rustic-style **backup product** | No snapshot policy, encrypted-repo lifecycle, or prune |
| ❌ macOS / Windows as acceptance platforms | Linux + fuse3 is first-class; other OS are experimental / unsupported |

Earlier phases delivered local CAS (Phase 1), remote read + RO single-blob mount (Phase 2), templates / doctor / gc (Phase 3), per-chunk PUT / `push` / `--jobs` (Phase 4), multi-file `.cfdir` + DirFs (Phase 5), incremental `archive --seed` + `pull` (Phase 6), listing **`diff`** / **`store scrub`** (Phase 7), HTTP **`--http-retries`** / **`diff --format json`** / minimal **`--aws-sigv4`** (Phase 8), **`extract --skip-unchanged`** / **`--dry-run`** + loose perf baseline + SigV4 shared-creds (Phase 9), and FUSE sequential prefetch + 1.0 stability freeze (Phase 10 / **1.0.0**). **Phase 11 is closed at 1.1.0**: `extract --skip-trust-mtime` + ops JSON + `--prefetch-chunks` — see [docs/stability.md](docs/stability.md).

## Quick start (local CAS)

```bash
# Requires Rust 1.85+ (edition 2024)
cargo build -p chunkforge-cli
./target/debug/chunkforge make --store ./store -o v1.cfidx ./fixtures/hello.txt
./target/debug/chunkforge verify --store ./store v1.cfidx
./target/debug/chunkforge cat --store ./store v1.cfidx -o /tmp/hello.out
cmp ./fixtures/hello.txt /tmp/hello.out
```

Or via Make: `make build` then the same `make` / `cat` / `verify` flow above.

## Phase 2: remote source + read-only mount

Chunks are read through `ChunkSource`: local store, `file://`, or HTTP static directory
(`GET {base}/chunks/ab/<62hex>.cnk`). Optional `--cache` fills a local store on miss.

```bash
# Remote verify / cat (HTTP static store root — same layout as local CAS)
python3 -m http.server 8765 --directory ./store &
./target/debug/chunkforge verify --source http://127.0.0.1:8765 v1.cfidx
./target/debug/chunkforge cat --source http://127.0.0.1:8765 --cache ./cache \
  v1.cfidx -o /tmp/hello.http.out
```

Linux + fuse3: mount a `.cfidx` as a **single** read-only file (directory trees via `.cfdir` — see Phase 5 below):

```bash
cargo build -p chunkforge-cli          # fuse feature on by default
mkdir -p ./mnt
./target/debug/chunkforge mount --store ./store v1.cfidx ./mnt
# other terminal: cmp ./fixtures/hello.txt ./mnt/v1
# HTTP + cache:
#   chunkforge mount --source http://127.0.0.1:8765 --cache ./cache v1.cfidx ./mnt
fusermount3 -u ./mnt
./scripts/demo_mount.sh                # local-store smoke: cmp + write-fail + unmount
```

Details: [docs/mount.md](docs/mount.md). Remote chunk layout: [docs/remote-layout.md](docs/remote-layout.md).

## Phase 3: templates, doctor, local gc

Object-store–friendly **read** paths: same `HttpChunkSource`, optional URL/header templates and key prefix. Default (no flags) stays Phase 2–compatible.

```bash
# Explicit default template (≡ bare --source http://…)
./target/debug/chunkforge verify \
  --source http://127.0.0.1:8765 \
  --url-template '{base}/{path}' \
  v1.cfidx

# Path-style “bucket + subdirectory” (serve ./mirror as docroot with store under data/)
# ./target/debug/chunkforge verify \
#   --source http://127.0.0.1:PORT \
#   --prefix 'data/' \
#   --url-template '{base}/{prefix}{path}' \
#   v1.cfidx

# Header auth via env (do not put real secrets in examples)
# export CF_TOKEN=demo
# ./target/debug/chunkforge cat --source http://127.0.0.1:8765 \
#   --header 'Authorization: Bearer {env:CF_TOKEN}' \
#   v1.cfidx -o /tmp/hello.out

# doctor: missing chunks → non-zero + ids on stdout
./target/debug/chunkforge doctor --store ./store v1.cfidx

# gc: dry-run unreferenced loose .cnk; --apply to delete (local store only)
./target/debug/chunkforge gc --store ./store v1.cfidx
# ./target/debug/chunkforge gc --store ./store v1.cfidx --apply
```

Placeholders, path-style / virtual-host examples, and auth patterns: [docs/remote-layout.md](docs/remote-layout.md). Doctor / gc details: [docs/doctor-gc.md](docs/doctor-gc.md).

## Phase 4: push (per-chunk PUT) + `--jobs`

Symmetric write path for the same HTTP key layout as Phase 3 reads: `ChunkSink` /
`HttpChunkSink` issue **single-object PUT** of plaintext chunks. CLI `push` uploads
missing ids from a local `--store` + listing set (`.cfidx` and, since **0.5.0**,
`.cfdir`) to `--dest`. Optional `--jobs N` (default **1** ≡ serial) speeds
`cat` / `verify` / `doctor` / `push`. Post-push check: **`push --verify`**
(Phase 5) or a separate `verify --source`.

```bash
# Local PUT stub + push + verify (or: bash scripts/demo_push.sh)
# Terminal 1:
#   python3 scripts/put_stub.py --root /tmp/cf-p4/mirror --port 8766
mkdir -p /tmp/cf-p4 && cd /tmp/cf-p4
chunkforge make --store ./store -o hello.cfidx /path/to/ChunkForge/fixtures/hello.txt
chunkforge push --store ./store --dest http://127.0.0.1:8766 hello.cfidx
chunkforge verify --source http://127.0.0.1:8766 hello.cfidx
chunkforge cat --source http://127.0.0.1:8766 hello.cfidx -o /tmp/hello.p4.out
cmp /path/to/ChunkForge/fixtures/hello.txt /tmp/hello.p4.out

# Idempotent re-push → uploaded=0, skipped≥1
chunkforge push --store ./store --dest http://127.0.0.1:8766 hello.cfidx

# Bounded concurrency (optional)
# chunkforge verify --source http://127.0.0.1:8766 --jobs 4 hello.cfidx
# chunkforge push --store ./store --dest http://127.0.0.1:8766 --jobs 4 hello.cfidx
```

Details: [docs/push.md](docs/push.md). PUT layout + non-goals: [docs/remote-layout.md](docs/remote-layout.md). Smoke: [`scripts/demo_push.sh`](scripts/demo_push.sh).

## Phase 5: directory archive (`.cfdir`) + DirFs

Multi-file workflow on a **new** `.cfdir` v1 listing (`.cfidx` v1 stays frozen /
single-blob). `archive` chunks a directory into the existing CAS; `extract` /
tree `verify` / read-only `mount` consume it; `push` / `doctor` / `gc` accept
`.cfdir` the same way as `.cfidx`.

```bash
cargo build -p chunkforge-cli

mkdir -p /tmp/cf-p5/src/sub
echo 'hello-tree' > /tmp/cf-p5/src/a.txt
cp fixtures/hello.txt /tmp/cf-p5/src/sub/b.txt
cp /tmp/cf-p5/src/a.txt /tmp/cf-p5/src/a-copy.txt   # cross-file dedup

./target/debug/chunkforge archive \
  --store /tmp/cf-p5/store -o /tmp/cf-p5/release.cfdir /tmp/cf-p5/src
# ./target/debug/chunkforge archive --dry-run --store /tmp/cf-p5/store \
#   -o /tmp/cf-p5/unused.cfdir /tmp/cf-p5/src   # stats only

./target/debug/chunkforge verify --store /tmp/cf-p5/store /tmp/cf-p5/release.cfdir
./target/debug/chunkforge extract --store /tmp/cf-p5/store \
  /tmp/cf-p5/release.cfdir -o /tmp/cf-p5/out
diff -qr /tmp/cf-p5/src /tmp/cf-p5/out

# Optional: read-only directory mount (Linux + fuse3)
mkdir -p /tmp/cf-p5/mnt
./target/debug/chunkforge mount --store /tmp/cf-p5/store \
  /tmp/cf-p5/release.cfdir /tmp/cf-p5/mnt
# cmp /tmp/cf-p5/src/a.txt /tmp/cf-p5/mnt/a.txt
# fusermount3 -u /tmp/cf-p5/mnt

# Optional: push chunks + post-push verify
# Terminal 1: python3 scripts/put_stub.py --root /tmp/cf-p5/mirror --port 8766
# ./target/debug/chunkforge push --store /tmp/cf-p5/store \
#   --dest http://127.0.0.1:8766 --verify /tmp/cf-p5/release.cfdir
```

Or one-shot: [`scripts/demo_archive.sh`](scripts/demo_archive.sh). Details:
[docs/archive.md](docs/archive.md), [docs/dir-format.md](docs/dir-format.md),
[docs/mount.md](docs/mount.md), [docs/push.md](docs/push.md).

## Phase 6: seed archive + pull (**0.6.0**)

Incremental directory archive and CAS fill on top of Phase 5 `.cfdir` /
DirFs. Without `--seed`, `archive` matches **0.5.0**.

- **`archive --seed prior.cfdir`**: reuse unchanged files' chunk tables via
  content BLAKE3 (size fast-reject); stderr `seed_reused_files=` /
  `rechunked_files=`; output is still a full `.cfdir` v1 — see
  [docs/archive.md](docs/archive.md)
- **`archive --jobs N`**: per-file parallel chunking (default **1** ≡ serial);
  seed map is read-only; store puts stay atomic
- **`chunkforge pull`**: fill a local `--store` from `--source` for missing
  chunks referenced by `.cfidx` / `.cfdir` — see [docs/pull.md](docs/pull.md)
- Quickstart smoke: [`scripts/demo_seed.sh`](scripts/demo_seed.sh) (~10 min
  local: archive → change one file → `--seed` → verify / extract / optional
  `pull`)

```bash
# Incremental archive (or: bash scripts/demo_seed.sh)
mkdir -p /tmp/cf-p6/src/sub
echo 'hello-seed-v1' > /tmp/cf-p6/src/a.txt
cp fixtures/hello.txt /tmp/cf-p6/src/sub/b.txt
cp /tmp/cf-p6/src/a.txt /tmp/cf-p6/src/a-copy.txt

./target/debug/chunkforge archive \
  --store /tmp/cf-p6/store -o /tmp/cf-p6/v1.cfdir /tmp/cf-p6/src

echo 'hello-seed-v2' > /tmp/cf-p6/src/a.txt
./target/debug/chunkforge archive \
  --store /tmp/cf-p6/store -o /tmp/cf-p6/v2.cfdir \
  --seed /tmp/cf-p6/v1.cfdir /tmp/cf-p6/src
# stderr: seed_reused_files=2, rechunked_files=1

./target/debug/chunkforge verify --store /tmp/cf-p6/store /tmp/cf-p6/v2.cfdir
```


## Phase 7: diff + store scrub (**0.7.0**)

Listing compare and local CAS integrity on top of Phase 6. Without new flags,
`archive` / `extract` / verify / doctor / gc / push / pull match **0.6.0**.

- **`chunkforge diff`**: listing↔listing path-level added / removed / changed /
  meta_changed + chunk-set stats; `--max-paths N`; exit like `diff(1)` — see
  [docs/diff.md](docs/diff.md)
- **`diff --tree <src-dir> <listing.cfdir>`**: tree↔listing (read-only; no
  store / `.cfdir` writes)
- **`chunkforge store scrub`**: re-BLAKE3 every loose `.cnk` in a local
  `--store`; report `ok=` / `corrupt=` / `unreadable=` — see
  [docs/doctor-gc.md](docs/doctor-gc.md)
- **`archive --seed-trust-mtime`**: opt-in size+mtime reuse (default off;
  content BLAKE3 otherwise) — see [docs/archive.md](docs/archive.md)
- **`extract --force`**: overwrite existing regular files at the destination
- Quickstart smoke: [`scripts/demo_diff_scrub.sh`](scripts/demo_diff_scrub.sh)
  (~10 min local: two archives → `diff` / `--tree` → healthy scrub → corrupt
  one `.cnk` → scrub non-zero)

```bash
# Diff + scrub (or: bash scripts/demo_diff_scrub.sh)
mkdir -p /tmp/cf-p7/src/sub
echo 'hello-diff-v1' > /tmp/cf-p7/src/a.txt
cp fixtures/hello.txt /tmp/cf-p7/src/sub/b.txt

./target/debug/chunkforge archive \
  --store /tmp/cf-p7/store -o /tmp/cf-p7/v1.cfdir /tmp/cf-p7/src

echo 'hello-diff-v2' > /tmp/cf-p7/src/a.txt
./target/debug/chunkforge archive \
  --store /tmp/cf-p7/store -o /tmp/cf-p7/v2.cfdir \
  --seed /tmp/cf-p7/v1.cfdir /tmp/cf-p7/src

./target/debug/chunkforge diff /tmp/cf-p7/v1.cfdir /tmp/cf-p7/v2.cfdir
# changed≥1 → exit 1

./target/debug/chunkforge diff --tree /tmp/cf-p7/src /tmp/cf-p7/v2.cfdir
# identical → exit 0

./target/debug/chunkforge store scrub --store /tmp/cf-p7/store
# scrub: ok=N corrupt=0 unreadable=0
```

## Phase 8: HTTP retries + diff JSON + minimal SigV4 (**0.8.0**)

HTTP data-plane resilience and scriptable ops on top of Phase 7. Default
`--http-retries 0`, `diff --format text`, and SigV4 **off** match **0.7.0**.

- **`--http-retries N`** / **`--http-retry-backoff-ms`**: bounded whole-chunk
  retries for transient HTTP failures (408/429/5xx/timeout); local `--store` /
  `file://` ignore the flags — see [docs/http-retry.md](docs/http-retry.md)
- **Error-class summaries**: push/pull report `failed_transient=` /
  `failed_permanent=` / `retries=`; 401 vs 503 distinguishable; Corrupt never
  retried
- **`diff --format text|json`**: default text ≡ 0.7.0; JSON stable fields —
  see [docs/diff.md](docs/diff.md)
- **`--aws-sigv4`** (P1, default off): minimal in-process AWS4-HMAC-SHA256 from
  env credentials; **no** `aws-sdk-*` — see [docs/sigv4.md](docs/sigv4.md)
- Quickstart smoke: [`scripts/demo_http_retry.sh`](scripts/demo_http_retry.sh)
  (local `put_stub --fail-transient`: retries=0 fails on 503; retries=3
  succeeds; summary contains `retries=`)

**Still not this Phase:** full AWS SDK / multipart / packfile / write mount /
bidirectional sync / video analysis / remote scrub / byte-range resume. Remote
listing-ref integrity → `verify --source` (not a remote scrub command).

```bash
# HTTP retry smoke (or: bash scripts/demo_http_retry.sh)
cargo build -p chunkforge-cli
mkdir -p /tmp/cf-p8/src /tmp/cf-p8/mirror
echo 'hello-retry-v1' > /tmp/cf-p8/src/a.txt
cp fixtures/hello.txt /tmp/cf-p8/src/b.txt
./target/debug/chunkforge archive \
  --store /tmp/cf-p8/store -o /tmp/cf-p8/v1.cfdir /tmp/cf-p8/src

# Terminal 1 (inject two 503s then succeed):
#   python3 scripts/put_stub.py --root /tmp/cf-p8/mirror --port 8768 --fail-transient 2
./target/debug/chunkforge push --store /tmp/cf-p8/store \
  --dest http://127.0.0.1:8768 --http-retries 3 --http-retry-backoff-ms 0 \
  /tmp/cf-p8/v1.cfdir
# → uploaded≥1 failed=0 … retries=3

./target/debug/chunkforge diff --format json \
  /tmp/cf-p8/v1.cfdir /tmp/cf-p8/v1.cfdir
# → identical JSON object; exit 0
```

Details: [docs/http-retry.md](docs/http-retry.md),
[docs/remote-layout.md](docs/remote-layout.md),
[docs/sigv4.md](docs/sigv4.md), [docs/diff.md](docs/diff.md).


## Phase 9: extract skip + dry-run + perf baseline (**0.9.0**)

Incremental materialize and a reproducible loose-HTTP perf baseline on top of
Phase 8. Default extract (no new flags), retries=0, and SigV4 off stay
**0.8.0**-compatible.

- **`extract --skip-unchanged`**: opt-in; skip when dest size + content BLAKE3
  match listing `blob_blake3` (no chunk fetch/write; match beats `--force`)
- **`extract --skip-trust-mtime`** (Phase 11): requires `--skip-unchanged`;
  size+mtime hit skips content BLAKE3 (default off ≡ 1.0.0; see
  [docs/extract.md](docs/extract.md))
- **`extract --format text|json`** (Phase 11 M2): default **text** ≡ 1.0.0;
  JSON one object on stdout (`ok` / `skipped`/`wrote`/`dirs` or dry-run
  `would_*`)
- **`push` / `pull --format text|json`** (Phase 11 M3): default **text** ≡
  1.0.0; JSON one object on stdout (`ok` / `skipped` / `uploaded|fetched` /
  failure-class fields / `retries` / `unique_chunks` / `listings` /
  `dry_run`) — see [docs/push.md](docs/push.md) / [docs/pull.md](docs/pull.md)
- **`extract --dry-run`**: plan only — no target writes; `would_skip` /
  `would_write` / `would_dirs` / `would_fail`
- Docs: [docs/extract.md](docs/extract.md) (flag overlap; **no prune** of extra
  files under `-o`)
- Quickstart smoke: [`scripts/demo_extract_skip.sh`](scripts/demo_extract_skip.sh)
- **P1 O1** loose HTTP perf baseline: [docs/perf.md](docs/perf.md) +
  [`scripts/bench_loose_http.sh`](scripts/bench_loose_http.sh) (**pack not**
  implemented; defaults stay jobs=1 / retries=0)
- **P1 O3** `--aws-sigv4` falls back to `~/.aws/credentials` when env keys are
  missing (still no IMDS/SSO/`aws-sdk-*`) — [docs/sigv4.md](docs/sigv4.md)
- **P1 O2** FUSE sequential prefetch: delivered in **Phase 10** (see below)

**Still not this Phase:** full AWS SDK / multipart / packfile / write mount /
bidirectional sync / **extract prune (`--delete`)** / video analysis / remote
scrub / byte-range resume / push listing upload. `extract` ≠ sync.

```bash
# Incremental extract smoke (or: bash scripts/demo_extract_skip.sh)
cargo build -p chunkforge-cli
bash scripts/demo_extract_skip.sh
# first extract → --skip-unchanged (skipped=all, zero HTTP GET) →
# change one file → skipped=N-1 wrote=1 → dry-run glance

# Optional loose HTTP wall-clock baseline:
bash scripts/bench_loose_http.sh
# optional: bash scripts/bench_loose_http.sh --also-jobs-4

./target/debug/chunkforge --version   # → chunkforge 0.9.0
```

Details: [docs/extract.md](docs/extract.md), [docs/perf.md](docs/perf.md),
[docs/sigv4.md](docs/sigv4.md).

## Phase 10 / 1.0.0: FUSE prefetch + stability freeze

Phase 10 delivers the missing RO mount UX from Phase 9 **O2** — **sequential
chunk prefetch** on read-only FUSE (default **on**; `--no-prefetch` ≡ 0.9.0
on-demand `get`) — and freezes the **1.0 contract** in
[docs/stability.md](docs/stability.md). Details:
[docs/mount.md](docs/mount.md). Workspace / CLI version is **1.0.0**.

**Non-goals (one line):** no full AWS SDK / multipart / packfile / write mount /
bidirectional sync / extract prune / remote scrub / byte-range resume / push
listing upload / changing default jobs·retries / tokio default runtime / video
analysis — see **Non-goals (Phase 10 / 1.0)** above and `docs/stability.md`.

| Command | Role (no remote scrub, no sync) |
|---|---|
| `verify` | Listing + referenced chunk integrity (`--format text\|json`) |
| `doctor` | Presence (optional `--deep`; `--format text\|json`) |
| `gc` | Local unreferenced loose chunks |
| `store scrub` | Local loose BLAKE3 rehash |
| `diff` | Listing↔listing; **not** sync |
| `extract --skip-unchanged` / `--skip-trust-mtime` / `--dry-run` | Incremental / plan-only; **no** prune |
| `mount` (+ prefetch / `--no-prefetch`) | RO FUSE; sequential prefetch only |

```bash
# Prefetch on (default) vs off (≡ 0.9.0 on-demand get)
# chunkforge mount --store ./store release.cfdir ./mnt
# chunkforge mount --store ./store --no-prefetch release.cfdir ./mnt

# Prefetch get-count demo (unit tests; optional real mount if fuse available)
bash scripts/demo_mount_prefetch.sh

# 1.0 compat gate (no aws-sdk, key flags, demo subset; ~10–15 min; no perf SLA)
bash scripts/check_compat_1_0.sh

./target/debug/chunkforge --version   # → chunkforge 1.0.0
```

Scripts: [`scripts/check_compat_1_0.sh`](scripts/check_compat_1_0.sh),
[`scripts/demo_mount_prefetch.sh`](scripts/demo_mount_prefetch.sh).
Details: [docs/stability.md](docs/stability.md), [docs/mount.md](docs/mount.md).


## Phase 11 / 1.1.0: skip-trust-mtime + ops JSON

Phase 11 closes the remaining 1.0 experience debt: **`extract --skip-trust-mtime`**
(symmetric to `archive --seed-trust-mtime`) and expands ops **`--format json`**
from verify/doctor to **`extract` / `push` / `pull`**, plus P1
**`mount --prefetch-chunks N`**. All opt-in; **defaults ≡ 1.0.0** (text
summaries, content-path skip, no silent mtime trust, prefetch depth 1).
Workspace / CLI version is **1.1.0**.

**Delivered:**

- **`extract --skip-trust-mtime`**: requires `--skip-unchanged`; size +
  `mtime_secs` hit skips content BLAKE3 (default **off** ≡ 1.0.0). Docs warn
  about clock drift / forged / `cp -p` mtimes — see
  [docs/extract.md](docs/extract.md).
- **`extract --format text|json`**: default **text** ≡ 1.0.0 stderr summary;
  JSON one object on stdout. Write path: `ok` / `dry_run` / `skipped` /
  `wrote` / `dirs`. Dry-run: `ok` / `dry_run` / `would_skip` /
  `would_write` / `would_dirs` / `would_fail`.
- **`push` / `pull --format text|json`**: default **text** ≡ 1.0.0; JSON one
  object on stdout — `ok` / `skipped` / `uploaded|fetched` / `failed` /
  `failed_transient` / `failed_permanent` / `retries` / `unique_chunks` /
  `listings` / `dry_run` (field rename is **breaking**). See
  [docs/push.md](docs/push.md) / [docs/pull.md](docs/pull.md).
- Smoke: [`scripts/demo_ops_json.sh`](scripts/demo_ops_json.sh) (trust-mtime +
  extract/push/pull JSON via local `put_stub`; no internet).
- **`mount --prefetch-chunks N`** (P1 O1): default **1** ≡ 1.0.0; hard cap
  **≤2**; `--no-prefetch` still wins. See [docs/mount.md](docs/mount.md).

**Not delivered in 1.1.0 / non-goals (carry forward):** full AWS SDK /
multipart / packfile / write mount / bidirectional sync / extract prune /
remote scrub / byte-range resume / push listing upload / changing default
jobs·retries / video analysis. Pack stays measured-only in
[docs/perf.md](docs/perf.md). **`gc --jobs N`** (default 1 ≡ serial) lands in
Unreleased / Phase 12 M1 — see [docs/doctor-gc.md](docs/doctor-gc.md).

```bash
# Phase 11 ops smoke (~minutes; local put_stub only)
cargo build -p chunkforge-cli
bash scripts/demo_ops_json.sh
# A: archive → extract → --skip-unchanged --skip-trust-mtime --force (skipped=all)
# B: extract/push/pull --format json → python3 json.load

./target/debug/chunkforge --version   # → chunkforge 1.1.0
```

Details: [docs/extract.md](docs/extract.md), [docs/push.md](docs/push.md),
[docs/pull.md](docs/pull.md), [docs/mount.md](docs/mount.md).

## Incremental dedup demo

Generate offline ≥64MiB fixtures (gitignored), then remake + mid-file mutate:

```bash
make gen-large          # → fixtures/gen/large-64m.bin (+ -mut)
make demo-dedup         # or: ./scripts/demo_dedup.sh
make demo-dedup-small   # 4MiB smoke (faster)
```

`chunkforge make` stderr reports `new=` / `reused=` from store put outcomes.
See [docs/demo-dedup.md](docs/demo-dedup.md) and [`scripts/demo_dedup.sh`](scripts/demo_dedup.sh).

## Index / archive format note

The `.cfidx` (single-blob) and `.cfdir` (directory-tree) formats are **not**
casync / desync bit-compatible (different CDC, BLAKE3 vs SHA512/256, custom
layout). See [docs/index-format.md](docs/index-format.md) and
[docs/dir-format.md](docs/dir-format.md).

## Develop / CI

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# optional compression:
cargo test -p chunkforge-store --features zstd
# optional large-file dedup (generates fixtures; ignored by default in CI):
# CHUNKFORGE_GEN_MIB=8 cargo test -p chunkforge-cli --test cli_integration large_file_dedup -- --ignored --nocapture
# optional real FUSE mount (needs fuse3 + /dev/fuse; ignored by default):
# cargo test -p chunkforge-fuse -- --ignored
```

GitHub Actions: [`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs fmt + clippy + test on push to `main`/`master` and on pull requests. Real FUSE mounts and the optional 64MiB `gen_large` smoke stay **`#[ignore]`** (not part of default CI).

## License

MIT — see [LICENSE](LICENSE). Changelog: [CHANGELOG.md](CHANGELOG.md).
