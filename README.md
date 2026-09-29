# ChunkForge

**ChunkForge**: FastCDC content-defined chunking + BLAKE3 content addressing to split large files/blobs into deduplicable chunks, write them into a local CAS store, and reassemble via a custom `.cfidx` index — local make / cat / verify / incremental reuse as a foundation for later multi-backend fetch and read-only mount.

## Non-goals (Phase 1)

| Not this | Why |
|---|---|
| ❌ Not a restic/rustic-style **backup product** | No snapshot policy, encrypted-repo lifecycle, or prune semantics; commands avoid `backup`/`restore` |
| ❌ Not syncthing-style **realtime P2P sync** | No device discovery, continuous watch, or bidirectional conflict resolution |
| ❌ **No GPU / no LLM** | Pure CPU data plane |
| ❌ Phase 1 has **no FUSE** | Read-only mount is Phase 2 |
| ❌ Phase 1 has **no network / remote store** | No HTTP/S3/SFTP; remote backends are Phase 2+ |
| ❌ Not a casync **binary drop-in** | Semantically aligned with casync/desync; **index is NOT casync-compatible** (native `.cfidx`, not `.caibx`) |
| ❌ No full directory-tree archive (`.catar` equivalent) | Phase 1 is **single-blob** (file) only |

## Status

**Phase 1 complete (0.1.0):** M1–M6 — chunk / store / index / CLI / fixtures+dedup demo / CI+README.

## Quick start

```bash
# Requires Rust 1.85+ (edition 2024)
cargo build -p chunkforge-cli
./target/debug/chunkforge make --store ./store -o v1.cfidx ./fixtures/hello.txt
./target/debug/chunkforge verify --store ./store v1.cfidx
./target/debug/chunkforge cat --store ./store v1.cfidx -o /tmp/hello.out
cmp ./fixtures/hello.txt /tmp/hello.out
```

Or via Make: `make build` then the same `make` / `cat` / `verify` flow above.

## Incremental dedup demo

Generate offline ≥64MiB fixtures (gitignored), then remake + mid-file mutate:

```bash
make gen-large          # → fixtures/gen/large-64m.bin (+ -mut)
make demo-dedup         # or: ./scripts/demo_dedup.sh
make demo-dedup-small   # 4MiB smoke (faster)
```

`chunkforge make` stderr reports `new=` / `reused=` from store put outcomes.
See [docs/demo-dedup.md](docs/demo-dedup.md) and [`scripts/demo_dedup.sh`](scripts/demo_dedup.sh).

## Index format note

The `.cfidx` index is **not** casync / desync bit-compatible (different CDC, BLAKE3 vs SHA512/256, custom layout). See [docs/index-format.md](docs/index-format.md).

## Develop / CI

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# optional compression:
cargo test -p chunkforge-store --features zstd
# optional large-file dedup (generates fixtures; ignored by default in CI):
# CHUNKFORGE_GEN_MIB=8 cargo test -p chunkforge-cli --test cli_integration large_file_dedup -- --ignored --nocapture
```

GitHub Actions: [`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs fmt + clippy + test on push to `main`/`master` and on pull requests. The optional 64MiB `gen_large` smoke is **not** part of CI (timeout / disk); ignored large tests stay ignored.

## License

MIT — see [LICENSE](LICENSE). Changelog: [CHANGELOG.md](CHANGELOG.md).
