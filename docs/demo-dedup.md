# Incremental dedup demo (M5)

Offline fixtures + observable chunk reuse. No network downloads.

## Generate large fixtures

Default: **64 MiB** deterministic `large-64m.bin` plus a mid-file mutated copy
`large-64m-mut.bin`. Output lives under `fixtures/gen/` (gitignored).

```bash
# from repo root
./scripts/gen_large.sh
# or
make gen-large

# smaller / custom size (MiB)
./scripts/gen_large.sh fixtures/gen 4
CHUNKFORGE_GEN_MIB=4 make gen-large
```

Seed defaults to `chunkforge-phase1-v1` (`CHUNKFORGE_GEN_SEED` to override).
Depends only on `python3` + `bash` (no `curl` / `wget`).

## Dedup demo

`chunkforge make` prints `new=` / `reused=` chunk counts from store `put`
outcomes. Same input twice → store `.cnk` file count does not increase; after
mid-file mutation → some new chunks and many reused.

```bash
# full 64MiB path
make demo-dedup
# or
./scripts/demo_dedup.sh

# faster 4MiB smoke
make demo-dedup-small
# or
./scripts/demo_dedup.sh 4
```

Manual steps (equivalent):

```bash
cargo build -p chunkforge-cli
./scripts/gen_large.sh fixtures/gen 64

rm -rf /tmp/cf-dedup-demo && mkdir -p /tmp/cf-dedup-demo
BIN=./target/debug/chunkforge
STORE=/tmp/cf-dedup-demo/store

$BIN make --store "$STORE" -o /tmp/cf-dedup-demo/v1.cfidx fixtures/gen/large-64m.bin
# stderr includes: new=N, reused=0
find "$STORE/chunks" -name '*.cnk' | wc -l

$BIN make --store "$STORE" -o /tmp/cf-dedup-demo/v1b.cfidx fixtures/gen/large-64m.bin
# stderr: new=0, reused=N — .cnk count unchanged

$BIN make --store "$STORE" -o /tmp/cf-dedup-demo/v2.cfidx fixtures/gen/large-64m-mut.bin
# stderr: new>0, reused>0 — .cnk count grows modestly
$BIN verify --store "$STORE" /tmp/cf-dedup-demo/v2.cfidx
```

## CI / always-on tests

Integration tests in `crates/chunkforge-cli/tests/cli_integration.rs` assert:

- identical remake does not increase `.cnk` count and reports `new=0`
- mid-file mutation with small `--chunk-size` shows both new and reused chunks

An optional `#[ignore]` large-file test can be run with:

```bash
CHUNKFORGE_GEN_MIB=8 cargo test -p chunkforge-cli --test cli_integration large_file_dedup -- --ignored --nocapture
```
