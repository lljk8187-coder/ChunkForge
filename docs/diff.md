# `chunkforge diff`

Compare two directory listings (`.cfdir`), or a live source tree against a
listing (`--tree`). **Read-only**: never writes a local store or a `.cfdir`.

Phase 7 (+ Phase 8 `--format json`). See also [`doctor-gc.md`](doctor-gc.md)
for presence / GC tooling, [`dir-format.md`](dir-format.md) for `.cfdir`
layout, [`http-retry.md`](http-retry.md) for HTTP retries / error classes,
and [`sigv4.md`](sigv4.md) for optional `--aws-sigv4`.

## Listing ↔ listing

```bash
chunkforge diff [--format text|json] [--max-paths N] left.cfdir right.cfdir
```

- Both arguments must be **`.cfdir`** files (`.cfidx` / bad magic → error).
- Bare directories without `--tree` are rejected (use `--tree`, below).
- Comparison is by relative **File** path (Dir-only entries are ignored).
- Orientation: **left** = baseline, **right** = comparison.

| Category | Meaning |
|---|---|
| **added** | Path in right, not in left |
| **removed** | Path in left, not in right |
| **changed** | Same path; content differs (`blob_blake3` and/or size) |
| **meta_changed** | Same path, same `blob_blake3`, but `mode` and/or `mtime_secs` differ (never also in `changed`) |

Chunk-set stats come from the unique chunk ids referenced by File entries on
each side (`all_chunk_ids`):

| Field | Meaning |
|---|---|
| `chunks_shared` | Unique ids present in both |
| `chunks_only_left` | Unique ids only in left |
| `chunks_only_right` | Unique ids only in right |

## Tree ↔ listing (`--tree`)

```bash
chunkforge diff --tree <src-dir> [--format text|json] [--max-paths N] <listing.cfdir>
# or:
chunkforge diff [--format text|json] [--max-paths N] --tree <src-dir> <listing.cfdir>
```

- Exactly **one** source directory (`--tree <src-dir>`) and **one** listing
  `.cfdir`. Two trees are **not** supported. `.cfidx` is rejected.
- Builds an **ephemeral in-memory `DirArchive`** from regular files under
  `src-dir` (no store put, no `.cfdir` write, no `--store` flag).
- Symlinks and special files (fifo/socket/device) are skipped with a `diff:`
  warn on stderr — same policy as `archive` (not recorded / not followed).
- For each regular file: `size`, `mode`, `mtime_secs`, stream BLAKE3 →
  `blob_blake3`.
- **Chunk tables on the tree side:**
  - If the path exists in the listing **and** `blob_blake3` matches → **copy**
    the listing’s File entry (chunk table + prefer listing meta) so an
    identical tree↔listing yields `chunks_shared` ≈ full and
    `chunks_only_*=0`.
  - If content differs or the path is new on the tree → empty chunk list for
    that tree entry (path-level diff still reports correctly; chunk-only_* may
    reflect that the tree side has no table for that file).
- Comparison uses `diff_dir_archives(tree, listing)` with **left = tree**,
  **right = listing**. Same summary line and exit codes as listing↔listing.

## Output

### `--format text` (default ≡ 0.7.0)

Omitting `--format` or passing `--format text` preserves the 0.7.0 text path:
non-empty path categories on **stdout** (sorted paths), then the summary line:

```text
added:
  new.txt
changed:
  a.txt
diff: added=1 removed=0 changed=1 meta_changed=0 chunks_shared=2 chunks_only_left=0 chunks_only_right=1
```

The last line is the stable, machine-parseable summary (exact field names):

```text
diff: added=… removed=… changed=… meta_changed=… chunks_shared=… chunks_only_left=… chunks_only_right=…
```

`--max-paths N` truncates each **text** category listing after N paths (prints
`... and K more`); **summary counts stay full**.

### `--format json` (Phase 8)

One JSON object on stdout (single line). Field names are stable and match the
text summary semantics:

| Field | Type | Meaning |
|---|---|---|
| `added` | string array | Paths in right, not in left (sorted) |
| `removed` | string array | Paths in left, not in right (sorted) |
| `changed` | string array | Same path; content differs |
| `meta_changed` | string array | Same path; same blake3; mode/mtime differ |
| `chunks_shared` | number | Unique chunk ids in both |
| `chunks_only_left` | number | Unique chunk ids only in left |
| `chunks_only_right` | number | Unique chunk ids only in right |

Example:

```bash
chunkforge diff --format json v1.cfdir v2.cfdir || true
```

```json
{"added":["new.txt"],"removed":[],"changed":["a.txt"],"meta_changed":[],"chunks_shared":2,"chunks_only_left":0,"chunks_only_right":1}
```

JSON always emits **full** path arrays (`--max-paths` does not truncate JSON).
Array lengths equal the corresponding `added=` / `removed=` / … counts in the
text summary for the same two inputs.

## Exit codes

| Code | When |
|---|---|
| **0** | No path differences and no `chunks_only_left` / `chunks_only_right` |
| **1** | Any added / removed / changed / meta_changed, or either chunks_only_* > 0 (like `diff(1)`; no `error:` prefix). **Independent of `--format`.** |
| **non-zero (≠1)** | Usage / decode / I/O errors (clap / anyhow) |

## Responsibility table

| Tool | Role | Needs listing? | Touches store objects? |
|---|---|---|---|
| **`verify`** | Listing structure + referenced chunk presence/hashes + per-file `blob_blake3` | Yes (`.cfidx` / `.cfdir`) | Read via `--store` / `--source` |
| **`doctor`** | Presence of referenced chunks (`has`, optional `--deep` = `get`) | Yes | Read presence only |
| **`gc`** | Delete (or dry-run) **unreferenced** loose `.cnk` under a local `--store` | Yes (reference set) | May delete unreferenced only (`--apply`) |
| **`diff`** | Path / meta / chunk-**set** comparison of two listings, or tree↔listing | Yes (and optional live tree) | **Never** — report only |
| **`store scrub`** | Local CAS **bitrot** check: traverse loose chunks and re-BLAKE3 (`get_verify`); report `ok=` / `corrupt=` / `unreadable=` | **No** (optional refs later) | Read-only rehash; does **not** delete |

`diff` is **not** sync, merge, or verify. `store scrub` is the
“deep integrity / bitrot” companion to `doctor` (presence) and `gc`
(unreferenced reclaim) — scrub does **not** require a listing and does **not**
belong inside `doctor --deep`. See [`doctor-gc.md`](doctor-gc.md).
