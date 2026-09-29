# ChunkForge

**ChunkForge**: FastCDC content-defined chunking + BLAKE3 content addressing to split large files/blobs into deduplicable chunks, write them into a local CAS store, and reassemble via a custom `.cfidx` index — with on-demand fetch (`ChunkSource`), object-store–friendly HTTP templates, per-chunk HTTP **PUT** (`ChunkSink` / `push`), **directory-tree archive** (`.cfdir` / `archive` / `extract`), and **read-only** FUSE mount of a single blob **or** a directory tree.

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

## Non-goals (current / Phase 7)

| Not this | Why |
|---|---|
| ❌ **In-process SigV4** | No GET or PUT HMAC; use public/CDN, fixed header templates, or externally presigned query in `--url-template` |
| ❌ **Full AWS/S3 SDK** | No `aws-sdk-*` / `aws-config` / ListObjects / credential chain — dependency surface stays **ureq** |
| ❌ **Complete S3 multipart upload API** | No InitiateMultipartUpload / UploadPart / Complete / Abort — chunks ≤256KiB; **single-object PUT** only |
| ❌ **Write mount / COW** | FUSE stays `RO` (single blob **and** directory tree); writes return `EROFS` / `EACCES` |
| ❌ **Bidirectional sync** | `archive` / `extract` / `push` / `pull` are explicit one-way — no watch directories, conflict resolution, or mutual sync |
| ❌ **`push` uploads listings** | Chunks only; `.cfdir` / `.cfidx` stay local (git / release artifact / optional manual URL) |
| ❌ **casync `.catar` / `.caibx` bit-compat** | Semantic alignment only; native `.cfdir` / `.cfidx` (not a binary drop-in) |
| ❌ **Packfile / multi-chunk single object** | Loose `.cnk` layout unchanged |
| ❌ **Remote GC / lifecycle** | `gc` only touches a **local** `--store` |
| ❌ **Remote scrub** | `store scrub` only rehashes a **local** `--store`; no remote bitrot scan |
| ❌ Not a restic/rustic-style **backup product** | No snapshot policy, encrypted-repo lifecycle, or prune |
| ❌ **P2P** / **GPU / LLM** / video analysis | Pure CPU data plane; no device discovery |
| ❌ macOS / Windows as acceptance platforms | Linux + fuse3 is first-class; other OS are experimental / unsupported |

Earlier phases delivered local CAS (Phase 1), remote read + RO single-blob mount (Phase 2), templates / doctor / gc (Phase 3), per-chunk PUT / `push` / `--jobs` (Phase 4), multi-file `.cfdir` + DirFs (Phase 5), and incremental `archive --seed` + `pull` (Phase 6). Phase 7 (**0.7.0**) adds listing **`diff`**, local CAS **`store scrub`**, **`--seed-trust-mtime`**, and **`extract --force`**.

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
