# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`--jobs N`** on `cat` / `verify` / `doctor` / `push` (Phase 4 M5): bounded
  concurrency via `std::thread::scope` in the CLI orchestration layer. Default
  **`1`** preserves 0.3.0 serial behaviour; errors still include the chunk id.
  FUSE mount concurrency unchanged; no tokio; workspace version remains **0.3.0**.

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

[0.3.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.3.0
[0.2.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.2.0
[0.1.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.1.0
