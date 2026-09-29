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

**M5 (current):** fixtures + incremental dedup demo (`scripts/gen_large.sh`, `make demo-dedup`); `chunkforge make` prints `new=` / `reused=` chunk stats.

## Quick demo

```bash
cargo build -p chunkforge-cli
./target/debug/chunkforge make --store ./store -o v1.cfidx ./fixtures/hello.txt
./target/debug/chunkforge verify --store ./store v1.cfidx
./target/debug/chunkforge cat --store ./store v1.cfidx -o /tmp/hello.out
cmp ./fixtures/hello.txt /tmp/hello.out
```

## Incremental dedup demo

Generate offline ≥64MiB fixtures (gitignored), then remake + mid-file mutate:

```bash
make gen-large          # → fixtures/gen/large-64m.bin (+ -mut)
make demo-dedup         # or: ./scripts/demo_dedup.sh
make demo-dedup-small   # 4MiB smoke
```

`chunkforge make` stderr reports `new=` / `reused=` from store put outcomes.
See [docs/demo-dedup.md](docs/demo-dedup.md) for exact commands.

## Develop

```bash
# Requires Rust 1.85+ (edition 2024); tested on stable 1.98+
cargo test --workspace
# optional compression:
cargo test -p chunkforge-store --features zstd
# optional large-file dedup (generates fixtures; ignored by default):
# CHUNKFORGE_GEN_MIB=8 cargo test -p chunkforge-cli --test cli_integration large_file_dedup -- --ignored --nocapture
```

## License

MIT
