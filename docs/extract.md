# `chunkforge extract`

Materialize a directory tree from a **`.cfdir`** listing plus a chunk source
(`--store` / `--source`). Parents are created as needed; file modes are restored
on Unix when recorded. Empty `Dir` entries create directories.

Phase 9 adds opt-in **`--skip-unchanged`** and **`--dry-run`**. Without those
flags, behaviour matches **0.8.0** (full write / conflict-fail; `--force`
overwrites existing regular files). See also [archive.md](archive.md),
[dir-format.md](dir-format.md), [http-retry.md](http-retry.md).

## Usage

```bash
chunkforge extract \
  --store <cas> | --source <PATH|URL> \
  -o <out-dir> \
  [--force] \
  [--skip-unchanged] \
  [--dry-run] \
  [--jobs N] \
  [--http-retries N] \
  [--cache <dir>] \
  archive.cfdir
```

| Flag | Meaning |
|---|---|
| `--store` / `--source` | Chunk origin (local CAS path, `file://`, or `http(s)://`) — same as `cat` / `verify` |
| `-o` / `--output` | Output directory (created if missing on a real extract; **not** created under `--dry-run`) |
| `--force` | Overwrite existing **regular files**. Type mismatches (file↔directory) still fail. Default **off** ≡ 0.8.0 conflict-fail |
| `--skip-unchanged` | Opt-in: if dest exists as a regular file, **size** matches the listing, and **content BLAKE3 ≡ `blob_blake3`**, skip chunk fetch and write (mode/mtime untouched). Default **off** ≡ 0.8.0 |
| `--dry-run` | Plan only: create/modify **no** paths under `-o` (output root included); never fetch chunks. Stderr `would_*` counters |
| `--jobs` / `--http-retries` / `--cache` / templates / SigV4 | Same as other read-side commands; skipped files issue **zero** chunk `get` |

### Summaries (stderr)

| Mode | Line shape |
|---|---|
| No `--skip-unchanged` / no `--dry-run` | `extract: wrote <out> (N files, D dirs)` — **0.8.0-compatible** |
| With `--skip-unchanged` (write path) | `extract: <out> skipped=S wrote=W dirs=D` |
| With `--dry-run` | `extract: dry-run: would_skip=… would_write=… would_dirs=… would_fail=…` |

## `--skip-unchanged` / `--force` / `--dry-run` overlap

| Situation | No skip (≡ 0.8.0) | `--skip-unchanged` | `--skip-unchanged` + `--force` | `--dry-run` (+ optional skip) |
|---|---|---|---|---|
| Dest missing | Write | Write | Write | `would_write` (no write) |
| Dest exists, size+BLAKE3 match | Fail (conflict) unless `--force` → **rewrite** | **Skip** (no get / no write) | **Skip** — **match beats force** | `would_skip` if skip; else `would_fail` / `would_write` with force |
| Dest exists, content mismatch | Fail unless `--force` → rewrite | Fail unless `--force` → rewrite | Rewrite (`wrote`) | `would_fail` without force; `would_write` with force |
| Type mismatch (file↔dir) | Fail | Fail | Fail (`--force` does not replace types) | `would_fail` |
| Matching skip path | n/a | **Zero** `ChunkSource::get` for that file's chunks | same | With skip: local stat/hash only; **no** chunk get. Without skip: no store/source open; all files `would_write` |

**Match criteria (P0):** size fast-reject, then streaming content BLAKE3 ≡ listing
`blob_blake3`. mtime is **not** trusted (no `--skip-trust-mtime` in this release).

**Dry-run exit:** **0** when the listing is valid (even if `would_fail>0`);
invalid listing → non-zero.

## Explicitly **no prune**

`extract` is **one-way materialize**, not sync:

| Non-goal | Detail |
|---|---|
| ❌ **Prune / `--delete`** | Extra files under `-o` that are **not** in the listing are **left alone**. There is no delete-extras / rsync-`--delete` mode |
| ❌ Bidirectional sync / watch | Use explicit `archive` / `extract` / `diff` |
| ❌ Rewrite matching files under `--force` | Match + `--skip-unchanged` always skips; no `--force-rewrite` |

If you need a clean tree, remove or choose a fresh `-o` yourself (or `diff`
first). Listing files still never ride along with `push`.

## Examples

```bash
# First materialize (0.8.0 path)
chunkforge extract --store ./store -o /tmp/out release.cfdir
# stderr: extract: wrote /tmp/out (N files, …)

# In-place refresh: skip unchanged; rewrite mismatches
chunkforge extract --store ./store -o /tmp/out \
  --skip-unchanged --force release.cfdir
# stderr: extract: /tmp/out skipped=S wrote=W dirs=D

# Plan only (no writes; with skip → local hash only, no chunk GET)
chunkforge extract --store ./store -o /tmp/out \
  --skip-unchanged --dry-run release.cfdir
# stderr: extract: dry-run: would_skip=… would_write=… would_dirs=… would_fail=…

# HTTP source + retries (skipped files still issue zero GET)
chunkforge extract --source http://127.0.0.1:8765 \
  --http-retries 2 --skip-unchanged --force -o /tmp/out release.cfdir
```

## Demo

```bash
bash scripts/demo_extract_skip.sh
# first extract → --skip-unchanged (skipped=all, zero HTTP GET) →
# change one file → skipped=N-1 wrote=1 → optional dry-run glance
```

## Related

- Archive / seed: [archive.md](archive.md)
- Listing bytes: [dir-format.md](dir-format.md)
- Diff (not sync): [diff.md](diff.md)
- HTTP retries: [http-retry.md](http-retry.md)
