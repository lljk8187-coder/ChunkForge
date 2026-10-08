# `chunkforge filter`

Persist a **path-scoped subset** of an existing `.cfdir` listing to a new
`.cfdir`. Phase24 / **1.14.0** closeout. Phase26 / **1.16.0** closeout
extends shared `filter_dir_archive`:
a **non-empty** PathFilter also **keeps an explicit Dir** when
`PathFilter::allows(path)` (leaf empty Dir from `archive --empty-dirs`).
Empty filter stays **identity**.

Consumes library [`filter_dir_archive`](../crates/chunkforge-index/src/filter_dir.rs)
(same PathFilter as `archive` / `extract` / `mount` / `diff` / `doctor` /
`verify`) then [`DirArchive::encode`](../crates/chunkforge-index/src/dir.rs).
**Does not** open a store, rechunk, walk a source tree, prune a dest tree, or
rewrite the input listing in place.

See also [dir-format.md](dir-format.md), [stability.md](stability.md),
[ops-json.md](ops-json.md), [perf.md](perf.md), [diff.md](diff.md),
[mount.md](mount.md), [ls.md](ls.md), [store.md](store.md). Smoke:
[`scripts/demo_filter_listing.sh`](../scripts/demo_filter_listing.sh) (Phase24)
and
[`scripts/demo_empty_dir_path_store_get.sh`](../scripts/demo_empty_dir_path_store_get.sh)
(Phase26 leaf-Dir + `store get`).

## Usage

```bash
chunkforge filter \
  [--path <prefix>]... \
  [--path-from <file>]... \
  [--exclude <pat>]... \
  [--exclude-from <file>]... \
  [--dry-run] \
  [--force] \
  [--format text|json] \
  -o <OUTPUT.cfdir> \
  <INPUT.cfdir>
```

| Rule | Detail |
|---|---|
| Input | **`.cfdir` only**. `.cfidx` / wrong magic → clear non-zero |
| Output | **`-o` required**. Without `--force`, existing `-o` → clear non-zero (no silent overwrite). With `--force`, atomic replace (temp sibling + rename) |
| Empty path 四件套 | ≡ **identity** (library contract): clone of the input listing, including every explicit **Dir** (not only the File+Symlink leaf set) ≡ 1.15 full listing; encode may normalize / recompute `format_version` from retained entries |
| Path 四件套 | `--path` / `--path-from` / `--exclude` / `--exclude-from` — same semantics as archive/extract/mount/diff (OR includes; excludes after; illegal middle `*` / `**` → clear error) |
| Kept entries | Matching **File** and **Symlink** leaves + **ancestor Dir** entries **and**, when the filter is non-empty, explicit **Dir** entries whose path `PathFilter::allows` (leaf empty Dir). Unrelated empty Dirs drop. **Do not** synthesize a Dir that was not in the input. Symlinks are **not** followed. **`filter_dir_archive` leaf-Dir ≠ prune ≠ `gc --path`** |
| Store / chunks | **Never** opens a store; **never** get/put. Pure listing transform |
| Encode version | `DirArchive::encode` recomputes write version from retained entries: ≥1 Symlink ⇒ **v2**; no Symlink ⇒ **v1** |
| `--dry-run` | Compute filtered listing + counts; **do not** write `-o` (still takes `-o` as the planned path). Orthogonal to `--format` |
| `--format text` | Default. Stderr summary (`filter: wrote …` or `filter: dry-run: …`) |
| `--format json` | One JSON object on stdout; no text dual-write; exit codes format-independent |

```bash
# Subset listing from an existing full tree (no source tree needed)
chunkforge filter --path pkgs/foo -o foo.cfdir full.cfdir --format json
chunkforge verify --store ./store foo.cfdir

# Empty filter ≡ identity (re-encode / normalize)
chunkforge filter -o copy.cfdir full.cfdir
chunkforge diff full.cfdir copy.cfdir   # expect identical / exit 0

# Plan-only
chunkforge filter --path pkgs/foo --dry-run -o foo.cfdir full.cfdir --format json

# Leaf empty Dir kept (Phase26; shared filter_dir_archive — not prune)
chunkforge filter --path empty_leaf -o empty.cfdir full.cfdir
chunkforge ls empty.cfdir    # dir\tempty_leaf  (listing non-empty)
```

## JSON fields (`--format json`)

Minimum stable set (Phase24 / new command field set — **additive**; does not
rename prior command fields):

| Field | Type | Meaning |
|---|---|---|
| `ok` | bool | `true` on success path |
| `dry_run` | bool | `true` under `--dry-run`; else `false` |
| `input` | string | Input `.cfdir` path (display) |
| `output` | string | Planned / written `-o` path (display) |
| `files` | number | Retained File entry count |
| `dirs` | number | Retained Dir entry count (ancestors of kept leaves **plus** path-matched explicit / leaf Dirs) |
| `symlinks` | number | Retained Symlink entry count |
| `excluded` | number | Input File+Symlink leaves that failed PathFilter (Dirs that are neither ancestors nor path-matched are not counted — same leaf accounting as archive) |

Exit codes are **format-independent**. See [ops-json.md](ops-json.md).

## Responsibility nail (CRITICAL)

**`filter` ≠ prune ≠ `gc --path` ≠ sync ≠ write mount ≠ pack ≠ `archive --path`.**

**`filter_dir_archive` leaf-Dir ≠ prune ≠ `gc --path`.** Keeping a
path-matched explicit Dir (empty leaf from `--empty-dirs`) is a **listing
keep** on an entry that already exists. It does not delete destination-tree
files and it does not add `gc --path`.

| This | Is | Is **not** |
|---|---|---|
| `filter` | Pure listing transform: existing `.cfdir` → new scoped `.cfdir` | Walking a source tree; rewriting input in place |
| leaf-Dir keep | Non-empty PathFilter retains an explicit Dir when `allows(path)` (shared with `ls` / `mount` / path-scoped `diff`) | prune; `gc --path`; synthesizing a ghost Dir |
| `archive --path` | Walk + chunk a **source tree** with PathFilter | Transforming an existing listing without the tree |
| prune / `--delete` | (non-goal) delete dest extras | filter never deletes tree files |
| `gc --path` | (**hard ban**) shrink GC keep-set via path flags | filter does **not** add `gc --path`; leaf-Dir keep is **not** gc-path |
| sync / watch | (non-goal) bidirectional / conflict | filter is one-shot listing I/O |
| write mount | (non-goal) writable FUSE | filter writes a **file**, not a mount |
| pack | (non-goal; [perf.md](perf.md)) multi-chunk objects | filter never changes `.cnk` layout |

### Warning: filtered listing → `gc`

Taking a **filtered** listing into `gc` uses **that listing's** reference set.
A narrower listing ⇒ a narrower keep-set ⇒ chunks only referenced by filtered-out
paths become unreferenced and may be deleted under `gc --apply`. Document this
risk — and **still provide no `gc --path` flag** (hard ban). Prefer passing the
**full** listing to `gc` unless you intentionally want the narrowed keep-set.

## Symlink / encode version

| Case | Result |
|---|---|
| Input has Symlink; filter **keeps** ≥1 Symlink | Output writes **`format_version=2`** (encode recomputes) |
| Filter drops **all** Symlinks (File/Dir only remain) | Output writes **`format_version=1`** via encode |
| Empty filter on a v2 listing | Identity (all entries, including explicit Dirs); still v2 if Symlinks remain |

Symlink entries contribute **0** chunks (same as archive `--symlinks record`).

## Non-goals (Phase24 filter surface)

- prune / `--delete` / rewrite input in place (leaf-Dir keep is **not** prune)
- `gc --path` (leaf-Dir keep is **not** gc-path)
- write mount / bidirectional sync
- pack / remote scrub / aws-sdk
- opening a store / rechunking / source-tree walk
- changing other command defaults
- default record symlink / follow / fifo·xattr / offline bundle

Pack stance: [perf.md](perf.md) — **Phase24 / 1.14.0 still does not
implement pack**. **Phase26 / 1.16.0 still does not implement pack**
(leaf-Dir is listing metadata only).

Compat gate **`check_compat_1_13.sh`** (Phase24-M5/M7) asserts `filter` help +
thin Symlink-keep subset; calls `check_compat_1_12`. Smoke:
[`scripts/demo_filter_listing.sh`](../scripts/demo_filter_listing.sh).
Phase26 smoke:
[`scripts/demo_empty_dir_path_store_get.sh`](../scripts/demo_empty_dir_path_store_get.sh);
gate **`check_compat_1_15.sh`** (Phase26; calls 1_14; asserts leaf-Dir keep on
`ls --path` / `filter --path` + thin `store get`).
