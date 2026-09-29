# ChunkForge

**ChunkForge**: FastCDC content-defined chunking + BLAKE3 content addressing to split large files/blobs into deduplicable chunks, write them into a local CAS store, and reassemble via a custom `.cfidx` index — with on-demand fetch (`ChunkSource`), object-store–friendly HTTP templates, and **read-only** FUSE mount of a single blob.

## Status

| Phase | Version | Delivered |
|---|---|---|
| **Phase 1** | **0.1.0** | Local chunk / store / index / CLI (`make` `cat` `verify` `chunk-id` `store has`) / fixtures + dedup demo / CI |
| **Phase 2** | **0.2.0** | `ChunkSource` + HTTP/`file://` remote + `--source`/`--cache` + read-only `mount` |
| **Phase 3** | **0.3.0** | URL/header templates + S3 path conventions; `doctor`; local `gc` dry-run / `--apply` |

## Non-goals (Phase 3)

| Not this | Why |
|---|---|
| ❌ **Full AWS/S3 SDK** | No `aws-sdk-s3` / `aws-config` / ListObjects — dependency surface stays **ureq** |
| ❌ **In-process SigV4** | No GET-only HMAC either; use public/CDN, fixed header templates, or externally presigned query in `--url-template` |
| ❌ **Upload / multipart** | No PUT/POST, CompleteMultipart, or object writes |
| ❌ **Remote GC / lifecycle** | `gc` only touches a **local** `--store` |
| ❌ **Bidirectional sync** | No watch directories, conflict resolution, or mutual push |
| ❌ **Write mount / COW** | FUSE stays `RO`; writes return `EROFS` / `EACCES` |
| ❌ Not a restic/rustic-style **backup product** | No snapshot policy, encrypted-repo lifecycle, or prune |
| ❌ Not a casync **binary drop-in** | Native `.cfidx` (not `.caibx`); single-blob only — no directory-tree archive |
| ❌ **P2P** / **GPU / LLM** | Pure CPU data plane; no device discovery |
| ❌ macOS / Windows as acceptance platforms | Linux + fuse3 is first-class; other OS are experimental / unsupported |

Earlier phases also deferred FUSE (Phase 1) and object-store templates / doctor / gc (Phase 2); those are now delivered as above.

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

Linux + fuse3: mount a `.cfidx` as a **single** read-only file:

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
# optional real FUSE mount (needs fuse3 + /dev/fuse; ignored by default):
# cargo test -p chunkforge-fuse -- --ignored
```

GitHub Actions: [`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs fmt + clippy + test on push to `main`/`master` and on pull requests. Real FUSE mounts and the optional 64MiB `gen_large` smoke stay **`#[ignore]`** (not part of default CI).

## License

MIT — see [LICENSE](LICENSE). Changelog: [CHANGELOG.md](CHANGELOG.md).
