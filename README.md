# ChunkForge

**ChunkForge** uses FastCDC content-defined chunking + BLAKE3 content addressing to split large files/blobs into deduplicable chunks, write them into a local CAS store, and describe reassembly with a custom index — enabling local make / cat / verify / incremental reuse demos as a foundation for later multi-backend fetch and read-only mount.

## Non-goals (Phase 1)

| Not this | Why |
|---|---|
| ❌ Not a restic/rustic-style **backup product** | No snapshot policy, encrypted-repo lifecycle, or prune semantics |
| ❌ Not syncthing-style **realtime P2P sync** | No device discovery, continuous watch, or bidirectional conflict resolution |
| ❌ **No GPU / no LLM** | Pure CPU data plane |
| ❌ Phase 1 has **no FUSE** | Read-only mount is Phase 2 |
| ❌ Phase 1 has **no network store** | No HTTP/S3/SFTP; remote backends are Phase 2+ |
| ❌ Not a casync **binary drop-in** | Semantically aligned; format is native (not bit-compatible) |
| ❌ No full directory-tree archive (`.catar` equivalent) | Phase 1 is **single-blob** only |

## Status

**M3 (current):** workspace + `chunkforge-chunk` + `chunkforge-store` + `chunkforge-index` (`.cfidx` v1 encode/decode).

Later milestones add the `chunkforge` CLI (`make` / `cat` / `verify`).

## Develop

```bash
# Requires Rust 1.85+ (edition 2024); tested on stable 1.98+
cargo test -p chunkforge-chunk
cargo test -p chunkforge-store
cargo test -p chunkforge-index
# optional compression:
cargo test -p chunkforge-store --features zstd
```

## License

MIT
