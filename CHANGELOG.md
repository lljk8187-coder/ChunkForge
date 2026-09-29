# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[0.1.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.1.0
