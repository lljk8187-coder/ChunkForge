# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Phase 6 in progress toward **0.6.0** (crate version still **0.5.0** until M7).

### Added

- **`archive --seed <prior.cfdir>`** (Phase 6 P0): incremental directory
  archive — reuse unchanged files' chunk tables via content BLAKE3 (size
  fast-reject); stderr `seed_reused_files=` / `rechunked_files=`; missing
  reused chunks force rechunk + `seed_missing_chunks=`; dry-run prints
  `would_seed_reuse=` / `would_rechunk=`; output remains full `.cfdir` v1 —
  see `docs/archive.md`
- **`archive --jobs N`** (Phase 6 P1): per-file parallel chunking (default
  **1** ≡ serial); seed map read-only; local store puts stay atomic /
  race-safe
- **`chunkforge pull`** (Phase 6 P1): fill a local CAS `--store` with missing
  chunks referenced by `.cfidx` / `.cfdir` listings from `--source`
  (path / `file://` / `http(s)://`); `--jobs` / `--dry-run`; stderr
  `skipped=` / `fetched=` / `failed=`; does not extract trees or upload/download
  listings — see `docs/pull.md`
- **`scripts/demo_seed.sh`**: local ~10 min smoke (archive → change one file →
  `--seed` → verify / extract / optional `pull` via `put_stub`)

### Notes

- Version bump / annotated tag **0.6.0** deferred to Phase6-M7
- Still deferred / non-goals: in-process SigV4 / `aws-sdk-*` / multipart; write
  mount; bidirectional sync; packfile; video analysis; `push` does **not**
  upload listings; `--seed-trust-mtime` not shipped

## [0.5.0] — 2026-09-29

Phase 5 closeout: directory-tree archive (`.cfdir` v1) with `archive` /
`extract` / tree `verify`, read-only FUSE directory mount (`DirFs`), and
`.cfdir`-aware `push` / `doctor` / `gc`, plus `push --verify` and
`archive --dry-run`. Single-blob `.cfidx` v1 stays frozen; no AWS SDK,
in-process SigV4, write mount, or seed archive.

### Added

- **`.cfdir` v1 + `DirArchive`**: parallel multi-file listing format
  (`CFDIR\0\0\x01`); encode/decode + path validation in `chunkforge-index`;
  `.cfidx` v1 bytes unchanged — see `docs/dir-format.md`
- **`chunkforge archive`**: recurse a source directory, FastCDC + BLAKE3 per
  regular file, write chunks into `--store` (dedup), emit `.cfdir`; stderr
  stats `files` / `chunks` / `new=` / `reused=`; symlinks and special files
  skipped with a warning (P0)
- **`archive --dry-run`**: stats only — no store writes and no `.cfdir` output
- **`chunkforge extract`** + **`verify` magic dispatch**: materialize tree
  (create parents; refuse existing targets); `verify` auto-detects `.cfidx`
  vs `.cfdir` (structure + per-file `blob_blake3` + missing chunk fails with
  id); `--jobs` on extract/verify chunk fetches (default 1)
- **Read-only FUSE directory mount (`DirFs`)**: `mount` magic-dispatches
  `.cfdir` → directory tree vs `.cfidx` → single blob; still forced `RO`
- **`push` / `doctor` / `gc` accept `.cfdir`**: reference set is the union of
  chunk ids (`DirArchive::all_chunk_ids`); listings are never uploaded
- **`push --verify`**: after a successful push (`failed=0`), treat `--dest`
  (same HTTP templates) as `ChunkSource` and verify each listing; skipped on
  `--dry-run` or when push already failed (clears Phase 4 O3)
- Docs: `docs/dir-format.md`, `docs/archive.md`; mount / push / doctor-gc
  updates; `scripts/demo_archive.sh`

### Not delivered / deferred (Phase 5)

- **`archive --seed prior.cfdir`** (spec O3): incremental seed / skip re-chunk
  when `blob_blake3` unchanged — deferred (Phase 5.5 / later)
- In-process **SigV4** / complete **`aws-sdk-*`** / S3 multipart upload API
- Write mount / COW / writable FUSE; bidirectional sync / watch directories
- casync `.catar` / `.caibx` bit-compat; video analysis / GPU·LLM / P2P

### Non-goals (Phase 5)

- No rewrite or deprecation of `.cfidx` v1 (single-blob index stays frozen)
- No casync `.catar` bit-compat (semantic alignment only; format is native)
- No write mount / COW; directory mount stays `RO`
- No bidirectional sync / conflict resolution
- No process-in SigV4 / `aws-sdk-*` / multipart; HTTP surface remains **ureq**
- No seed archive in this release; no remote GC / packfiles

### Notes

- Default HTTP chunk layout remains byte-compatible with **0.4.0** / **0.3.0**
  when no template flags are set — `.cfdir` push keys match subsequent
  `verify --source` / `push --verify`
- `make` / single-file `mount` / `.cfidx` `push` behaviour matches **0.4.0**
  when no new flags are used

## [0.4.0] — 2026-09-29

Phase 4 closeout: per-chunk HTTP PUT write path (`ChunkSink` / `HttpChunkSink` /
`chunkforge push`) plus bounded concurrency `--jobs` on read and push commands,
without an AWS SDK, in-process SigV4, or S3 multipart upload API.

### Added

- **`ChunkSink`** write trait in `chunkforge-store` (`has` + `put` → `PutOutcome::{Written,SkippedExists}`); `Store` implements it; **not** folded into read-only `ChunkSource`
- **`HttpChunkSink`** in `chunkforge-remote`: single-object PUT isomorphic with `HttpChunkSource` URL/header templates (`{base}` `{path}` `{2hex}` `{62hex}` `{id}`/`{hex}` `{prefix}` `{env:NAME}`); default `{base}/{path}`; plaintext body; optional `verify_hash`; 2xx / 409 success; ureq only
- **`chunkforge push`**: `--store` + one or more `.cfidx` + `--dest` HTTP(S); merge referenced ids → remote `has` skip / `put`; `--dry-run`; stderr stats `skipped=` / `uploaded=` / `failed=`; non-zero on failures; does **not** upload indexes
- **`--jobs N`** on `cat` / `verify` / `doctor` / `push`: bounded concurrency via `std::thread::scope` in the CLI orchestration layer. Default **`1`** preserves 0.3.0 serial behaviour; errors still include the chunk id. FUSE mount concurrency unchanged; no tokio
- Docs: `docs/push.md`; `docs/remote-layout.md` PUT section; `scripts/demo_push.sh` + `scripts/put_stub.py`

### Not delivered (Phase 4)

- **`push --verify`** (spec O3): not implemented in 0.4.0 — verify after push
  with a separate `chunkforge verify --source <dest> …`. **Delivered in 0.5.0.**

### Non-goals (Phase 4)

- No complete S3 multipart upload API (Initiate / UploadPart / Complete / Abort)
- No `aws-sdk-*` / `aws-config` / ListObjects / credential-provider chain
- No in-process SigV4 (GET or PUT); template headers / external presign / open write endpoints only
- No directory-tree archive / multi-blob container / casync `.catar`; `.cfidx` v1 stays single-blob
- No bidirectional sync / watch directories / conflict resolution
- No write mount / COW / writable FUSE (FUSE stays `RO`)
- No remote GC / bucket lifecycle; no packfile bundling; no P2P / GPU·LLM / video analysis
- macOS / Windows not acceptance platforms

### Notes

- Default HTTP layout remains byte-compatible with **0.3.0** / **0.2.0** when no template flags are set — push keys match subsequent `verify --source`
- Local store may use optional zstd; push decompresses to plaintext before PUT (remote layout = plaintext chunks)

## [0.3.0] — 2026-09-29

Phase 3 closeout: object-store–friendly read paths (URL/header templates + S3 path conventions) plus `doctor` and local `gc`, without an AWS SDK or in-process SigV4.

### Added

- **HTTP URL / header templates** on `HttpChunkSource`: `--url-template` / `--prefix` / `--header` (closed placeholders `{base}` `{path}` `{2hex}` `{62hex}` `{id}`/`{hex}` `{prefix}` `{env:NAME}`); default `{base}/{path}` ≡ Phase 2 layout
- **S3-compatible path conventions** (read-only): default key `{prefix}chunks/{2hex}/{62hex}.cnk`; path-style preferred; virtual-host documented; no bucket field parsing — see `docs/remote-layout.md`
- CLI wiring: `cat` / `verify` / `mount` / `doctor` accept template flags for `http(s)://` sources (non-HTTP + templates → readable error)
- **`chunkforge doctor`**: `.cfidx` readability + chunk presence via `ChunkSource::has` (optional `--deep` uses `get`); missing ids on stdout + non-zero exit; optional HTTP base probe; local `meta.toml` summary
- **`chunkforge gc`**: local dry-run of unreferenced loose `.cnk` (merge chunk ids from given indexes; `Store::list_chunk_ids`); `--apply` deletes serially; **no remote GC**
- Docs: `docs/remote-layout.md` (placeholders, path-style / virtual-host, non-goals); `docs/doctor-gc.md`

### Non-goals (Phase 3)

- No `aws-sdk-*` / full S3 SDK; no in-process SigV4 (even GET-only)
- No upload / multipart / PUT / POST; no bidirectional sync; FUSE stays read-only
- No remote GC / object-storage lifecycle; no auto batch-presign per chunk

### Notes

- Default HTTP layout remains byte-compatible with **0.2.0** when no template flags are set
- Presigned URLs: query may appear in `url_template`; per-object differing signatures are out of scope

## [0.2.0] — 2026-09-29

Phase 2 closeout: read-only FUSE mount + remote chunk fetch skeleton on the Phase 1 local CAS.

### Added

- `ChunkSource` + `SourceError` in `chunkforge-store` (`impl` for `Store`); `CacheSource<P, S>` overlays a local cache store on miss
- `chunkforge-remote`: `HttpChunkSource` (ureq) + `FileUrlSource`; layout docs in `docs/remote-layout.md`
- CLI `cat` / `verify`: `--source` / `--cache` (Phase 1 `--store` retained as a local-path synonym)
- `chunkforge-fuse`: read-only single-blob FUSE library (`BlobFs`, `read_range`, `MountOption::RO` hard-coded); unit tests without `/dev/fuse`
- CLI `mount` (`--source` / `--store` / `--cache` / `--name`); `docs/mount.md`; `scripts/demo_mount.sh`; cargo feature `fuse` (default on Linux)
- Workspace crates: `chunkforge-remote`, `chunkforge-fuse`

### Non-goals (Phase 2)

- No bidirectional sync, write mount / COW write-back, full S3 SDK, or P2P
- No restic-style backup product; no directory-tree archive; macOS/Windows not acceptance platforms

### Notes

- Real FUSE mounts stay `#[ignore]` in CI (need fuse3 + `/dev/fuse`); library FS logic is always tested
- Optional `doctor` / `gc` deferred (not required for 0.2.0)

## [0.1.0] — 2026-09-29

Phase 1 MVP closeout: local content-addressed chunking with make / cat / verify.

### Added

- Workspace crates: `chunkforge-chunk`, `chunkforge-index`, `chunkforge-store`, `chunkforge-cli`
- FastCDC v2020 chunking + BLAKE3 content addressing (default 16KiB / 64KiB / 256KiB)
- Native `.cfidx` v1 index format (not casync bit-compatible)
- Local CAS store (`chunks/<2hex>/<62hex>.cnk`) with atomic put and optional zstd
- CLI: `make`, `cat`, `verify`, `chunk-id`, `store has`
- Fixtures + offline `scripts/gen_large.sh` and incremental dedup demo (`scripts/demo_dedup.sh`)
- GitHub Actions CI: fmt, clippy (`-D warnings`), `cargo test --workspace`

### Non-goals (Phase 1)

- Not a restic/syncthing replacement; no GPU/LLM; no FUSE; no remote/network store;
  no casync binary drop-in; no full directory-tree archive

[0.5.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.5.0
[0.4.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.4.0
[0.3.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.3.0
[0.2.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.2.0
[0.1.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.1.0
