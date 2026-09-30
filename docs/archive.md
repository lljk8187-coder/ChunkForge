# Directory archive (`archive` / `extract` / tree `verify` / push)

Phase 5 multi-file workflow built on [`.cfdir` v1](dir-format.md). Single-blob
[`.cfidx`](index-format.md) commands (`make` / `cat`) are unchanged.

## Commands

| Command | Role |
|---|---|
| `chunkforge archive --store <cas> -o out.cfdir [--seed prior.cfdir] [--seed-trust-mtime] [--dry-run] [--jobs N] [--path P]… [--exclude PAT]… [--format text\|json] <src-dir>` | Recursively chunk regular files into the local CAS; write a `.cfdir` listing (`--seed` / `--seed-trust-mtime` / `--dry-run` / `--jobs` as before; `--path`/`--exclude`: filter which files are chunked+listed, default full tree ≡ 1.2.0; `--format`: `text` default ≡ 1.2.0 stderr summary, `json` one object on stdout) |
| `chunkforge extract --store\|--source … archive.cfdir -o <out-dir> [--force]` | Materialize the tree (parents created; existing paths → non-zero unless `--force`) |
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
./scripts/demo_archive.sh      # Phase 5 tree / push / mount smoke
./scripts/demo_seed.sh         # Phase 6 seed: change one file → reuse stats → verify / extract / optional pull
./scripts/demo_path_filter.sh  # Phase 13: exclude → archive json → extract --path → pull --path
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
2. **Optional `--seed-trust-mtime`** (default **off**): if size **and**
   `mtime_secs` both equal the prior entry → **Reuse** without content BLAKE3.
3. **Content fingerprint** (default path, or when trust is off / mtime differs):
   the file is streamed and compared to `blob_blake3`. A match reuses the prior
   chunk table (and recorded `mode` / `size` / `mtime_secs` / `blob_blake3`) and
   skips FastCDC / store `put` for that file.

Without `--seed-trust-mtime`, mtime is **never** the sole criterion — content
BLAKE3 decides reuse (≡ 0.6.0).

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

### `--seed-trust-mtime` (opt-in)

Requires `--seed`. When set, a path that matches the prior on **both size and
`mtime_secs`** is reused **without** hashing file contents.

> **Warning:** trusting mtime can **miss content changes** if mtime is forged,
> truncated (coarse FS precision), or preserved across edits (`cp -p`, some
> backup/restore tools, network filesystems). Prefer the default content-BLAKE3
> path unless you accept that risk for large trees where re-reading every file
> is too expensive. Missing chunks after a trust-based Reuse still force rechunk
> + `seed_missing_chunks=` (same as 0.6.0).

```bash
chunkforge archive --store ./store -o v2.cfdir \
  --seed v1.cfdir --seed-trust-mtime ./src
```

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

## Path filter (`--path` / `--exclude`)

Phase 13 opt-in. Repeatable `--path P` and `--exclude PAT` build a
`PathFilter` (same rules as `chunkforge-index::PathFilter`):

- **`--path P`**: keep iff `path == P` or `path` is under `P/` (OR across flags).
- **`--exclude PAT`**: exact match; trailing `/` → directory prefix; single `*`
  only at start (`*.o`) or end (`temp*`). Illegal middle `*` / `**` → non-zero
  with a clear error.
- If any `--path` is given, a candidate must hit an include **before** excludes.
- **No** `--path` / `--exclude` ⇒ full tree (≡ **1.2.0**).
- Orthogonal to `--seed` / `--seed-trust-mtime` / `--dry-run` / `--jobs` /
  `--format`.

Filtered-out regular files are **not** chunked and **do not** appear in the
written `.cfdir`; they increment `excluded` in the text summary / JSON.

**`path` ≠ prune ≠ sync:** archive path/exclude only **shrinks the listing**
(fewer entries written). It does not delete anything on disk, does not prune
an extract destination, and is not bidirectional sync. See
[extract.md](extract.md) (non-prune materialize) and [pull.md](pull.md)
(subset CAS fill).

### Walk order vs symlink / special

1. **Type skip first**: symlinks and special files (fifo/socket/device) are
   skipped with a stderr warning and counted in `skipped_symlinks` /
   `skipped_special` (not followed, not recorded).
2. **Then path filter**: remaining regular-file candidates are checked with
   `PathFilter::allows`; rejects increment `excluded` and are omitted from the
   listing.

## `--format text|json`

Default **`text`** ≡ 1.2.0: human summary on stderr (`archive: wrote …` /
`archive: dry-run: …`), including `excluded=N`.

**`--format json`**: one JSON object on **stdout** (no duplicate text summary).
Exit codes are format-independent.

| Field | Meaning |
|---|---|
| `ok` | `true` on success |
| `dry_run` | whether `--dry-run` was set |
| `files` / `dirs` / `chunks` | listing file count, Dir entries (usually 0 — empty dirs omitted), total chunk refs |
| `written` / `reused` | chunk put outcomes (**normal write only**) |
| `would_write` / `would_reuse` | same accounting under **`--dry-run` only** (not both with written/reused) |
| `seed_reused_files` / `rechunked_files` | seed file-level counters (0 when no `--seed`) |
| `skipped_symlinks` / `skipped_special` | type-skip counts |
| `excluded` | regular files rejected by `--path`/`--exclude` (0 when no filter) |

```bash
chunkforge archive --store ./store -o app.cfdir \
  --exclude .git/ --exclude '*.o' --format json ./src
# stdout: {"ok":true,"dry_run":false,"files":…,"excluded":…,…}
```

## Archive policy (P0)

- **Regular files** only are recorded (optional empty `Dir` entries omitted).
- **Symlinks**: skipped with a stderr warning (not followed, not recorded) —
  **before** `--path`/`--exclude`.
- **fifo / socket / device**: skipped with a stderr warning — **before** path
  filter.
- Chunk params default to FastCDC 16KiB / 64KiB / 256KiB; override with
  `--chunk-size min:avg:max`.

## Extract conflicts / `--force`

Without `--force` (≡ 0.6.0): if any destination path already exists, `extract`
exits non-zero — remove or choose a fresh `-o` directory.

With `--force`:

- **Existing regular files** are truncated and overwritten.
- **Type mismatches** still fail with a clear error: a directory where a file is
  expected (or a file where a directory is expected) is **not** replaced;
  `--force` does not `rm -rf` directories.
- The `-o` output root itself, if it already exists as a **file**, is never
  overwritten (even with `--force`).

## Related

- Format bytes: [dir-format.md](dir-format.md)
- FUSE directory mount: [mount.md](mount.md)
- Push / doctor / gc accept `.cfdir`: [push.md](push.md), [doctor-gc.md](doctor-gc.md)
- Ops JSON matrix (incl. archive): [ops-json.md](ops-json.md)
