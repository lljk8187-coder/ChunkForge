# `chunkforge diff`

Compare two directory listings (`.cfdir`), or a live source tree against a
listing (`--tree`). **Read-only**: never writes a local store or a `.cfdir`.

Phase 7 (+ Phase 8 `--format json`; Phase 16 P1 `--path` / `--exclude` /
`--exclude-from`; Phase 19 `--progress`; Phase 20 `--path-from`; Phase22
listing↔listing Symlink compare; Phase23 **`diff --tree --symlinks
skip|record`**). See also [`doctor-gc.md`](doctor-gc.md) for presence / GC
tooling, [`dir-format.md`](dir-format.md) for `.cfdir` layout,
[`http-retry.md`](http-retry.md) for HTTP retries / error classes, and
[`sigv4.md`](sigv4.md) for optional `--aws-sigv4`.

## Path filter (`--path` / `--path-from` / `--exclude` / `--exclude-from`)

Phase 16 P1 (O1) + Phase 20 **`--path-from`**. Optional repeatable
**`--path`** / **`--path-from`** / **`--exclude`** / **`--exclude-from`**
narrow **both** sides' File/Dir/**Symlink** entry sets with the same
[`PathFilter`](../crates/chunkforge-index/src/path_filter.rs) used by
`archive` / `extract` / `push` / `pull` / `doctor` / `verify` **before**
compare. `--path-from` loads include prefixes from a UTF-8 file (discipline ≡
`--exclude-from`; merged OR with `--path`). Symlink paths are filtered like
Files (same as `archive --symlinks record` / `filter_dir_archive`).

- Default (no path/exclude flags) ≡ **1.9** / **1.5** full-listing / full-tree diff.
- JSON field **names** unchanged; arrays / chunk counts reflect the narrowed set.
- **`path-from` ≠ sync ≠ prune ≠ `--delete` ≠ gc-path ≠ pack**. Filtering only
  reduces what is compared.

```bash
chunkforge diff --path keep/ left.cfdir right.cfdir
chunkforge diff --path-from includes.txt left.cfdir right.cfdir
chunkforge diff --exclude skip/ --exclude '*.tmp' left.cfdir right.cfdir
chunkforge diff --exclude-from excludes.txt --tree ./src listing.cfdir
```

## `--progress` (Phase 19 / 1.9.0 opt-in; Phase23 honesty)

Opt-in **`--progress`** emits `progress: op=diff done=N/TOTAL` on **stderr**
only. Default **off** ≡ **1.8.0** quiet.

| Rule | Detail |
|---|---|
| Granularity | One tick per **File or Symlink** path in the **union** of both sides after `--path` / `--path-from` / `--exclude` / `--exclude-from` (TOTAL = \|left ∪ right\| filtered File+Symlink paths; Dir-only ignored) |
| JSON | **Orthogonal** — progress never enters the `--format json` object (stdout stays the single diff object) |
| Default | Flag omitted ⇒ no `progress:` lines (≡ 1.8) |

```bash
chunkforge diff --progress left.cfdir right.cfdir
# stderr: progress: op=diff done=1/N …
chunkforge diff --format json --progress left.cfdir right.cfdir
# stdout: JSON object; stderr: progress lines only
```

## Listing ↔ listing

```bash
chunkforge diff [--format text|json] [--progress] [--max-paths N] left.cfdir right.cfdir
```

- Both arguments must be **`.cfdir`** files (`.cfidx` / bad magic → error).
- Bare directories without `--tree` are rejected (use `--tree`, below).
- Comparison is by relative **File + Symlink** path (**Dir-only** entries are
  ignored). Symlink paths participate in added / removed / changed /
  meta_changed (Phase22 library; Phase23 docs align).
- Orientation: **left** = baseline, **right** = comparison.
- **`--symlinks`** is **not** accepted without `--tree` (clap `requires =
  "tree"`). Listing↔listing already compares Symlink in-lib; the flag does
  not change this mode.

| Category | Meaning |
|---|---|
| **added** | Path in right, not in left (File or Symlink) |
| **removed** | Path in left, not in right (File or Symlink) |
| **changed** | Same path; content differs — File: `blob_blake3` and/or size; Symlink: **target** differs; **kind mismatch** (File vs Symlink at the same path) → `changed` |
| **meta_changed** | Same path, same content identity, but meta differs — File: same `blob_blake3`, `mode` and/or `mtime_secs` differ; Symlink: same target, **mode-only** → `meta_changed` (never also in `changed`) |

Chunk-set stats come from the unique chunk ids referenced by **File** entries
on each side (`all_chunk_ids`). Symlink entries contribute **0** chunks:

| Field | Meaning |
|---|---|
| `chunks_shared` | Unique ids present in both |
| `chunks_only_left` | Unique ids only in left |
| `chunks_only_right` | Unique ids only in right |

## Tree ↔ listing (`--tree`)

```bash
chunkforge diff --tree <src-dir> [--symlinks skip|record] [--format text|json] [--max-paths N] <listing.cfdir>
# or:
chunkforge diff [--format text|json] [--max-paths N] --tree <src-dir> [--symlinks skip|record] <listing.cfdir>
```

- Exactly **one** source directory (`--tree <src-dir>`) and **one** listing
  `.cfdir`. Two trees are **not** supported. `.cfidx` is rejected.
- Builds an **ephemeral in-memory `DirArchive`** from the live tree under
  `src-dir` (no store put, no `.cfdir` write, no `--store` flag).
- **`--symlinks skip|record`** (Phase23; **requires `--tree`** via clap):
  - **`skip`** (default ≡ **1.12.0** tree skip+warn): live symlinks and
    special files (fifo/socket/device) are skipped with a `diff:` warn on
    stderr — not recorded / **not followed**. Against an
    `archive --symlinks record` listing this can report false **`added`**
    for symlink paths that exist only on the listing side.
  - **`record`**: push ephemeral `DirEntryKind::Symlink` (target as-is from
    `read_link`; mode from `symlink_metadata`; **0 chunks**; **not
    followed** — directory symlinks are not walked into). Absolute or empty
    target → clear **non-zero** (same policy as `archive --symlinks
    record`). Eliminates false `added` when comparing an identical tree to
    an `archive --symlinks record` listing (exit **0** when otherwise
    identical). Special files still skip+warn.
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

**Responsibility nail:** **`diff --tree --symlinks record` ≠ write mount ≠
follow ≠ pack ≠ sync ≠ prune ≠ `gc --path` ≠ default record**. Default without
the flag (or `--symlinks skip`) stays ≡ **1.12.0** skip+warn.

```bash
# A: archive record listing, then tree default skip (may false-added symlink)
chunkforge archive --store ./store -o tree.cfdir --symlinks record ./src
chunkforge diff --tree ./src tree.cfdir --format json
# B: symmetric record → identical exit 0
chunkforge diff --tree ./src --symlinks record tree.cfdir
```

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
| `added` | string array | Paths in right, not in left (sorted; File or Symlink) |
| `removed` | string array | Paths in left, not in right (sorted; File or Symlink) |
| `changed` | string array | Same path; content differs (File blake3/size; Symlink target; kind mismatch) |
| `meta_changed` | string array | Same path; same content identity; mode(/mtime) differ |
| `chunks_shared` | number | Unique chunk ids in both (File entries only) |
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
| **non-zero (≠1)** | Usage / decode / I/O errors (clap / anyhow) — including tree `--symlinks record` with absolute/empty symlink target |

## Responsibility table

| Tool | Role | Needs listing? | Touches store objects? |
|---|---|---|---|
| **`verify`** | Listing structure + referenced chunk presence/hashes + per-file `blob_blake3` | Yes (`.cfidx` / `.cfdir`) | Read via `--store` / `--source` |
| **`doctor`** | Presence of referenced chunks (`has`, optional `--deep` = `get`) | Yes | Read presence only |
| **`gc`** | Delete (or dry-run) **unreferenced** loose `.cnk` under a local `--store` | Yes (reference set) | May delete unreferenced only (`--apply`) |
| **`diff`** | Path / meta / chunk-**set** comparison of two listings, or tree↔listing (opt-in `--symlinks record` on `--tree`) | Yes (and optional live tree) | **Never** — report only |
| **`store scrub`** | Local CAS **bitrot** check: traverse loose chunks and re-BLAKE3 (`get_verify`); report `ok=` / `corrupt=` / `unreadable=` | **No** (optional refs later) | Read-only rehash; does **not** delete |

`diff` is **not** sync, merge, or verify. **`diff --tree --symlinks record` ≠
write mount ≠ follow ≠ pack ≠ sync ≠ prune ≠ `gc --path` ≠ default record**.
`store scrub` is the “deep integrity / bitrot” companion to `doctor`
(presence) and `gc` (unreferenced reclaim) — scrub does **not** require a
listing and does **not** belong inside `doctor --deep`. See
[`doctor-gc.md`](doctor-gc.md).
