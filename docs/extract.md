# `chunkforge extract`

Materialize a directory tree from a **`.cfdir`** listing plus a chunk source
(`--store` / `--source`). Parents are created as needed; file modes are restored
on Unix when recorded. Empty matching `Dir` entries create directories.

Phase 9 adds opt-in **`--skip-unchanged`** and **`--dry-run`**; Phase 11 adds
opt-in **`--skip-trust-mtime`** (requires `--skip-unchanged`) and
**`--format text|json`** (default **text** ≡ 1.0.0). Phase 13 adds opt-in
**`--path`** / **`--path-from`** / **`--exclude`** / **`--exclude-from`** (default: full tree ≡ **1.9.0** / **1.2.0**). Without those
flags, behaviour matches **1.9.0** / **1.2.0** / **1.0.0** / **0.8.0** (full write /
conflict-fail; `--force` overwrites existing regular files). See also
[archive.md](archive.md), [dir-format.md](dir-format.md),
[http-retry.md](http-retry.md).

## Usage

```bash
chunkforge extract \
  --store <cas> | --source <PATH|URL> \
  [--fallback <PATH|URL>]... \
  -o <out-dir> \
  [--path P]... [--path-from FILE]... [--exclude PAT]... [--exclude-from FILE]... \
  [--force] \
  [--skip-unchanged] \
  [--skip-trust-mtime] \
  [--dry-run] \
  [--format text|json] \
  [--progress] \
  [--jobs N] \
  [--http-retries N] \
  [--cache <dir>] \
  [--cache-max-bytes N|1M|…] \
  archive.cfdir
```

| Flag | Meaning |
|---|---|
| `--store` / `--source` | Chunk origin (local CAS path, `file://`, or `http(s)://`) — same as `cat` / `verify` |
| `--fallback` | Repeatable extra origin tried **only on Missing** (CLI order). Zero times ≡ **1.5** single origin. **≠ cache ≠ sync ≠ prune**. Outer Cache wraps the whole Fallback chain when `--cache` is set |
| `-o` / `--output` | Output directory (created if missing on a real extract; **not** created under `--dry-run`) |
| `--path P` | Repeatable include prefix (OR). With any `--path`, listing paths must match at least one (`==` or `P/…`) before excludes. Omit all ⇒ include-all (≡ **1.2.0** full tree) |
| `--exclude PAT` | Repeatable exclude: exact, trailing-`/` directory prefix, or single edge `*` (`*.o`, `temp*`). Illegal middle `*` / `**` → clear error. **Not** prune |
| `--exclude-from FILE` | Repeatable UTF-8 file: one pattern per line (blank / `#` skipped, trim). Merged with `--exclude` into one `PathFilter`. Missing file or illegal line → clear non-zero. **Not** prune |
| `--path-from FILE` | Phase 20 opt-in: UTF-8 one include prefix per line (≡ `--path`); blank/`#`/trim; merged with `--path` (OR). Missing file → non-zero. **`path-from` ≠ prune ≠ gc-path ≠ sync ≠ pack** |
| `--force` | Overwrite existing **regular files**. Type mismatches (file↔directory) still fail. Default **off** ≡ 0.8.0 conflict-fail |
| `--skip-unchanged` | Opt-in: if dest exists as a regular file, **size** matches the listing, and **content BLAKE3 ≡ `blob_blake3`**, skip chunk fetch and write (mode/mtime untouched). Default **off** ≡ 0.8.0 / 1.0.0 |
| `--skip-trust-mtime` | Requires `--skip-unchanged`. When size **and** dest `mtime_secs` both match the listing File entry, skip **without** content BLAKE3 (fast path). Default **off** ≡ 1.0.0 content path. **WARNING:** forged / clock-drifted / `cp -p`-preserved mtimes can miss content changes — prefer the content fingerprint unless you accept that risk |
| `--dry-run` | Plan only: create/modify **no** paths under `-o` (output root included); never fetch chunks. Text: stderr `would_*` counters over the **filtered** set |
| `--format` | `text` (default ≡ **1.0.0** stderr summary) or `json` (one object on **stdout**; no duplicate stderr summary). Exit codes are format-independent |
| `--progress` | Opt-in stderr `progress: op=extract done=N/TOTAL` per filtered File (default **off** ≡ 1.6). Orthogonal to `--format json` / `--jobs`. **≠** tracing / otel |
| `--jobs` / `--http-retries` / `--cache` / `--cache-max-bytes` / templates / SigV4 | Same as other read-side commands; `--cache-max-bytes` requires `--cache` (soft refuse-fill; accepts `1M`/`64Mi`/…; **≠ LRU ≠ trim ≠ GC ≠ sync**); skipped files issue **zero** chunk `get` |

Matching is orthogonal to `--force` / `--skip-*` / `--dry-run` / `--format` /
`--jobs`: those flags never change which listing paths are selected.

### Summaries (`--format text`, stderr ≡ 1.0.0)

| Mode | Line shape |
|---|---|
| No `--skip-unchanged` / no `--dry-run` | `extract: wrote <out> (N files, D dirs)` — **0.8.0 / 1.0.0-compatible** |
| With `--skip-unchanged` (write path) | `extract: <out> skipped=S wrote=W dirs=D` |
| With `--dry-run` | `extract: dry-run: would_skip=… would_write=… would_dirs=… would_fail=…` |

### `--format json` (stdout; Phase 11 M2)

One JSON **object** on stdout on success. Failures still go through anyhow (non-zero exit; no success JSON). Exit code is **independent** of `--format` (dry-run listing valid → **0** even if `would_fail>0`).

| Mode | Fields |
|---|---|
| Write path | `{"ok":true,"dry_run":false,"skipped":S,"wrote":W,"dirs":D}` — always includes `skipped`/`wrote`/`dirs` (`skipped=0` when `--skip-unchanged` is off) |
| `--dry-run` | `{"ok":true,"dry_run":true,"would_skip":…,"would_write":…,"would_dirs":…,"would_fail":…}` |

## `--skip-unchanged` / `--force` / `--dry-run` overlap

| Situation | No skip (≡ 0.8.0) | `--skip-unchanged` | `--skip-unchanged` + `--force` | `--dry-run` (+ optional skip) |
|---|---|---|---|---|
| Dest missing | Write | Write | Write | `would_write` (no write) |
| Dest exists, size+BLAKE3 match | Fail (conflict) unless `--force` → **rewrite** | **Skip** (no get / no write) | **Skip** — **match beats force** | `would_skip` if skip; else `would_fail` / `would_write` with force |
| Dest exists, content mismatch | Fail unless `--force` → rewrite | Fail unless `--force` → rewrite | Rewrite (`wrote`) | `would_fail` without force; `would_write` with force |
| Type mismatch (file↔dir) | Fail | Fail | Fail (`--force` does not replace types) | `would_fail` |
| Matching skip path | n/a | **Zero** `ChunkSource::get` for that file's chunks | same | With skip: local stat/hash only; **no** chunk get. Without skip: no store/source open; all files `would_write` |

**Match criteria:** size fast-reject; with `--skip-trust-mtime`, size **and**
mtime_secs hit → **Unchanged** (no content read); otherwise streaming content
BLAKE3 ≡ listing `blob_blake3` (≡ 1.0.0). Default (no `--skip-trust-mtime`)
does **not** trust mtime. Symmetrical to `archive --seed-trust-mtime`.

**Dry-run exit:** **0** when the listing is valid (even if `would_fail>0`);
invalid listing → non-zero.

## Explicitly **no prune** (`path` ≠ prune ≠ sync; `fallback` ≠ cache ≠ sync)

`extract` is **one-way materialize**, not sync. Path filtering only **writes
less**; it never removes destination paths. Read-path **`--fallback`** is
Missing-only multi-origin failover — it does **not** fill a cache, does **not**
write back to primary, and is **not** bidirectional sync (see [mount.md](mount.md)).

| Non-goal | Detail |
|---|---|
| ❌ **Prune / `--delete`** | There is **no** `--delete` flag. Extra files under `-o` that are **not** in the listing stay. Listing paths that exist but are **filtered out** by `--path`/`--exclude` are also **not** deleted (and not written) |
| ❌ Bidirectional sync / watch | Use explicit `archive` / `extract` / `diff`. Path scope is **not** sync; `--fallback` is **not** sync |
| ❌ `--fallback` as cache / write-back | Fallback never writes origins; use `--cache` (+ optional `--cache-max-bytes`) to fill a local cache |
| ❌ Rewrite matching files under `--force` | Match + `--skip-unchanged` always skips; no `--force-rewrite` |

**`--path` / `--exclude` behaviour (Phase 13):**

1. The listing file is read in full (bytes unchanged).
2. Only **matching File** entries are materialized; necessary parent directories
   are created so those files can be written (same `create_dir_all` path as
   before). Unmatched siblings are **not** given empty shells.
3. Dry-run / JSON `would_*` / write counts reflect the **filtered** set only.
4. Default (no path/exclude flags) ≡ **1.2.0** full-tree extract.

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

# Opt-in mtime fast path (requires --skip-unchanged; WARNING: clock / cp -p risk)
chunkforge extract --store ./store -o /tmp/out \
  --skip-unchanged --skip-trust-mtime --force release.cfdir

# Plan only (no writes; with skip → local hash only, no chunk GET)
chunkforge extract --store ./store -o /tmp/out \
  --skip-unchanged --dry-run release.cfdir
# stderr: extract: dry-run: would_skip=… would_write=… would_dirs=… would_fail=…

# Ops JSON (stdout; no duplicate stderr summary)
chunkforge extract --store ./store -o /tmp/out --format json release.cfdir
# → {"ok":true,"dry_run":false,"skipped":0,"wrote":W,"dirs":D}
chunkforge extract --store ./store -o /tmp/out \
  --skip-unchanged --dry-run --format json release.cfdir
# → {"ok":true,"dry_run":true,"would_skip":…,"would_write":…,"would_dirs":…,"would_fail":…}

# HTTP source + retries (skipped files still issue zero GET)
chunkforge extract --source http://127.0.0.1:8765 \
  --http-retries 2 --skip-unchanged --force -o /tmp/out release.cfdir

# Phase 13: only materialize a subtree (extra / filtered-out dest paths stay)
chunkforge extract --store ./store -o /tmp/out \
  --path packages/foo --force release.cfdir
# packages/foo/… written; other listing paths skipped; pre-existing extras kept

chunkforge extract --store ./store -o /tmp/out \
  --exclude '*.o' --exclude junk/ --dry-run --format json release.cfdir
# → would_* counts exclude filtered paths; no writes; no deletes
```



## `--progress` / disk zstd note (Phase 17 / 1.7.0)

`extract --progress` (default **off** ≡ 1.6) writes
`progress: op=extract …` to **stderr** only; JSON field shapes in
[ops-json.md](ops-json.md) are unchanged. Extract does **not** take
`--compression` (read path). Local stores created with
`make`/`archive --compression zstd` still serve **plaintext** via `get`;
HTTP bodies stay plaintext. Disk zstd ≠ wire compression ≠ pack — see
[remote-layout.md](remote-layout.md) / [archive.md](archive.md).

## Demo

```bash
bash scripts/demo_extract_skip.sh
bash scripts/demo_path_filter.sh  # Phase 13 path/exclude + non-prune
bash scripts/demo_fallback_bytes_suffix.sh  # Phase 16 fallback + suffix + bytes_plaintext
bash scripts/demo_zstd_progress.sh  # Phase 17: zstd create + archive/extract/make --progress
# first extract → --skip-unchanged (skipped=all, zero HTTP GET) →
# change one file → skipped=N-1 wrote=1 → optional dry-run glance
```

## Related

- Archive / seed: [archive.md](archive.md)
- Listing bytes: [dir-format.md](dir-format.md)
- Diff (not sync): [diff.md](diff.md)
- HTTP retries: [http-retry.md](http-retry.md)
- Ops JSON: [ops-json.md](ops-json.md)
