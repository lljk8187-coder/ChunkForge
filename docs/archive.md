# Directory archive (`archive` / `extract` / tree `verify` / push)

Phase 5 multi-file workflow built on [`.cfdir` v1](dir-format.md). Single-blob
[`.cfidx`](index-format.md) commands (`make` / `cat`) are unchanged.

## Commands

| Command | Role |
|---|---|
| `chunkforge archive --store <cas> -o out.cfdir [--seed prior.cfdir] [--dry-run] [--jobs N] <src-dir>` | Recursively chunk regular files into the local CAS; write a `.cfdir` listing (`--seed`: reuse unchanged files' chunk tables; `--dry-run`: stats only; `--jobs`: per-file parallel chunking, default 1) |
| `chunkforge extract --store\|--source … archive.cfdir -o <out-dir>` | Materialize the tree (parents created; existing paths → non-zero) |
| `chunkforge verify --store\|--source … archive.cfdir` | Magic-dispatch: tree structure + per-file `blob_blake3` |
| `chunkforge mount --store\|--source … archive.cfdir <mnt>` | Read-only FUSE directory tree (see [mount.md](mount.md)) |
| `chunkforge push --store <cas> --dest http(s)://… archive.cfdir` | Upload **chunks only** referenced by the `.cfdir` |

`--jobs N` on archive / extract / verify / push / doctor / pull defaults to **1** (serial). For `archive`, jobs parallelize **per file** (seed map is read-only; store puts are atomic).

## Typical flow

```bash
cargo build -p chunkforge-cli

mkdir -p /tmp/cf-arch/src/sub
echo 'hello-tree' > /tmp/cf-arch/src/a.txt
cp fixtures/hello.txt /tmp/cf-arch/src/sub/b.txt
cp /tmp/cf-arch/src/a.txt /tmp/cf-arch/src/a-copy.txt   # cross-file dedup

./target/debug/chunkforge archive \
  --store /tmp/cf-arch/store -o /tmp/cf-arch/release.cfdir /tmp/cf-arch/src

./target/debug/chunkforge verify --store /tmp/cf-arch/store /tmp/cf-arch/release.cfdir
./target/debug/chunkforge extract --store /tmp/cf-arch/store \
  /tmp/cf-arch/release.cfdir -o /tmp/cf-arch/out
diff -qr /tmp/cf-arch/src /tmp/cf-arch/out
```

### Push chunks (listing stays local)

`push` never uploads the `.cfdir` / `.cfidx` file itself — only plaintext chunk
objects under the same layout as `verify --source` (see [push.md](push.md)).

```bash
# terminal 1
python3 scripts/put_stub.py --root /tmp/cf-arch/mirror --port 8766

# terminal 2
./target/debug/chunkforge push \
  --store /tmp/cf-arch/store \
  --dest http://127.0.0.1:8766 \
  --verify \
  /tmp/cf-arch/release.cfdir
```

### Smoke scripts

```bash
./scripts/demo_archive.sh   # Phase 5 tree / push / mount smoke
./scripts/demo_seed.sh      # Phase 6 seed: change one file → reuse stats → verify / extract / optional pull
```

`demo_archive.sh` covers archive → verify → extract → diff, optional FUSE mount
(skipped if fuse unavailable), push via `put_stub.py` + `verify --source`, and a
`.cfidx` make/verify regression path.

`demo_seed.sh` covers first archive → edit one file → `archive --seed` (expect
`seed_reused_files` / `rechunked_files=1`) → verify → extract+diff → dry-run full
reuse → optional `pull` via local `put_stub`.

## `--dry-run`

`archive --dry-run` walks the tree, runs FastCDC, and prints
`files` / `chunks` / `would_write` / `would_reuse` without putting chunks into
`--store` or writing the `.cfdir`. If `--store` already exists, `would_reuse`
counts ids already present (plus in-run cross-file dedup); a missing store is
not created.

```bash
chunkforge archive --store ./store -o release.cfdir --dry-run ./src
# stderr: archive: dry-run: N files, M chunks (would_write=…, would_reuse=…); no store/.cfdir written …
```

Combined with `--seed`, dry-run also prints `would_seed_reuse=` / `would_rechunk=`
(see [Seed](#seed---seed-priorcfdir) below) and still writes nothing.

## Seed (`--seed prior.cfdir`)

Incremental archive against a **prior `.cfdir` only** (not a `.cfidx`). For each
source file whose relative path exists in the prior listing:

1. **Size fast-reject**: if the on-disk size differs from the prior entry, the
   file is rechunked (no full-file hash).
2. **Content fingerprint**: otherwise the file is streamed and compared to
   `blob_blake3`. A match reuses the prior chunk table (and recorded
   `mode` / `size` / `mtime_secs` / `blob_blake3`) and skips FastCDC / store
   `put` for that file.
3. **mtime is not the sole criterion** — content BLAKE3 decides reuse (mtime
   alone never marks a file unchanged).

Reuse copies the prior chunk table into the **new** listing. If any reused chunk
id is missing from `--store`, that file is forced to rechunk and a stderr warning
increments `seed_missing_chunks=`. Files absent from the prior, or whose content
changed, are chunked as usual.

The output is always a **full `.cfdir` v1** (self-contained listing) — never a
delta against the prior. Consumers (`verify` / `extract` / `mount` / `push`) do
not need the seed file.

Write-path stderr (no `--dry-run`) reports `seed_reused_files=` /
`rechunked_files=` (plus the usual `new=` / `reused=` chunk counters).

Optional `--jobs N` (default **1**) parallelizes per-file work; the seed map is
read-only across workers and store puts remain content-addressed / race-safe.

### Dry-run × seed

With both `--dry-run` and `--seed`, ChunkForge previews reuse without writing
store chunks or the output `.cfdir`. File-level counters use dry-run vocabulary:

`would_seed_reuse=` / `would_rechunk=` (alongside chunk-level
`would_write=` / `would_reuse=`).

```bash
# First archive
chunkforge archive --store ./store -o v1.cfdir ./src

# Preview a second pass against the same tree (expect full seed reuse)
chunkforge archive --store ./store -o v2.cfdir --seed v1.cfdir --dry-run ./src
# stderr: … would_seed_reuse=N, would_rechunk=0 …; no store/.cfdir written …
```

```bash
# After editing one file, dry-run should show would_rechunk=1
echo 'changed' > ./src/a.txt
chunkforge archive --store ./store -o v2.cfdir --seed v1.cfdir --dry-run ./src
# stderr: … would_seed_reuse=…, would_rechunk=1 …; no store/.cfdir written …
```

## Archive policy (P0)


- **Regular files** only are recorded (optional empty `Dir` entries omitted).
- **Symlinks**: skipped with a stderr warning (not followed, not recorded).
- **fifo / socket / device**: skipped with a stderr warning.
- Chunk params default to FastCDC 16KiB / 64KiB / 256KiB; override with
  `--chunk-size min:avg:max`.

## Extract conflicts

If any destination path already exists, `extract` exits non-zero. There is no
`--force` yet — remove or choose a fresh `-o` directory.

## Related

- Format bytes: [dir-format.md](dir-format.md)
- FUSE directory mount: [mount.md](mount.md)
- Push / doctor / gc accept `.cfdir`: [push.md](push.md), [doctor-gc.md](doctor-gc.md)
