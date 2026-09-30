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
| **Phase 12** | **1.2.0** | `gc --jobs` + `gc`/`store scrub --format json` + ops JSON field matrix + `demo_ops_maint` + `check_compat_1_1` + opt-in `--progress` |
| **Phase 13** | **1.3.0** | Path scope: `archive`/`extract`/`pull --path`/`--exclude` + `archive --format json` + ops-json archive row + `demo_path_filter` + `check_compat_1_2`; path **≠** prune **≠** sync; **not** pack / write mount |
| **Phase 14** | **1.4.0** | Publish symmetry + local observability: `push --path`/`--exclude` + `store stats`/`du` + `--exclude-from` + ops-json expand + `demo_push_path_store_stats` + `check_compat_1_3`; defaults ≡ 1.3; **not** pack / write mount / aws-sdk |
| **Phase 15** | **1.5.0** | Ops JSON closeout + cache soft budget: `--cache-max-bytes` (refuse-fill ≠ LRU) + `make`/`cat --format json` + ops-json finalize + `demo_cache_budget_ops_json` + `check_compat_1_4` (+ P1 `store scrub --listing`); defaults ≡ 1.4; **not** pack / write mount / aws-sdk / LRU |
| **Phase 16** | **1.6.0** | Read-path Failover + ops sugar: `--fallback` (Missing-only; Cache wraps whole chain) + `--cache-max-bytes` human suffixes (`1M`…) + `store stats` `bytes_plaintext`/`--decode` + `demo_fallback_bytes_suffix` + **`check_compat_1_5`** (+ P1 `diff --path`/`--exclude`); defaults ≡ 1.5; **not** pack / write mount / aws-sdk / prune / LRU / remote scrub |
| **Phase 17** | **1.7.0** | CLI create-time **`--compression none\|zstd`** (default **none** ≡ 1.6) + `archive`/`extract`/`make --progress` (default off) + docs matrix + `demo_zstd_progress` + **`check_compat_1_6`** (+ P1 CacheSource hit/refuse counters); defaults ≡ 1.6; **not** pack / write mount / aws-sdk / prune / LRU / default zstd / wire compression |
| **Phase 18** | **1.8.0** | **`pull --verify`** (symmetric to push; default off) + **`--cache-stats`** / ops-json additive **`cache_*`** (≠ LRU) + **`cat`/`verify --progress`** (default off) + docs + `demo_pull_verify_cache_stats` + **`check_compat_1_7`** (+ P1 **`doctor --progress`**); defaults ≡ 1.7; **not** pack / write mount / aws-sdk / prune / LRU / default zstd / wire compression / remote scrub |
| **Phase 19** | **1.9.0** | **`store create`** + **`pull --compression`** (create-time; omit ≡ none ≡ 1.8) + **`diff --progress`** (default off) + docs + `demo_store_create_pull_compression` + **`check_compat_1_8`** + P1 honest **`make --jobs`** (post-chunk put only; default 1; **not** parallel FastCDC); defaults ≡ 1.8; **not** pack / write mount / aws-sdk / prune / LRU / default zstd / recompress / push `--fallback` / remote scrub |
| **Phase 20** | **1.10.0** | **`--path-from`** (`load_path_file`) on archive/extract/push/pull/diff/doctor/verify + **`doctor`/`verify` path scope** (default ≡ 1.9 full) + docs + `demo_path_from_doctor_verify` + **`check_compat_1_9`** + P1 **`push` local/`file://` dest** (Store as `ChunkSink`; single dest; create **none**); **`path-from` ≠ prune ≠ gc-path ≠ sync ≠ pack**; **push local ≠ fallback**; **not** pack / write mount / aws-sdk / prune / **`gc --path`** / LRU / default zstd / recompress / push `--fallback` / remote scrub |
| **Phase 21** | **1.11.0** | **`mount` path quartet** (`--path`/`--exclude`/`--exclude-from`/`--path-from` → `filter_dir_archive` → DirFs; default ≡ 1.10 full tree; `.cfidx`+path → non-zero) + docs + `demo_mount_path` + **`check_compat_1_10`** + P1 **`push --compression`** + P1 **`store list`** (+ thin docs); **`mount path` ≠ write mount ≠ prune ≠ gc-path ≠ sync ≠ pack**; **not** pack / write mount / aws-sdk / prune / **`gc --path`** / LRU / default zstd / recompress / push `--fallback` / remote scrub / mount `--progress` |
| **Phase 22** | **1.12.0** | **`.cfdir` Symlink opt-in** (`archive --symlinks skip\|record`; default **skip** ≡ 1.11 + write v1; record → `KIND_SYMLINK` / `format_version=2`; extract materialize; DirFs `readlink`; still RO) + docs + `demo_symlink` + **`check_compat_1_11`** + P1 **`make --dry-run`**; **`record` ≠ write mount ≠ follow ≠ pack ≠ offline bundle ≠ prune ≠ `gc --path` ≠ default record**; **`make --dry-run` ≠ seed ≠ pack ≠ recompress**; **not** pack / write mount / aws-sdk / prune / **`gc --path`** / LRU / default zstd / default record / follow / fifo/xattr / offline bundle / mount `--progress` |
| **Phase 23** | **1.13.0** | **`diff --tree --symlinks skip\|record`** (default **skip** ≡ 1.12; **record** → ephemeral Symlink; clap requires `--tree`) + docs + [`demo_diff_tree_symlink`](scripts/demo_diff_tree_symlink.sh) + **`check_compat_1_12`** + P1 extract dry-run **`would_symlinks`** (`would_write` still includes symlink ≡ 1.12); **`diff --tree --symlinks record` / `would_symlinks` ≠ write mount ≠ follow ≠ pack ≠ sync ≠ prune ≠ `gc --path` ≠ default record**; **not** pack / write mount / aws-sdk / prune / **`gc --path`** / LRU / default zstd / default record / follow / fifo/xattr / offline bundle / mount `--progress` |
| **Phase 24** | **1.14.0** | **`chunkforge filter`** persist path-scoped subset `.cfdir` + docs + [`demo_filter_listing`](scripts/demo_filter_listing.sh) + **`check_compat_1_13`** + P1 **`make --seed`** / mount help File+Symlink honesty; **`filter` ≠ prune ≠ `gc --path` ≠ sync ≠ write mount ≠ pack ≠ `archive --path`**; **not** pack / write mount / aws-sdk / prune / **`gc --path`** / LRU / default zstd / default record / follow / fifo/xattr / offline bundle / mount `--progress` |
| **Phase 25** | **1.15.0** | **`chunkforge ls`** listing inventory (`.cfidx`/`.cfdir`; path 四件套; text/json; `--chunks`; File+Symlink+Dir; **no store**) + **`cat --path`** single File from `.cfdir` + docs + [`demo_ls_cat_path`](scripts/demo_ls_cat_path.sh) + **`check_compat_1_14`** + P1 **`archive --empty-dirs`** + `chunk-id`/`store has --format json`; **`ls` ≠ mount ≠ extract ≠ verify ≠ pack ≠ filter**; **`cat --path` ≠ extract ≠ prune ≠ sync**; **not** pack / write mount / aws-sdk / prune / **`gc --path`** / LRU / default zstd / default record / follow / fifo/xattr / offline bundle / mount `--progress` |
| **Phase 26** | **toward 1.16.0** (workspace still **1.15.0**) | **`filter_dir_archive` leaf-Dir** (PathFilter `allows` explicit Dir; empty filter ≡ identity; empty-dirs ↔ `ls`/`filter`/`mount`/diff path) + **`store get`** (`--store`, hex id, `-o`, optional `--verify`, text\|json `{ok,id,bytes}`) + docs + [`demo_empty_dir_path_store_get`](scripts/demo_empty_dir_path_store_get.sh); **`check_compat_1_15` note-only until M5**; **`filter_dir_archive` leaf-Dir ≠ prune ≠ `gc --path`**; **`store get` ≠ scrub ≠ cat ≠ extract ≠ recompress**; **not** pack / write mount |

## Non-goals (Phase 10 / 1.0)

| Not this | Why |
|---|---|
| ❌ **Full AWS/S3 SDK** (`aws-sdk-*` / `aws-config` / ListObjects / IMDS / SSO) | HTTP stays **ureq**; optional minimal SigV4 via `--aws-sigv4` (env + shared credentials file; see `docs/sigv4.md`) |
| ❌ **Complete S3 multipart upload API** | Chunks ≤256KiB; **single-object PUT** only |
| ❌ **Packfile / multi-chunk single object** | Loose `.cnk` layout unchanged; [docs/perf.md](docs/perf.md) measures only — pack **not** implemented |
| ❌ **Write mount / COW / writable FUSE** | FUSE stays `RO` (blob **and** tree); prefetch is **not** write-back; writes return `EROFS` / `EACCES` |
| ❌ **Cache LRU / auto trim / `store trim`** | Soft budget (`--cache-max-bytes`) is **refuse-fill only**; never evicts; ≠ GC ≠ sync |
| ❌ **Bidirectional sync / watch dirs / conflict resolution** | `diff` is **not** sync; `extract --skip-unchanged` is **not** sync |
| ❌ **Extract prune / `--delete`** | `extract` never removes extra files under `-o` |
| ❌ **`gc --path` / path-scoped GC** | Shrinking the keep-set mis-deletes; `gc` stays full listing refs. **`path-from` ≠ gc-path** |
| ❌ **Remote scrub / remote GC / bucket lifecycle** | Referenced remote integrity → `verify --source`; presence → `doctor`; `gc` / `store scrub` stay **local** `--store` only |
| ❌ **Byte-range / partial-chunk HTTP resume** | Retries **whole chunks** only (chunks ≤256KiB) |
| ❌ **`push` uploads listings** | Chunks only; `.cfdir` / `.cfidx` stay out-of-band (git / release artifact) |
| ❌ **Change default `--jobs` / `--http-retries`** | Stay **jobs=1**, **retries=0** (≡ 0.9.0) |
| ❌ **Tokio as default runtime** | Keep `std::thread` + ureq; mount prefetch may sync-get the next chunk on the call thread |
| ❌ **Rewrite / abandon `.cfidx` v1 or `.cfdir` v1** | Prefetch / stability / optional JSON do **not** bump magic |
| ❌ **Full POSIX fidelity / default symlink recording / fifo·xattr·ACL** | Default **`--symlinks skip`** ≡ 1.11 skip+warn. Phase22 **opt-in** `--symlinks record` is the product path (not default; not full POSIX) |
| ❌ **casync `.catar` / `.caibx` bit-compat** | Semantic alignment only; native `.cfdir` / `.cfidx` |
| ❌ **P2P** / **GPU / LLM** / video analysis | Pure CPU data plane; no device discovery |
| ❌ Not a restic/rustic-style **backup product** | No snapshot policy, encrypted-repo lifecycle, or prune |
| ❌ macOS / Windows as acceptance platforms | Linux + fuse3 is first-class; other OS are experimental / unsupported |

Earlier phases delivered local CAS (Phase 1), remote read + RO single-blob mount (Phase 2), templates / doctor / gc (Phase 3), per-chunk PUT / `push` / `--jobs` (Phase 4), multi-file `.cfdir` + DirFs (Phase 5), incremental `archive --seed` + `pull` (Phase 6), listing **`diff`** / **`store scrub`** (Phase 7), HTTP **`--http-retries`** / **`diff --format json`** / minimal **`--aws-sigv4`** (Phase 8), **`extract --skip-unchanged`** / **`--dry-run`** + loose perf baseline + SigV4 shared-creds (Phase 9), FUSE sequential prefetch + 1.0 stability freeze (Phase 10 / **1.0.0**), and `extract --skip-trust-mtime` + ops JSON + `--prefetch-chunks` (Phase 11 / **1.1.0**). **Phase 12 closed at 1.2.0** (`gc --jobs` + gc/scrub JSON + ops-json matrix + `check_compat_1_1` + opt-in `--progress`). **Phase 13 closed at 1.3.0** (`PathFilter` + path scope + `check_compat_1_2`). **Phase 14 is closed at 1.4.0**: `store stats`/`du` + `push --path`/`--exclude` + `--exclude-from` + `check_compat_1_3` — see [docs/stability.md](docs/stability.md) / [docs/ops-json.md](docs/ops-json.md). **Phase 15 closed at 1.5.0**: `--cache-max-bytes` (refuse-fill) + `make`/`cat --format json` + ops-json finalize + `demo_cache_budget_ops_json` + `check_compat_1_4` (+ P1 `store scrub --listing`) — see [docs/stability.md](docs/stability.md) / [docs/ops-json.md](docs/ops-json.md). **Phase 16 closed at 1.6.0**: `--fallback` + cache-max suffixes + `bytes_plaintext`/`--decode` + `demo_fallback_bytes_suffix` + `check_compat_1_5` (+ P1 `diff --path`/`--exclude`) — see [docs/stability.md](docs/stability.md) / [docs/ops-json.md](docs/ops-json.md); defaults ≡ 1.5; **not** pack / write mount / aws-sdk / prune / LRU / remote scrub. **Phase 17 closed at 1.7.0**: create-time `--compression none|zstd` + `archive`/`extract`/`make --progress` + `demo_zstd_progress` + `check_compat_1_6` (+ P1 CacheSource hit/refuse counters) — see [docs/stability.md](docs/stability.md) / [docs/ops-json.md](docs/ops-json.md); defaults ≡ 1.6; **not** pack / write mount / aws-sdk / prune / LRU / default zstd / wire compression. **Phase 18 closed at 1.8.0**: `pull --verify` + Cache observation CLI/JSON + `cat`/`verify --progress` + `demo_pull_verify_cache_stats` + `check_compat_1_7` (+ P1 `doctor --progress`) — see [docs/stability.md](docs/stability.md) / [docs/ops-json.md](docs/ops-json.md) / [docs/pull.md](docs/pull.md); defaults ≡ 1.7; **not** pack / write mount / aws-sdk / prune / LRU / default zstd / wire compression / remote scrub.
**Phase 19 closed at 1.9.0**: `store create` + `pull --compression` + `diff --progress` + `demo_store_create_pull_compression` + `check_compat_1_8` + P1 honest `make --jobs` (post-chunk put; FastCDC stays serial) — see [docs/stability.md](docs/stability.md) / [docs/ops-json.md](docs/ops-json.md) / [docs/store.md](docs/store.md) / [docs/pull.md](docs/pull.md); defaults ≡ 1.8; **`store create` ≠ recompress ≠ default zstd ≠ pack**; **not** pack / write mount / aws-sdk / prune / LRU / default zstd / recompress / push `--fallback` / remote scrub.
**Phase 20 closed at 1.10.0**: `--path-from` + `doctor`/`verify` path scope + `demo_path_from_doctor_verify` + `check_compat_1_9` + P1 **`push` local/`file://` `--dest`** (Store as `ChunkSink`; single dest; create compression **none**) — see [docs/stability.md](docs/stability.md) / [docs/ops-json.md](docs/ops-json.md) / [docs/doctor-gc.md](docs/doctor-gc.md) / [docs/push.md](docs/push.md); defaults ≡ 1.9; **`path-from` ≠ prune ≠ gc-path ≠ sync ≠ pack**; **push local ≠ `--fallback` / multi-dest**; **not** pack / write mount / aws-sdk / prune / **`gc --path`** / LRU / default zstd / recompress / push `--fallback` / remote scrub.
**Phase 21 closed at 1.11.0**: `mount` path quartet (`filter_dir_archive` → DirFs; default ≡ 1.10 full tree) + docs + `demo_mount_path` + `check_compat_1_10` + P1 `push --compression` + P1 `store list` — see [docs/mount.md](docs/mount.md) / [docs/store.md](docs/store.md) / [docs/stability.md](docs/stability.md); **`mount path` ≠ write mount ≠ prune ≠ gc-path ≠ sync ≠ pack**; **not** pack / write mount / **`gc --path`** / mount `--progress` / default zstd.
**Phase 22 closed at 1.12.0**: `.cfdir` Symlink opt-in (`--symlinks record`; default skip ≡ 1.11) + extract/DirFs wiring + `demo_symlink` + `check_compat_1_11` + P1 `make --dry-run` — see [docs/dir-format.md](docs/dir-format.md) / [docs/archive.md](docs/archive.md) / [docs/stability.md](docs/stability.md); **`record` ≠ write mount ≠ follow ≠ pack ≠ prune ≠ `gc --path` ≠ default record**; **`make --dry-run` ≠ seed ≠ pack ≠ recompress**; **not** pack / write mount / **`gc --path`** / mount `--progress` / default zstd / default record.
**Phase 23 closed at 1.13.0**: `diff --tree --symlinks skip|record` (default skip ≡ 1.12) + docs + `demo_diff_tree_symlink` + `check_compat_1_12` + P1 extract dry-run `would_symlinks` — see [docs/diff.md](docs/diff.md) / [docs/stability.md](docs/stability.md); **`diff --tree --symlinks record` / `would_symlinks` ≠ write mount ≠ follow ≠ pack ≠ sync ≠ prune ≠ `gc --path` ≠ default record**; **not** pack / write mount / **`gc --path`** / default record / follow / mount `--progress` / default zstd.
**Phase 24 closed at 1.14.0**: first-class **`chunkforge filter`** + docs + `demo_filter_listing` + **`check_compat_1_13`** + P1 **`make --seed <prior.cfidx>`** / mount help File+Symlink honesty — see [docs/filter.md](docs/filter.md) / [docs/ops-json.md](docs/ops-json.md) / [docs/stability.md](docs/stability.md); **`filter` ≠ prune ≠ `gc --path` ≠ sync ≠ write mount ≠ pack ≠ `archive --path`**; **not** pack / write mount / **`gc --path`** / default record / follow / mount `--progress` / default zstd.
**Phase 25 closed at 1.15.0**: first-class **`chunkforge ls`** (listing inventory; path 四件套; text/json; `--chunks`; no store) + **`cat --path`** (`.cfdir` single File) + docs + `demo_ls_cat_path` + **`check_compat_1_14`** + P1 **`archive --empty-dirs`** / thin `chunk-id`/`store has --format json` — see [docs/ls.md](docs/ls.md) / [docs/stability.md](docs/stability.md); **`ls` ≠ mount ≠ extract ≠ verify ≠ pack ≠ filter**; **`cat --path` ≠ extract ≠ prune ≠ sync**; **not** pack / write mount / **`gc --path`** / default record / follow / mount `--progress` / default zstd.
**Phase 26 (toward 1.16.0; version still 1.15.0)**: shared **`filter_dir_archive` leaf-Dir** keep (empty-dirs path scope on `ls` / `filter` / `mount` / diff) + **`store get`** + docs + `demo_empty_dir_path_store_get` — see [docs/filter.md](docs/filter.md) / [docs/store.md](docs/store.md) / [docs/stability.md](docs/stability.md); **`filter_dir_archive` leaf-Dir ≠ prune ≠ `gc --path`**; **`store get` ≠ scrub ≠ cat ≠ extract ≠ recompress**; **not** pack / write mount. **`check_compat_1_15.sh` is M5** (M4 demo is note-only). Defaults ≡ **1.15**.

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
Optional **`--cache-max-bytes N`** (Phase 15, with `--cache`) soft-caps fill:
over budget **skips put**, still serves primary — **≠ LRU ≠ trim ≠ GC ≠ sync**.

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
| `diff` | Listing↔listing (+ optional `--path`/`--exclude`); **not** sync |
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
[docs/perf.md](docs/perf.md). **`gc --jobs N`**, **`gc`/`store scrub --format json`**, and the ops JSON
matrix landed in **1.2.0** — see [Phase 12](#phase-12--12-local-maint-ops-json)
and [docs/doctor-gc.md](docs/doctor-gc.md).

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


## Phase 12 / 1.2: local maint ops JSON

Phase 12 closes local maintenance symmetry with **1.1.0**: **`gc --jobs N`**
(default **1** ≡ serial, symmetric to `store scrub --jobs`), expands ops
**`--format json`** to **`gc`** and **`store scrub`**, publishes the
**[Ops JSON field matrix](docs/ops-json.md)**, adds **`check_compat_1_1.sh`**,
and ships opt-in **`--progress`**. All additive; **defaults ≡ 1.1.0** (text
summaries, jobs=1, progress off). Workspace / CLI version is **1.2.0**.

**Delivered:**

- **`gc --jobs N`**: default **1**; `--apply` parallel per-id delete; dry-run
  path listing stays ordered.
- **`gc --format text|json`**: default **text**. JSON minimum set (`ok` /
  `dry_run` / `applied` / `listings` / `referenced` / `unreferenced` /
  `deleted`) — see [docs/doctor-gc.md](docs/doctor-gc.md) / Phase12 §3.2.
- **`store scrub --format text|json`**: default **text**. JSON minimum set
  (`ok` / `checked` / `ok_count` / `corrupt` / `unreadable` / `corrupt_ids` /
  `unreadable_ids`).
- **Ops JSON field matrix**: [docs/ops-json.md](docs/ops-json.md) (linked from
  [docs/stability.md](docs/stability.md)) — `diff` / `verify` / `doctor` /
  `extract` / `push` / `pull` / `gc` / `store scrub`; **field rename → breaking**.
- Smoke: [`scripts/demo_ops_maint.sh`](scripts/demo_ops_maint.sh) (gc dry-run
  json + scrub json + `--jobs 4`; local only).
- **Compat gate**: [`scripts/check_compat_1_1.sh`](scripts/check_compat_1_1.sh)
  — runs `check_compat_1_0.sh`, then asserts 1.1/1.2 help flags
  (`--skip-trust-mtime`, extract/push/pull/`gc`/`store scrub --format`,
  `mount --prefetch-chunks`, `gc --jobs`); thin no-pack CLI assert. No
  absolute perf SLA.
- **Opt-in `--progress`** (M6 / O1): default **off** ≡ 1.1.0. Long paths
  **`push` / `pull` / `store scrub`** (+ **`gc --apply`**). Stderr
  `progress: op=… done=N/TOTAL`; orthogonal to `--format json`. No
  indicatif / tracing / otel.

**Responsibility split (no remote scrub):**

| Command | Role |
|---|---|
| `verify` | Listing structure + referenced chunk integrity (incl. HTTP) |
| `doctor` | Presence of referenced chunks (`has` / `--deep`) |
| `gc` | Local unreferenced loose chunks (dry-run / `--apply`) |
| `store scrub` | Local loose-chunk full BLAKE3 rehash (no listing) |

**Not delivered / non-goals (carry forward from 1.2):** packfile; write mount;
remote scrub; aws-sdk; extract prune; bidirectional sync; byte-range resume;
push listing upload; changing default jobs·retries; video analysis.
`archive --format json` + path filters moved to **Phase 13** (see below).
Pack stays measured-only in [docs/perf.md](docs/perf.md).

```bash
# Phase 12 maint smoke + compat gate (~minutes; no internet)
cargo build -p chunkforge-cli
bash scripts/demo_ops_maint.sh
# A: archive v1 → change → archive --seed v2 (orphan chunks vs v2-only)
# B: gc --format json (unreferenced>0) + store scrub --format json + --jobs 4
bash scripts/check_compat_1_1.sh   # includes check_compat_1_0 + 1.1/1.2 flags

./target/debug/chunkforge --version   # → chunkforge 1.2.0
```

Details: [docs/doctor-gc.md](docs/doctor-gc.md), [docs/ops-json.md](docs/ops-json.md),
[docs/stability.md](docs/stability.md). Gate:
[`scripts/check_compat_1_1.sh`](scripts/check_compat_1_1.sh).


## Phase 13 / 1.3: path-scope filter

Phase 13 closes path-scope selectivity on tree ops at **1.3.0**: opt-in
**`--path` / `--exclude`** on **`archive` / `extract` / `pull`**, plus
**`archive --format json`**, ops-json archive rows, `demo_path_filter`, and
**`check_compat_1_2`**. All additive; **defaults ≡ 1.2.0** (no path flags ⇒
full tree / full reference set; `archive --format` default **text**).
Workspace / CLI version is **1.3.0**.

**Delivered:**

- **`PathFilter`** (`chunkforge-index`): `--path` prefix include (OR) +
  `--exclude` exact / trailing-`/` / edge `*` — no `ignore`/`globset`.
- **`archive --path` / `--exclude` + `--format json`**: filtered listing;
  JSON minimum set in [docs/ops-json.md](docs/ops-json.md) (`ok` / `dry_run` /
  `files` / `dirs` / `chunks` / `written`|`would_write` / `reused`|`would_reuse`
  / `seed_reused_files` / `rechunked_files` / `skipped_symlinks` /
  `skipped_special` / `excluded`). Field rename → **breaking**.
- **`extract --path` / `--exclude`**: materialize matching Files only;
  **never** deletes filtered-out listing paths or extra dest files (**not**
  prune / **not** sync / **no** `--delete`).
- **`pull --path` / `--exclude`**: fetch chunk ids from matching Files only;
  JSON field names unchanged; `unique_chunks` = filtered set.
- Smoke: [`scripts/demo_path_filter.sh`](scripts/demo_path_filter.sh)
  (exclude → archive json → extract `--path` → pull `--path`; local
  `put_stub` only).
- **Compat gate**: [`scripts/check_compat_1_2.sh`](scripts/check_compat_1_2.sh)
  — runs `check_compat_1_1.sh`, then asserts 1.3 help flags
  (`archive --format` + `--path`/`--exclude`, `extract --path`, `pull --path`);
  thin no-`--delete` / no-pack; invokes `demo_path_filter.sh`. No absolute
  perf SLA.

**Responsibility split (path filter ≠ prune ≠ sync):**

| Command | Role |
|---|---|
| `archive --path` / `--exclude` | Write a **smaller** `.cfdir` listing (subset of tree) |
| `extract --path` / `--exclude` | Materialize **fewer** dest paths; **never** deletes extras |
| `pull --path` / `--exclude` | Fetch a **subset** of referenced chunk ids into local CAS |
| `push --path` / `--exclude` / `--exclude-from` | Upload a **subset** of referenced chunk ids; **not** listing upload |
| `diff` | Listing↔listing (+ optional `--path`/`--exclude`); **not** sync |
| `gc` / `store scrub` | Local CAS only; **not** remote scrub |

**Not delivered in 1.3 / non-goals (carry forward):** packfile;
write mount / COW; bidirectional sync; extract prune / `--delete`; remote
scrub; aws-sdk; byte-range resume; push listing upload; changing default
jobs·retries; video analysis. Pack remains measured-only in
[docs/perf.md](docs/perf.md) — **1.3.0 still does not implement pack**.

```bash
# Phase 13 path-filter smoke + compat gate (~minutes; local put_stub only)
cargo build -p chunkforge-cli
bash scripts/demo_path_filter.sh
# A: source with packages/foo + junk/.git → archive --exclude --format json
# B: extract --path packages/foo (extra dest file kept; no prune)
# C: push → pull --path → unique_chunks is subset
bash scripts/check_compat_1_2.sh   # includes 1_1 → 1_0 + 1.3 flags + demo

./target/debug/chunkforge --version   # → chunkforge 1.3.0
```

Details: [docs/archive.md](docs/archive.md), [docs/extract.md](docs/extract.md),
[docs/pull.md](docs/pull.md), [docs/ops-json.md](docs/ops-json.md),
[docs/stability.md](docs/stability.md),
[docs/remote-layout.md](docs/remote-layout.md). Gate:
[`scripts/check_compat_1_2.sh`](scripts/check_compat_1_2.sh).

## Phase 14 / 1.4: push path + store stats + exclude-from

Phase 14 closes **publish symmetry** and **local CAS observability** at
**1.4.0**: opt-in **`push --path` / `--exclude`**, **`store stats`/`du`**,
**`--exclude-from`** on archive/extract/pull/push, ops-json expand,
`demo_push_path_store_stats`, and **`check_compat_1_3`**. All additive;
**defaults ≡ 1.3.0** (no path/exclude/exclude-from ⇒ full reference set;
jobs=1, retries=0, text, progress off). Workspace / CLI version is **1.4.0**.

**Delivered:**

- **`store stats`** (alias **`du`**) + `--format text|json`: local chunk count
  + on-disk `.cnk` bytes via `Store::stats` (no plaintext decode; not GC /
  scrub / trim / LRU). JSON: `ok` / `chunks` / `bytes_on_disk` / `compression`.
  See [docs/ops-json.md](docs/ops-json.md) / [docs/doctor-gc.md](docs/doctor-gc.md).
- **`push --path` / `--exclude`**: upload chunk ids from matching `.cfdir`
  **File** entries only; listing **not** uploaded; `.cfidx`+path → clear error;
  JSON field names unchanged (`unique_chunks` = filtered). See
  [docs/push.md](docs/push.md).
- **`--exclude-from <file>`** on archive / extract / pull / push: one
  `ExcludePat` per line; ∪ with `--exclude`; no `ignore`/`globset`.
- Smoke: [`scripts/demo_push_path_store_stats.sh`](scripts/demo_push_path_store_stats.sh)
  (full-tree archive → store stats json → push `--path` PUT count < full →
  `--exclude-from`; local `put_stub` only).
- **Compat gate**: [`scripts/check_compat_1_3.sh`](scripts/check_compat_1_3.sh)
  — runs `check_compat_1_2.sh`, then asserts 1.4 help flags (`push --path`/
  `--exclude`, `store stats`/`du --format`, archive/extract/pull/push
  `--exclude-from`); thin no-pack / no-`--delete`; asserts
  `demo_push_path_store_stats.sh` present. No absolute perf SLA.

**`path` ≠ listing upload ≠ sync / prune:**

| Flag / command | Role |
|---|---|
| `push --path` / `--exclude` / `--exclude-from` | Upload a **subset** of referenced chunk ids; **not** listing upload; **not** remote delete |
| `store stats` / `du` | Observe local CAS size; **not** trim / LRU |
| `extract --path` | Write fewer dest paths; **never** deletes extras |

**Not delivered / non-goals (carry forward):** `make`/`cat --format json`
(P1); packfile; write mount / COW; bidirectional sync; extract prune /
`--delete`; remote scrub; full aws-sdk / multipart / IMDS / SSO / ListObjects;
byte-range resume; push listing upload; cache LRU; changing default
jobs·retries·SigV4·progress; absolute perf SLA in CI; video analysis. Pack
stance: [docs/perf.md](docs/perf.md) — **1.4.0 still does not implement pack**.

```bash
# Phase 14 push-path + store-stats smoke (~minutes; local put_stub only)
cargo build -p chunkforge-cli
bash scripts/demo_push_path_store_stats.sh
# A: archive full tree → store stats --format json
# B: push full vs push --path packages/foo (PUT count < full; no listing upload)
# C: push --exclude-from (subset)

bash scripts/check_compat_1_3.sh   # includes 1_2 → 1_1 → 1_0 + 1.4 flags
./target/debug/chunkforge --version   # → chunkforge 1.4.0
```

Details: [docs/push.md](docs/push.md), [docs/ops-json.md](docs/ops-json.md),
[docs/doctor-gc.md](docs/doctor-gc.md), [docs/stability.md](docs/stability.md),
[docs/perf.md](docs/perf.md). Gate:
[`scripts/check_compat_1_3.sh`](scripts/check_compat_1_3.sh).


## Phase 15 / 1.5.0: cache soft budget + make/cat ops JSON

Phase 15 closes **ops JSON closeout** and **cache soft budget** at
**1.5.0**: opt-in **`--cache-max-bytes`** (refuse-fill ≠ LRU),
**`make`/`cat --format text|json`**, ops-json finalize,
`demo_cache_budget_ops_json`, and **`check_compat_1_4`** (+ P1
`store scrub --listing`). All additive; **defaults ≡ 1.4.0** (no
`--cache-max-bytes` ⇒ unbounded cache fill; `make`/`cat` default **text**;
jobs=1, retries=0, SigV4 off, progress off, mount prefetch depth 1).
Workspace / CLI version is **1.5.0**.

**Delivered:**

- **`--cache-max-bytes N`** on `cat` / `verify` / `extract` / `mount` (requires
  `--cache`): soft fill budget in pure integer bytes. Over budget **refuse-fill**
  (still returns primary plaintext); **never** evicts / LRU / trim / GC.
  Omit ≡ **1.4** unbounded. See [docs/mount.md](docs/mount.md) /
  [docs/extract.md](docs/extract.md).
- **`make --format text|json`**: default text ≡ 1.4 stderr summary; json one
  object `{ok, bytes, chunks, new, reused}`.
- **`cat --format text|json`**: default text ≡ 1.4; json `{ok, bytes}` (still
  writes `-o`). Orthogonal to `--cache` / `--cache-max-bytes` / `--jobs`.
- **Ops-json finalize**: [docs/ops-json.md](docs/ops-json.md) make/cat rows
  final; Out of scope make/cat removed; cache-max = refuse-fill explicitly.
- Smoke: [`scripts/demo_cache_budget_ops_json.sh`](scripts/demo_cache_budget_ops_json.sh)
  (make/cat json parse + first miss fills / second miss no disk growth).
- **Compat gate**: [`scripts/check_compat_1_4.sh`](scripts/check_compat_1_4.sh)
  — runs `check_compat_1_3.sh`, asserts 1.5 help flags (`make`/`cat --format`,
  `--cache-max-bytes` on cat/verify/extract/mount); thin non-goals (no pack /
  prune / LRU); demo presence. No absolute perf SLA.
- **P1 `store scrub --listing <index>`**: local referenced-id rehash only
  (default ≡ 1.4 full-store); **not** remote scrub.

**`cache-max-bytes` ≠ LRU ≠ trim ≠ GC ≠ sync:**

| Flag / command | Role |
|---|---|
| `--cache-max-bytes` | Cap **new** cache fills; over budget skip `put`; read still succeeds |
| `store stats` / `du` | Observe local CAS size; **not** an eviction knob |
| `gc` / `store scrub` | Local unreferenced / bitrot; **not** cache policy |

**Not delivered / non-goals (carry forward):** `bytes_plaintext` (store stats
P1); packfile; write mount / COW; bidirectional sync; extract prune /
`--delete`; remote scrub; full aws-sdk / multipart / IMDS / SSO / ListObjects;
byte-range resume; push listing upload; **cache LRU** / auto trim; changing
default jobs·retries·SigV4·progress; absolute perf SLA in CI; video analysis.
Pack stance: [docs/perf.md](docs/perf.md) — **1.5.0 still does not implement pack**.

```bash
# Phase 15 cache-budget + ops-json smoke (~minutes; local file primary)
cargo build -p chunkforge-cli
bash scripts/demo_cache_budget_ops_json.sh
# A: make --format json (parse ok/bytes/chunks/new/reused)
# B: cat --format json + --cache + small --cache-max-bytes
#    first blob fills; second distinct blob: disk unchanged, read OK
# C: --cache-max-bytes without --cache → clear non-zero

bash scripts/check_compat_1_4.sh   # includes 1_3 → 1_2 → 1_1 → 1_0 + 1.5 flags
./target/debug/chunkforge --version   # → chunkforge 1.5.0

# Manual sketch (same semantics):
# ./target/debug/chunkforge make --store ./store -o a.cfidx --format json ./a.bin
# ./target/debug/chunkforge cat --source ./store --cache ./cache \
#   --cache-max-bytes 10000 -o /tmp/out.bin --format json ./a.cfidx
# ./target/debug/chunkforge store stats --store ./cache --format json
# ./target/debug/chunkforge store scrub --store ./store --listing ./a.cfidx
```

Details: [docs/ops-json.md](docs/ops-json.md), [docs/mount.md](docs/mount.md),
[docs/stability.md](docs/stability.md), [docs/perf.md](docs/perf.md),
[docs/doctor-gc.md](docs/doctor-gc.md). Gate:
[`scripts/check_compat_1_4.sh`](scripts/check_compat_1_4.sh).


## Phase 16 / 1.6.0: `--fallback` + byte suffixes + `bytes_plaintext`

Phase 16 closes **read-path Failover** and **ops sugar** at **1.6.0**:
opt-in **`--fallback`** (Missing-only; outer Cache wraps the whole chain),
human **`--cache-max-bytes`** suffixes (`1M` …), **`store stats`
`bytes_plaintext`/`--decode`**, `demo_fallback_bytes_suffix`, and
**`check_compat_1_5`** (+ P1 `diff --path`/`--exclude`/`--exclude-from`).
All additive; **defaults ≡ 1.5.0** (no `--fallback` ⇒ single origin;
plain-integer `--cache-max-bytes` still works; no `--decode` ⇒ zstd stats
stay cheap; no diff path flags ⇒ full listing; jobs=1, retries=0, SigV4 off,
progress off, mount prefetch depth 1). Workspace / CLI version is **1.6.0**.

**Delivered:**

- **`--fallback <PATH|URL>`** (repeatable) on `cat` / `verify` / `extract` /
  `mount` / `pull` / `doctor`: Missing-only failover behind primary; Transient /
  Corrupt fail fast. Recommended composition: **outer Cache wraps the whole
  Fallback chain**. Zero times ≡ **1.5** single origin. **≠ cache ≠ sync ≠
  prune ≠ write-back**. Does **not** apply to `push --dest`.
- **`--cache-max-bytes` human suffixes**: `1M` / `64Mi` / `K`/`G`/`Ki`/`Gi`
  (1024-base) alongside plain integers; illegal suffixes → clear non-zero.
  Refuse-fill semantics unchanged (≠ LRU).
- **`store stats` / `du` `bytes_plaintext`**: `compression=none` ⇒ equals
  `bytes_on_disk` (cheap); zstd ⇒ `null` unless opt-in **`--decode`**.
  Observation only — **≠ trim ≠ LRU**.
- Docs: [docs/ops-json.md](docs/ops-json.md) fallback narrative;
  [docs/mount.md](docs/mount.md) / [docs/extract.md](docs/extract.md) /
  [docs/pull.md](docs/pull.md) (`fallback` ≠ `cache` ≠ sync);
  [docs/stability.md](docs/stability.md) 1.6.0 opt-in;
  [docs/perf.md](docs/perf.md) «1.6.0 still does not implement pack».
- Smoke: [`scripts/demo_fallback_bytes_suffix.sh`](scripts/demo_fallback_bytes_suffix.sh).
- **Compat gate**: [`scripts/check_compat_1_5.sh`](scripts/check_compat_1_5.sh)
  — runs `check_compat_1_4.sh`, asserts 1.6 help flags (`--fallback` on read
  commands; `cat`/`mount --cache-max-bytes`; `store stats` `--decode` /
  `bytes_plaintext`); thin non-goals (no pack / `--delete` / LRU / aws-sdk);
  demo presence. No absolute perf SLA. Keeps `check_compat_1_0`…`1_4`
  independently runnable.
- **P1 `diff --path` / `--exclude` / `--exclude-from`**: narrow both sides with
  `PathFilter` before compare; default no flags ≡ 1.5 full diff; JSON field
  names unchanged. **Not** sync / prune.

**`fallback` ≠ `cache` ≠ sync:**

| Mechanism | Role |
|---|---|
| `--fallback` | Read-only multi-origin; Missing → next; never writes origins |
| `--cache` (+ `--cache-max-bytes`) | Writes cache store on miss; over budget refuse-fill |
| Sync / prune / write mount / LRU | **Not implemented** |

**Not delivered / non-goals (carry forward):** O3 `archive`/`extract`/`make
--progress`; packfile; write mount / COW; bidirectional sync; extract prune /
`--delete`; remote scrub; full aws-sdk / multipart / IMDS / SSO / ListObjects;
byte-range resume; push listing upload; **cache LRU** / auto trim; Transient
auto-switch to next fallback; `--fallback` on `push --dest`; changing default
jobs·retries·SigV4·progress; absolute perf SLA in CI; video analysis.
Pack stance: [docs/perf.md](docs/perf.md) — **1.6.0 still does not implement pack**.

```bash
# Phase 16 fallback + suffix + bytes_plaintext smoke (~minutes; local dual store)
cargo build -p chunkforge-cli
bash scripts/demo_fallback_bytes_suffix.sh
# A: primary missing chunks + --fallback mirror → cat/verify OK
# B: --cache + --cache-max-bytes 1M parse smoke
# C: store stats --format json → bytes_plaintext ≡ bytes_on_disk (none)
# D: store stats --decode (none → no-op) smoke

bash scripts/check_compat_1_5.sh   # includes 1_4 → … → 1_0 + 1.6 flags
bash scripts/check_compat_1_4.sh   # still independently green
./target/debug/chunkforge --version   # → chunkforge 1.6.0

# Manual sketch:
# ./target/debug/chunkforge cat --source ./primary --fallback ./mirror \
#   -o /tmp/out.bin ./blob.cfidx
# ./target/debug/chunkforge cat --source ./store --cache ./cache \
#   --cache-max-bytes 1M -o /tmp/out2.bin ./blob.cfidx
# ./target/debug/chunkforge store stats --store ./store --format json
# ./target/debug/chunkforge store stats --store ./store --decode --format json
```

Details: [docs/ops-json.md](docs/ops-json.md), [docs/mount.md](docs/mount.md),
[docs/extract.md](docs/extract.md), [docs/pull.md](docs/pull.md),
[docs/stability.md](docs/stability.md), [docs/perf.md](docs/perf.md).
Gate: [`scripts/check_compat_1_5.sh`](scripts/check_compat_1_5.sh)
(calls [`check_compat_1_4.sh`](scripts/check_compat_1_4.sh)).


## Phase 17 / 1.7.0: create-time `--compression` + long-job `--progress`

Phase 17 closes **CLI opt-in local store zstd** and **symmetric long-job
progress** at **1.7.0**: create-time **`--compression none|zstd`** on
`make` / `archive` (omit ≡ **`none`** ≡ 1.6), and **`archive` / `extract` /
`make --progress`** (default **off** ≡ 1.6; reuse `ProgressReporter`). All
additive; **defaults ≡ 1.6.0** (create none; progress off; jobs=1, retries=0,
SigV4 off, text, mount prefetch depth 1, no `--fallback` ⇒ single origin).
Workspace / CLI version is **1.7.0**.

**Delivered:**

- **`--compression none|zstd`** (create-time only): applied when `meta.toml` is
  absent; existing stores open by meta (explicit conflict → clear non-zero).
  `chunkforge-cli` enables the store `zstd` feature. Disk encoding is **not**
  HTTP Content-Encoding / wire compression and **not** pack — `get` / HTTP PUT
  bodies stay **plaintext**.
- **`archive` / `extract` / `make --progress`**: stderr
  `progress: op=archive|extract|make done=N/TOTAL`; orthogonal to
  `--format json` (JSON → stdout). Default off ≡ 1.6.
- Docs: [docs/ops-json.md](docs/ops-json.md) (`--progress` ↔ JSON orthogonal;
  compression does not reshape ops JSON; `store stats` `compression` reflects
  meta); [docs/stability.md](docs/stability.md) 1.7.0 opt-in;
  [docs/perf.md](docs/perf.md) «1.7.0 still does not implement pack»;
  [docs/remote-layout.md](docs/remote-layout.md) / [docs/archive.md](docs/archive.md) /
  [docs/extract.md](docs/extract.md) (disk zstd ≠ wire ≠ pack).
- Smoke: [`scripts/demo_zstd_progress.sh`](scripts/demo_zstd_progress.sh).
- Gate: [`scripts/check_compat_1_6.sh`](scripts/check_compat_1_6.sh) (calls
  `check_compat_1_5` + asserts `--compression` / archive·extract·make
  `--progress`; thin no-pack / no-`--delete` / no-LRU / no-aws-sdk; does **not**
  treat absolute perf as CI SLA).
- P1 **CacheSource** observation counters: `hits` / `miss_fills` /
  `miss_refused` (get-path only; **≠** LRU / trim).

**Not delivered in 1.7 / non-goals (carry forward):** packfile; write mount /
COW; bidirectional sync; extract prune / `--delete`; remote scrub; full
aws-sdk / multipart / IMDS / SSO / ListObjects; byte-range resume; push
listing upload; **cache LRU** / auto trim; **default** store zstd; HTTP
Content-Encoding / wire compression; `store recompress`; changing default
jobs·retries·SigV4·progress; absolute perf SLA in CI; video analysis.
(`cat`/`verify --progress`, `pull --verify`, Cache CLI/JSON productization
land in **Phase 18** — see below.) Pack stance: [docs/perf.md](docs/perf.md)
— **1.7.0 still does not implement pack**.

```bash
# Phase 17 zstd + progress smoke (~minutes; local only)
cargo build -p chunkforge-cli
bash scripts/demo_zstd_progress.sh
# A: make --compression zstd → store stats compression=zstd; --decode plaintext
# B: default make → compression=none
# C: archive/extract --progress → stderr progress: op=…
# D: optional make --progress → progress: op=make

bash scripts/check_compat_1_6.sh
./target/debug/chunkforge --version   # → chunkforge 1.7.0
```

Details: [docs/ops-json.md](docs/ops-json.md), [docs/stability.md](docs/stability.md),
[docs/perf.md](docs/perf.md), [docs/remote-layout.md](docs/remote-layout.md),
[docs/archive.md](docs/archive.md), [docs/extract.md](docs/extract.md).
Gate: [`scripts/check_compat_1_6.sh`](scripts/check_compat_1_6.sh)
(calls [`check_compat_1_5.sh`](scripts/check_compat_1_5.sh)).

## Phase 18 / 1.8.0: `pull --verify` + Cache observation + cat/verify `--progress`

Phase 18 closes **push/pull post-verify symmetry**, **CacheSource observation
productization**, and **read-side reassemble progress** at **1.8.0**: opt-in
**`pull --verify`** (verify local `--store` after a successful pull; dry-run /
failed pull skip; default **off** ≡ 1.7), **`--cache-stats`** stderr +
ops-json additive **`cache_hits` / `cache_miss_fills` / `cache_miss_refused`**
when `--cache` (observation only; **≠** LRU / trim), **`cat` / `verify
--progress`** (default **off** ≡ 1.7; reuse `ProgressReporter`; per listing
chunk), and **`check_compat_1_7`** (+ P1 **`doctor --progress`**). All
additive; **defaults ≡ 1.7.0**. Workspace / CLI version is **1.8.0**.

**Delivered:**

- **`pull --verify`**: after successful fetch, treat `--store` as
  `ChunkSource` and verify each listing (symmetric to `push --verify` on
  dest). See [docs/pull.md](docs/pull.md).
- **`--cache-stats`** + ops-json **`cache_*`**: stderr
  `cache: hits=H miss_fills=F miss_refused=R` (requires `--cache`); JSON
  additive fields when `--cache` + `--format json` (omit without `--cache`).
  **≠ LRU ≠ sync ≠ fallback**. See [docs/ops-json.md](docs/ops-json.md) /
  [docs/remote-layout.md](docs/remote-layout.md).
- **`cat` / `verify --progress`**: stderr
  `progress: op=cat|verify done=N/TOTAL`; orthogonal to `--format json` /
  `--jobs` / `--cache` / `--fallback` / `--cache-stats`. Default off ≡ 1.7.
- **P1 `doctor --progress`**: stderr `progress: op=doctor done=N/TOTAL` per
  checked chunk; default off ≡ 1.7; orthogonal to `--format` / `--jobs` /
  `--cache` / `--fallback` / `--cache-stats`.
- Docs: [docs/ops-json.md](docs/ops-json.md) (progress orthogonal; `cache_*`;
  pull `--verify` does not reshape JSON); [docs/stability.md](docs/stability.md)
  1.8 opt-in; [docs/pull.md](docs/pull.md) `--verify` semantics;
  [docs/perf.md](docs/perf.md) «1.8.0 still does not implement pack»;
  [docs/remote-layout.md](docs/remote-layout.md) cache-stats ≠ LRU ≠ sync ≠
  fallback.
- Smoke: [`scripts/demo_pull_verify_cache_stats.sh`](scripts/demo_pull_verify_cache_stats.sh).
- Gate: [`scripts/check_compat_1_7.sh`](scripts/check_compat_1_7.sh) (calls
  [`check_compat_1_6.sh`](scripts/check_compat_1_6.sh)).

**Not delivered / deferred:** P1 `make --jobs` (FastCDC streaming chunking is
inherently serial per file; not forced as fake jobs).

**Not delivered / non-goals:** packfile; write mount / COW; bidirectional
sync; extract prune / `--delete`; remote scrub; full aws-sdk / multipart /
IMDS / SSO / ListObjects; byte-range resume; push listing upload; **cache
LRU** / auto trim; **default** store zstd; HTTP Content-Encoding / wire
compression; `store recompress`; changing default jobs·retries·SigV4·progress;
absolute perf SLA in CI; video analysis; offline bundle; `.cfdir` v2 symlink.
Pack stance: [docs/perf.md](docs/perf.md) — **1.8.0 still does not implement
pack**.

```bash
# Phase 18 pull --verify + cache-stats + cat/verify --progress smoke (~minutes; local only)
cargo build -p chunkforge-cli
bash scripts/demo_pull_verify_cache_stats.sh
# A: pull --verify → success path verifies --store
# B: --cache + --cache-stats → stderr cache: hits=…
# C: cat --progress / verify --progress → stderr progress: op=…
# D: default quiet path (no progress/cache/verify noise)

bash scripts/check_compat_1_7.sh
bash scripts/check_compat_1_6.sh
./target/debug/chunkforge --version   # → chunkforge 1.8.0
./target/debug/chunkforge doctor --help | grep -F -- --progress
./target/debug/chunkforge make --help | grep -F -- --jobs || true  # still absent
```

Details: [docs/ops-json.md](docs/ops-json.md), [docs/stability.md](docs/stability.md),
[docs/pull.md](docs/pull.md), [docs/perf.md](docs/perf.md),
[docs/remote-layout.md](docs/remote-layout.md).
Gate: [`scripts/check_compat_1_7.sh`](scripts/check_compat_1_7.sh)
(calls [`check_compat_1_6.sh`](scripts/check_compat_1_6.sh)).

## Phase 19 / 1.9.0: `store create` + `pull --compression` + `diff --progress` (+ P1 `make --jobs`)

Phase 19 closes **store lifecycle**, **pull create-time compression
symmetry**, and **diff progress** at **1.9.0**: **`chunkforge store create
--store … [--compression none|zstd] [--format text|json]`** (calls
`Store::create`; existing → non-zero; omit ≡ **none** ≡ 1.8),
**`pull --compression`** (same create semantics as make/archive/`store create`;
omit ≡ none ≡ 1.8; existing store by meta / conflict → non-zero; dry-run never
creates), and **`diff --progress`** (default **off** ≡ 1.8; stderr
`progress: op=diff done=N/TOTAL`; TOTAL = filtered File-path union; orthogonal
to `--format json`). P1: honest **`make --jobs`** (default **1** ≡ 1.8;
FastCDC cut-points stay serial; only post-chunk store put / on-disk zstd
encoding is concurrent — **not** parallel FastCDC). All additive;
**defaults ≡ 1.8.0**. Workspace / CLI version is **1.9.0**.

**Responsibility (store create):** **`store create` ≠ recompress ≠ default
zstd ≠ pack** — create empty CAS only; never migrates an existing store;
creation default remains **none**; loose `.cnk` layout unchanged.

**Delivered:**

- **`store create`**: empty local CAS; json fields `ok` / `store` /
  `compression`. **≠** recompress / trim / default zstd / pack. See
  [docs/store.md](docs/store.md).
- **`pull --compression`**: create-time only; does **not** rename pull JSON
  fields. See [docs/pull.md](docs/pull.md).
- **`diff --progress`**: stderr only; orthogonal to `--format json`. See
  [docs/diff.md](docs/diff.md).
- Docs: [docs/ops-json.md](docs/ops-json.md) (`store create` row; pull
  compression note; diff progress orthogonal); [docs/stability.md](docs/stability.md)
  1.9 opt-in; [docs/perf.md](docs/perf.md) «Phase19 / 1.9.0 still does not
  implement pack»; [docs/remote-layout.md](docs/remote-layout.md) Phase 19 tags.
- Smoke: [`scripts/demo_store_create_pull_compression.sh`](scripts/demo_store_create_pull_compression.sh).
- Gate: [`scripts/check_compat_1_8.sh`](scripts/check_compat_1_8.sh) (calls 1_7 + 1.9 flags).
- **P1 `make --jobs`**: post-chunk put concurrency; default 1; help honest.

**Non-goals:** packfile; write mount / COW; bidirectional sync; extract prune
/ `--delete`; remote scrub; full aws-sdk / multipart / IMDS / SSO /
ListObjects; byte-range resume; push listing upload; **cache LRU** / auto
trim; **default** store zstd; HTTP Content-Encoding / wire compression;
**`store recompress`**; **`push --fallback`** / multi dest; changing default
jobs·retries·SigV4·progress·create compression; absolute perf SLA in CI;
video analysis; offline bundle; `.cfdir` v2 symlink; claiming parallel
FastCDC. Pack stance:
[docs/perf.md](docs/perf.md) — **Phase19 / 1.9.0 still does not implement
pack**.

```bash
# Phase 19 store create + pull --compression + diff --progress smoke (~minutes; local only)
cargo build -p chunkforge-cli
bash scripts/demo_store_create_pull_compression.sh
# A: store create --compression zstd → pull into that store (omit compression)
# B: pull omit → new store compression=none (≡ 1.8)
# C: pull --compression zstd → new zstd store + fetch
# D: diff --progress → stderr progress: op=diff
# E: default paths quiet (no progress noise)
# F: repeat store create → non-zero (create ≠ recompress)

./target/debug/chunkforge --version   # → chunkforge 1.9.0
bash scripts/check_compat_1_8.sh
```

Details: [docs/ops-json.md](docs/ops-json.md), [docs/stability.md](docs/stability.md),
[docs/store.md](docs/store.md), [docs/pull.md](docs/pull.md),
[docs/diff.md](docs/diff.md), [docs/perf.md](docs/perf.md).

## Phase 20 / 1.10.0: `--path-from` + `doctor`/`verify` path scope (+ P1 push local dest)

Phase 20 closes the **PathFilter include-file** gap and **doctor/verify path
symmetry** at **1.10.0**: repeatable **`--path-from FILE`** (UTF-8; one include
prefix per line; blank / `#` / trim ≡ `--exclude-from`; library
**`load_path_file`**; merged OR with `--path`) on **archive / extract / push /
pull / diff / doctor / verify**, and the full path quartet on **`doctor` /
`verify`** (only File entries; Dir never contribute chunks; default no flags ≡
**1.9** full set; `.cfidx` + any path flag → clear non-zero). JSON field
**names** unchanged; filtered counts may shrink. P1: **`push --dest`** accepts
local path / `file://` via **`Store` as `ChunkSink`** (single dest; create
**none**). All additive; **defaults ≡ 1.9.0**. Workspace / CLI version is
**1.10.0**.

**Responsibility:** **`path-from` ≠ prune ≠ gc-path ≠ sync ≠ pack** —
filtering only shrinks what is archived / extracted / pushed / pulled /
diffed / **checked**. `gc` keeps the **full** listing reference set (**no**
`--path`). Extract still never deletes extras. **Push local ≠ `--fallback` /
multi-dest**. Not packfile, not write mount, not bidirectional sync.

**Delivered:**

- Library **`load_path_file`** + CLI **`--path-from`** (M1–M2).
- **`doctor` / `verify`** `--path` / `--exclude` / `--exclude-from` /
  `--path-from` (M3).
- Docs + smoke:
  [`scripts/demo_path_from_doctor_verify.sh`](scripts/demo_path_from_doctor_verify.sh)
  (M4).
- Gate **`check_compat_1_9.sh`** (M5; calls 1_8; requires executable).
- P1 **`push` local / `file://` dest** (M6).
- Workspace / CLI **1.10.0** (M7).

**Non-goals:** packfile; write mount / COW; bidirectional sync; extract prune
/ `--delete`; **`gc --path`**; remote scrub; full aws-sdk / multipart / IMDS /
SSO / ListObjects; byte-range resume; push listing upload; cache LRU; **default**
zstd; HTTP wire compression; `store recompress`; `push --fallback`; multi dest;
changing default jobs·retries·SigV4·progress·create compression; absolute perf
SLA in CI; video analysis; offline bundle; `.cfdir` v2 symlink. Pack stance:
[docs/perf.md](docs/perf.md) — **Phase20 / 1.10.0 still does not implement pack**.

```bash
# Phase 20 path-from + doctor/verify path smoke (~minutes; local only)
cargo build -p chunkforge-cli
bash scripts/demo_path_from_doctor_verify.sh
# A: archive --path-from + --exclude-from
# B: doctor/verify --path / --path-from subset (counts may shrink)
# C: no flags ≡ 1.9 full + quiet
# D: path-from + exclude-from combined
# E: missing path-from file → non-zero
# F: .cfidx + path → non-zero
# G: gc --help has no --path (path-from ≠ gc-path)
# H: version 1.10.0; check_compat_1_8 + check_compat_1_9 present + executable

./target/debug/chunkforge --version   # → chunkforge 1.10.0
bash scripts/check_compat_1_9.sh
```

Details: [docs/ops-json.md](docs/ops-json.md), [docs/stability.md](docs/stability.md),
[docs/archive.md](docs/archive.md), [docs/pull.md](docs/pull.md),
[docs/push.md](docs/push.md), [docs/diff.md](docs/diff.md),
[docs/doctor-gc.md](docs/doctor-gc.md), [docs/perf.md](docs/perf.md).

## Phase 21 / 1.11.0: `mount` path scope (+ P1 push --compression / store list)

Phase 21 closes the **`.cfdir` tree-consumption path asymmetry** at
**1.11.0**: archive/extract/push/pull/diff/doctor/verify already had the
PathFilter quartet; **`mount`** now gets the same **`--path` / `--exclude` /
`--exclude-from` / `--path-from`** (library `filter_dir_archive` → `DirFs`;
empty filter ≡ identity ≡ **1.10** full tree; `.cfidx` + any path flag → clear
non-zero). Still **read-only**. P1: **`push --compression`** (local/`file://`
dest **create**; omit ≡ none) and **`store list`** (sorted hex / `--format
json`). All additive; **defaults ≡ 1.10.0**. Workspace / CLI version is
**1.11.0**.

**Responsibility:** **`mount path` ≠ write mount ≠ prune ≠ gc-path ≠ sync ≠
pack** — filtering only **shows fewer** paths under the mount point. `gc`
keeps the full listing reference set (**no** `--path`). Extract still never
deletes extras. **`push --compression` ≠ recompress ≠ default zstd**.
**`store list` ≠ GC ≠ scrub ≠ trim ≠ LRU**. Not packfile, not write mount, not
bidirectional sync, not mount `--progress`.

**Delivered:**

- Library **`filter_dir_archive`** + DirFs wiring (M1).
- **`mount`** path quartet CLI (M2).
- Docs + smoke:
  [`scripts/demo_mount_path.sh`](scripts/demo_mount_path.sh) (M3; DirFs /
  `filter_dir_archive` library asserts are primary; real FUSE optional).
- Gate **`check_compat_1_10.sh`** (M4; calls 1_9; requires executable).
- P1 **`push --compression`** (M5; local/`file://` dest create; omit ≡ none).
- P1 **`store list`** (M6; sorted hex ids / `--format json`; **≠** GC/scrub/trim/LRU)
  + thin docs (`docs/store.md` / ops-json responsibility nails).
- Workspace / CLI **1.11.0** (M7).

**Non-goals:** packfile; write mount / COW; bidirectional sync; extract prune
/ `--delete`; **`gc --path`**; remote scrub; full aws-sdk / multipart / IMDS /
SSO / ListObjects; byte-range resume; push listing upload; cache LRU;
**default** zstd; HTTP wire compression; `store recompress`; `push --fallback`;
multi dest; mount `--progress`; changing default jobs·retries·SigV4·progress·
create compression; absolute perf SLA in CI; video analysis; offline bundle;
default-record symlink (Phase22 delivers **opt-in** record only; see Phase 22
section). Pack stance: [docs/perf.md](docs/perf.md) — **Phase21 /
1.11.0 still does not implement pack**.

```bash
# Phase 21 mount path smoke (~minutes; local only; no /dev/fuse required)
cargo build -p chunkforge-cli
bash scripts/demo_mount_path.sh
# A: archive full tree
# B: library DirFs / filter_dir_archive subset + empty≡full (PRIMARY)
# C: optional real FUSE --path / path-from+exclude-from / no-flags full
# D: .cfidx + path → non-zero; missing path-from → non-zero
# E: mount --help has quartet; gc --help has no --path
# F: version 1.11.0; check_compat_1_9 + check_compat_1_10 present + executable

./target/debug/chunkforge --version   # → chunkforge 1.11.0
bash scripts/check_compat_1_10.sh
```

Details: [docs/mount.md](docs/mount.md), [docs/stability.md](docs/stability.md),
[docs/ops-json.md](docs/ops-json.md), [docs/perf.md](docs/perf.md).


## Phase 22 / 1.12.0: `.cfdir` Symlink opt-in (+ P1 make --dry-run)

Phase 22 closes the **directory-tree fidelity** gap at **1.12.0**: opt-in
**`archive --symlinks record`** writes `DirEntryKind::Symlink`
(`KIND_SYMLINK=3`) and bumps the listing to **`format_version=2`** when ≥1
Symlink is present. Default **`--symlinks skip`** (or omit) ≡ **1.11.0**
skip+warn + default write **`format_version=1`**. Extract materializes
symlinks; DirFs exposes **`readlink`**; mount stays **read-only**. Absolute
targets → clear non-zero; directory symlinks are **not** followed. Path
filter treats Symlink paths like Files. P1: **`make --dry-run`** (plan-only
FastCDC + `would_write`/`would_reuse`; no store create/put; no `.cfidx`
write). All additive; **defaults ≡ 1.11.0**. Workspace / CLI version is
**1.12.0**.

**Responsibility:** **`archive --symlinks record` ≠ write mount ≠ follow dir
symlink ≠ pack ≠ offline bundle ≠ prune ≠ `gc --path` ≠ default record**.
**`make --dry-run` ≠ seed ≠ pack ≠ recompress**. Default path stays
quiet-compatible with 1.11.

**Delivered:**

- Library Symlink kind + v2 encode/decode (M1).
- `archive --symlinks skip|record` CLI (M2).
- extract / verify / doctor / filter / seed / diff wiring (M3).
- DirFs Symlink + `readlink` (still RO) (M4).
- Docs + smoke: [`scripts/demo_symlink.sh`](scripts/demo_symlink.sh) (M5).
- Gate **`check_compat_1_11.sh`** (M6; calls 1_10; requires executable).
- P1 **`make --dry-run`** (M7; text `would_*`; JSON `dry_run`/`would_write`/
  `would_reuse`; omit ≡ real write) + workspace / CLI **1.12.0**.

**Non-goals:** packfile; write mount / COW; bidirectional sync; extract prune
/ `--delete`; **`gc --path`**; remote scrub; full aws-sdk / multipart / IMDS /
SSO / ListObjects; byte-range resume; push listing upload; cache LRU;
**default** zstd; HTTP wire compression; `store recompress`; `push --fallback`;
multi dest; mount `--progress`; **default record symlink**; follow directory
symlink; fifo / socket / device / xattr / ACL; offline bundle; changing
default jobs·retries·SigV4·progress·create compression; absolute perf SLA in
CI; video analysis. Pack stance: [docs/perf.md](docs/perf.md) — **Phase22 /
1.12.0 still does not implement pack**.

```bash
# Phase 22 symlink smoke (~minutes; local only; no /dev/fuse required)
cargo build -p chunkforge-cli --features fuse
bash scripts/demo_symlink.sh
# A: default skip ≡ 1.11 quiet (v1 + skipped_symlinks; no Symlink kind)
# B: --symlinks record → extract → readlink matches
# C: absolute target → non-zero
# D: path filter keeps/excludes symlink
# E: library DirFs readlink (PRIMARY)
# F: optional real FUSE readlink + RO write-fail
# G: help / ≠ prune / ≠ gc-path
# H: version 1.12.0; check_compat_1_10 + check_compat_1_11 present + executable

./target/debug/chunkforge --version   # → chunkforge 1.12.0
bash scripts/check_compat_1_11.sh
```

Details: [docs/dir-format.md](docs/dir-format.md), [docs/archive.md](docs/archive.md),
[docs/extract.md](docs/extract.md), [docs/mount.md](docs/mount.md),
[docs/stability.md](docs/stability.md), [docs/ops-json.md](docs/ops-json.md),
[docs/perf.md](docs/perf.md).

## Phase 23 / 1.13.0: `diff --tree --symlinks` (+ P1 would_symlinks)

Phase 23 closes the **tree↔listing Symlink asymmetry** at **1.13.0**: after
Phase22, listing↔listing already compared Symlink target/mode, but
`diff --tree` hardcoded skip. Opt-in **`diff --tree --symlinks record`**
builds ephemeral `DirEntryKind::Symlink` entries on the tree side (target
as-is; mode from `symlink_metadata`; **0 chunks**; **not** followed;
absolute/empty target → clear non-zero). Default **`--symlinks skip`** (or
omit) ≡ **1.12.0** tree skip+warn. Clap **`requires = "tree"`** — the flag
is rejected without `--tree` (listing↔listing already compares Symlink
in-lib). P1: extract dry-run additive **`would_symlinks`** (`would_write`
still includes symlink ≡ 1.12). All additive; **defaults ≡ 1.12.0**.
Workspace / CLI version is **1.13.0**.

**Responsibility:** **`diff --tree --symlinks record` ≠ write mount ≠ follow
≠ pack ≠ sync ≠ prune ≠ `gc --path` ≠ default record**.
**`would_symlinks` ≠ prune ≠ sync ≠ pack ≠ write mount**.

**Delivered:**

- `diff --symlinks skip|record` + ephemeral tree record path (M1).
- Path quartet orthogonal + progress honesty for Symlink ticks (M2).
- Correctness matrix (identical record → exit 0; skip path; kind mismatch) (M3).
- Docs + smoke: [`scripts/demo_diff_tree_symlink.sh`](scripts/demo_diff_tree_symlink.sh)
  (M4); [docs/diff.md](docs/diff.md) / [docs/stability.md](docs/stability.md) /
  [docs/perf.md](docs/perf.md).
- Gate **`check_compat_1_12.sh`** (M5; calls 1_11; requires executable).
- P1 extract dry-run additive **`would_symlinks`** (M6; `would_write` still
  includes symlink would-writes ≡ 1.12; always emit incl. 0) —
  [docs/ops-json.md](docs/ops-json.md) / [docs/extract.md](docs/extract.md).
- Workspace / CLI **1.13.0** (M7 closeout).

**Non-goals (unchanged):** packfile; write mount / COW; bidirectional sync;
extract prune / `--delete`; **`gc --path`**; remote scrub; full aws-sdk /
multipart / IMDS / SSO / ListObjects; byte-range resume; push listing upload;
cache LRU; **default** zstd; HTTP wire compression; `store recompress`;
`push --fallback`; multi dest; mount `--progress`; **default record
symlink**; follow directory symlink; fifo / socket / device / xattr / ACL;
offline bundle; changing default jobs·retries·SigV4·progress·create
compression; absolute perf SLA in CI; video analysis. Pack stance:
[docs/perf.md](docs/perf.md) — **Phase23 / 1.13.0 still does not implement
pack**.

```bash
# Phase 23 tree↔listing Symlink smoke (~minutes; local only)
cargo build -p chunkforge-cli
bash scripts/demo_diff_tree_symlink.sh
# A: archive --symlinks record → listing
# B: diff --tree default skip → false added / skip warn (exit non-zero OK)
# C: diff --tree --symlinks record → identical exit 0
# D: absolute target → non-zero under record
# E: thin --path on symlink path
# G: version 1.13.0; check_compat_1_11 + check_compat_1_12 present + executable

./target/debug/chunkforge --version   # → chunkforge 1.13.0
bash scripts/check_compat_1_12.sh
```

Details: [docs/diff.md](docs/diff.md), [docs/stability.md](docs/stability.md),
[docs/perf.md](docs/perf.md), [docs/dir-format.md](docs/dir-format.md).

## Phase 24 / 1.14.0: `chunkforge filter` (+ demo + make --seed)

Phase 24 closes the **listing-subset persistence** gap at **1.14.0**:
library **`filter_dir_archive`** becomes a first-class **`chunkforge filter`**
subcommand — read an existing `.cfdir`, apply the same path 四件套 as
archive/extract/mount/diff, encode the subset to `-o`. Empty path flags ≡
**identity**. Symlink keep → encode **v2**; filtering out all Symlinks →
encode **v1**. **`--dry-run`** / **`--force`** / **`--format text|json`**
(`ok` / `dry_run` / `input` / `output` / `files` / `dirs` / `symlinks` /
`excluded`). **Does not** open a store, walk a source tree, prune a dest
tree, or rewrite the input in place.

Workspace / CLI version is **1.14.0**.

**Responsibility:** **`filter` ≠ prune ≠ `gc --path` ≠ sync ≠ write mount ≠
pack ≠ `archive --path`** (latter needs a source-tree walk). Warning: taking a
filtered listing into `gc` uses **that listing's** refs — still **no**
`gc --path`.

**Delivered:**

- `filter` skeleton + `filter_dir_archive` → encode → `-o` (M1).
- Path 四件套 + `--dry-run` / `--force` / `--format text|json` (M2).
- Correctness matrix (Symlink keep / all-filtered → v1 / verify green) (M3).
- Docs + smoke: [`scripts/demo_filter_listing.sh`](scripts/demo_filter_listing.sh)
  (M4); [docs/filter.md](docs/filter.md) / [docs/stability.md](docs/stability.md) /
  [docs/ops-json.md](docs/ops-json.md) / [docs/perf.md](docs/perf.md).
- Gate [`scripts/check_compat_1_13.sh`](scripts/check_compat_1_13.sh) (M5).
- P1 **`make --seed <PRIOR.cfidx>`** / **`--seed-trust-mtime`** + mount help
  File+Symlink honesty (M6); additive ops-json `seed_reused` when seeding.
- Workspace / CLI **1.14.0** (M7 closeout).

**Non-goals (unchanged):** packfile; write mount / COW; bidirectional sync;
extract prune / `--delete`; **`gc --path`**; remote scrub; full aws-sdk /
multipart / IMDS / SSO / ListObjects; byte-range resume; push listing upload;
cache LRU; **default** zstd; HTTP wire compression; `store recompress`;
`push --fallback`; multi dest; mount `--progress`; **default record
symlink**; follow directory symlink; fifo / socket / device / xattr / ACL;
offline bundle; changing default jobs·retries·SigV4·progress·create
compression; absolute perf SLA in CI; video analysis. Pack stance:
[docs/perf.md](docs/perf.md) — **Phase24 / 1.14.0 still does not
implement pack**.

```bash
# Phase 24 filter listing smoke (~minutes; local only)
cargo build -p chunkforge-cli
bash scripts/demo_filter_listing.sh
# A: archive --symlinks record full tree (pkgs/foo + pkgs/bar + symlink)
# B: filter --path pkgs/foo -o foo.cfdir (--format json); Symlink keep → v2
# C: verify --store … foo.cfdir green
# D: empty filter ≡ identity (diff exit 0)
# E: exclude all Symlinks → encode v1
# G: help has filter; gc --help has no --path; no pack
# H: version 1.14.0; check_compat_1_12 + check_compat_1_13 present + executable

./target/debug/chunkforge --version   # → chunkforge 1.14.0
bash scripts/check_compat_1_13.sh
```

Details: [docs/filter.md](docs/filter.md), [docs/stability.md](docs/stability.md),
[docs/ops-json.md](docs/ops-json.md), [docs/perf.md](docs/perf.md),
[docs/dir-format.md](docs/dir-format.md).

## Phase 25 / 1.15.0: `chunkforge ls` + `cat --path` (+ demo + empty-dirs)

Phase 25 closes the **listing inventory + single-File fetch** gap at
**1.15.0**: new subcommand **`chunkforge ls`** (magic-dispatch `.cfidx` /
`.cfdir`; `.cfdir` path 四件套; `--format text|json`; optional `--chunks`;
File / Symlink / Dir rows; **never** opens a store) and extended **`cat
--path`** (`.cfdir` requires exact File path → single `-o`; `.cfidx` ≡ 1.14
omit `--path`). Closes the post-filter "subset listing → human/script
inventory + take one file" product gap without mount or whole-tree extract.

Workspace / CLI version is **1.15.0**.

**Responsibility:** **`ls` ≠ mount ≠ extract ≠ verify ≠ pack ≠ filter**
(read-only inventory; no store; does not persist a subset). **`cat --path` ≠
extract ≠ prune ≠ sync** (one File to `-o`; never deletes a dest tree).

**Delivered:**

- `ls` skeleton + decode wiring + text inventory (M1).
- Path 四件套 + `--format json` + `--chunks` + `cat --path` for `.cfdir` (M2).
- Correctness matrix (Symlink/Dir/error paths; no-store ls; ≡ extract bytes) (M3).
- Docs + smoke: [`scripts/demo_ls_cat_path.sh`](scripts/demo_ls_cat_path.sh)
  (M4); [docs/ls.md](docs/ls.md) / [docs/stability.md](docs/stability.md) /
  [docs/ops-json.md](docs/ops-json.md) / [docs/perf.md](docs/perf.md).
- Gate [`scripts/check_compat_1_14.sh`](scripts/check_compat_1_14.sh) (M5).
- P1 **`archive --empty-dirs`** + `chunk-id`/`store has --format json` (M6);
  omit `--empty-dirs` ≡ 1.14.
- Workspace / CLI **1.15.0** (M7 closeout).

**Non-goals (unchanged):** packfile; write mount / COW; bidirectional sync;
extract prune / `--delete`; **`gc --path`**; remote scrub; full aws-sdk /
multipart / IMDS / SSO / ListObjects; byte-range resume; push listing upload;
cache LRU; **default** zstd; HTTP wire compression; `store recompress`;
`push --fallback`; multi dest; mount `--progress`; **default record
symlink**; follow directory symlink; fifo / socket / device / xattr / ACL;
offline bundle; changing default jobs·retries·SigV4·progress·create
compression; absolute perf SLA in CI; video analysis; re-litigating Phase24
`filter` / Phase23 `diff --tree --symlinks`. Pack stance:
[docs/perf.md](docs/perf.md) — **Phase25 / 1.15.0 still does not
implement pack**.

```bash
# Phase 25 ls + cat --path smoke (~minutes; local only)
cargo build -p chunkforge-cli
bash scripts/demo_ls_cat_path.sh
# A: archive --symlinks record full tree (pkgs/foo + pkgs/bar + symlink)
# B: filter --path pkgs/foo -o foo.cfdir
# C: ls shows retained File + Symlink (text + json; target / size)
# D: cat --path pkgs/foo/a.txt ≡ source bytes
# E: cat --path ≡ extract same File; ≠ prune
# F: .cfidx ls/cat regression; path flags rejected on cfidx
# G: help has ls; cat --help has --path; gc no --path; no pack
# H: version 1.15.0; compat_1_13 + compat_1_14 present+executable

./target/debug/chunkforge --version   # → chunkforge 1.15.0
bash scripts/check_compat_1_14.sh
```

Details: [docs/ls.md](docs/ls.md), [docs/stability.md](docs/stability.md),
[docs/ops-json.md](docs/ops-json.md), [docs/perf.md](docs/perf.md),
[docs/filter.md](docs/filter.md), [docs/dir-format.md](docs/dir-format.md).

## Phase 26 / toward 1.16.0: leaf-Dir + `store get` (docs + demo; version still 1.15.0)

Phase 26 closes the post-1.15 gap where `archive --empty-dirs` wrote an
explicit leaf Dir that **extract `--path`** could materialize, but shared
`filter_dir_archive` dropped it — so `ls --path` / `filter --path` / mount
path / path-scoped `diff` did not see that Dir. The library keep rule now
retains an explicit Dir when a **non-empty** PathFilter `allows` that path
(ancestor Dirs unchanged; **empty filter ≡ identity** ≡ 1.15). It also
productizes library `Store::get` / `get_verify` as **`chunkforge store get`**.

Workspace / CLI version **stays 1.15.0** until M7. This section is the M4
docs + demo slice, not the 1.16.0 closeout.

**Responsibility:** **`filter_dir_archive` leaf-Dir ≠ prune ≠ `gc --path`**
(listing keep of a Dir that already exists; no dest delete; no path-scoped
GC). **`store get` ≠ scrub ≠ cat ≠ extract ≠ recompress ≠ remove** (one
local chunk id → plaintext `-o`). **Not** pack. **Not** write mount.

**Delivered so far (M1–M4):**

- `filter_dir_archive` leaf-Dir keep + library tests (M1).
- `ls` / `filter --path` see the empty leaf; `store get` writes `-o` (M2).
- Correctness matrix: mount/diff path, `--verify`, bad id, no-empty-dirs regression (M3).
- Docs + smoke: [`scripts/demo_empty_dir_path_store_get.sh`](scripts/demo_empty_dir_path_store_get.sh) (M4).

**Not in M4 (later milestones):** `check_compat_1_15.sh` (**M5**; the demo
must not fail if that script is absent), P1 help polish beyond these docs
(**M6**), version bump to **1.16.0** (**M7**).

**Non-goals (unchanged):** packfile; write mount / COW; bidirectional sync;
extract prune / `--delete`; **`gc --path`**; remote scrub; full aws-sdk;
cache LRU; **default** zstd; `store recompress`; `push --fallback`; mount
`--progress`; **default record symlink**; follow; fifo/xattr; offline bundle.
Pack stance: [docs/perf.md](docs/perf.md) — **Phase26 / 1.16.0 still does not
implement pack**.

```bash
# Phase 26 leaf-Dir + store get smoke (~minutes; local only)
cargo build -p chunkforge-cli
bash scripts/demo_empty_dir_path_store_get.sh
# archive --empty-dirs → listing has empty leaf Dir
# ls --path <empty_leaf> prints dir<TAB>…
# filter --path <empty_leaf> keeps Dir; ls of subset is non-empty
# store get <hex> -o bytes match the source chunk
# help nails: ≠ prune / ≠ scrub / ≠ cat / ≠ pack / ≠ write mount / ≠ gc-path
# version still 1.15.0; compat_1_14 required; compat_1_15 note-only

./target/debug/chunkforge --version   # → chunkforge 1.15.0
```

Details: [docs/filter.md](docs/filter.md), [docs/ls.md](docs/ls.md),
[docs/dir-format.md](docs/dir-format.md), [docs/store.md](docs/store.md),
[docs/stability.md](docs/stability.md), [docs/ops-json.md](docs/ops-json.md),
[docs/perf.md](docs/perf.md).

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
