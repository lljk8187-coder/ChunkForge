# `chunkforge ls` (+ `cat --path`)

List paths in a `.cfidx` / `.cfdir` listing (**inventory only**). Phase25 /
toward **1.15.0** (workspace / CLI version stays **1.14.0** until M7).

Consumes public [`DirArchive::decode`](../crates/chunkforge-index/src/dir.rs) /
[`Index::decode`](../crates/chunkforge-index/src/index.rs) (magic-dispatch like
`mount` / `verify`). Optional path 四件套 on `.cfdir` reuses the same
[`PathFilter`](../crates/chunkforge-index/src/filter_dir.rs) /
[`filter_dir_archive`](../crates/chunkforge-index/src/filter_dir.rs) rules as
`mount` / `filter`. **Does not** open a store, fetch chunks, mount, extract,
verify hashes, or write a new listing.

Companion: **`cat --path`** reassembles **one File** from a `.cfdir` to `-o`
(reuses the existing chunk-fetch pipeline). See also [filter.md](filter.md),
[dir-format.md](dir-format.md), [stability.md](stability.md),
[ops-json.md](ops-json.md), [perf.md](perf.md), [mount.md](mount.md),
[extract.md](extract.md). Smoke:
[`scripts/demo_ls_cat_path.sh`](../scripts/demo_ls_cat_path.sh).

## Usage (`ls`)

```bash
chunkforge ls \
  [--path <prefix>]... \
  [--path-from <file>]... \
  [--exclude <pat>]... \
  [--exclude-from <file>]... \
  [--format text|json] \
  [--chunks] \
  <LISTING>
```

| Rule | Detail |
|---|---|
| Input | **`.cfidx` or `.cfdir`** (magic dispatch) |
| `.cfidx` | One logical file path (stem without `.cfidx`, same naming as mount). **Any** path/exclude flag → clear non-zero |
| `.cfdir` | Every **File** / **Dir** / **Symlink** entry present after PathFilter (empty 四件套 ≡ full listing). Dir rows appear only when the listing explicitly stores Dir entries (today's `archive` omits empty dirs → usually 0 Dir rows) |
| Sort | Path lexicographic (stable) |
| Store / chunks | **Never** opens a store; **never** get/put. Pure listing decode. **≠** verify |
| `--format text` | Default. One line per entry on stdout (tab-separated columns) |
| `--format json` | One object on stdout: `{ "ok": true, "entries": [ … ] }`; no text dual-write; exit format-independent |
| `--chunks` | Default **off**. When set, File entries include chunk id list (lowercase hex); Symlink/Dir omit. Still **no** store open — ids come from the listing only |

### Text columns

| Kind | Columns |
|---|---|
| File | `file\tpath\tsize` (+ optional 4th column of comma-joined chunk hex when `--chunks`) |
| Dir | `dir\tpath` |
| Symlink | `symlink\tpath\ttarget` |

### Symlink target / File size / no store

| Surface | Contract |
|---|---|
| Symlink **target** | Printed / JSON `target` = listing-recorded target string (**not** followed; absolute/empty targets stay as recorded) |
| File **size** | Listing plaintext size (bytes); **not** on-disk `.cnk` sum |
| No store | `ls` never requires `--store` / `--source`; chunk ids under `--chunks` are decode-only |

```bash
# Full tree inventory
chunkforge ls tree.cfdir

# Path-scoped (same PathFilter as filter/mount)
chunkforge ls --path pkgs/foo --format json tree.cfdir

# After filter: human / script confirmation of retained set
chunkforge filter --path pkgs/foo -o foo.cfdir tree.cfdir
chunkforge ls foo.cfdir

# Single-blob index
chunkforge ls blob.cfidx --format json --chunks
```

## JSON fields (`ls --format json`)

Minimum stable set (Phase25 / new command field set — **additive**; does not
rename prior command fields). See [ops-json.md](ops-json.md).

| Field | Type | Meaning |
|---|---|---|
| `ok` | bool | `true` on success path |
| `entries` | array | Sorted entry objects |

Each element of `entries`:

| Field | Type | When |
|---|---|---|
| `kind` | string | `"file"` \| `"dir"` \| `"symlink"` |
| `path` | string | Listing-relative path |
| `size` | number | **File only** — plaintext size |
| `target` | string | **Symlink only** — recorded target |
| `chunks` | string array | **File only**, and only when `--chunks` — lowercase hex chunk ids |

Exit codes are **format-independent**.

## `cat --path` (`.cfdir` single File)

```bash
chunkforge cat --store ./store --path pkgs/foo/a.txt -o a.bin foo.cfdir
```

| Rule | Detail |
|---|---|
| `.cfidx` | ≡ 1.14 — reassemble whole blob; **do not** pass `--path` (`.cfidx` + `--path` → clear non-zero). JSON field names stay `ok` / `bytes` (+ optional `cache_*`) |
| `.cfdir` | **Requires** `--path <rel>` exact-matching **one File** entry (normalized: trim; strip leading `./`). Symlink / Dir / missing → clear non-zero |
| Output | Always writes single `-o` file; reuses jobs / cache / fallback / progress / format |
| Multi-file | **Not** this phase — single path only |

Cross-link nail: **`cat --path` ≠ extract ≠ prune ≠ sync**.

| This | Is | Is **not** |
|---|---|---|
| `cat --path` | Reassemble **one** File from a tree listing to `-o` | Whole-tree materialize; deleting extras; multi-file batch |
| `extract --path` | Materialize a **scoped tree** under `-o` (dirs + files + symlinks) | Single-file fetch without a dest tree |
| prune / `--delete` | (non-goal) delete dest extras | `cat --path` never deletes tree files |
| sync / watch | (non-goal) bidirectional / conflict | `cat --path` is one-shot fetch |

## Responsibility nail (CRITICAL)

**`ls` ≠ mount ≠ extract ≠ verify ≠ pack ≠ filter.**

| This | Is | Is **not** |
|---|---|---|
| `ls` | Read-only inventory of listing paths (File/Dir/Symlink) | FUSE session; tree materialize; hash fetch; new listing write |
| `mount` | RO FUSE view (may need fuse; session-typed) | Path inventory without a mount |
| `extract` | Materialize tree (or scoped tree) under `-o` | Listing-only print |
| `verify` | Structure + referenced chunk integrity (opens store/source) | Inventory without I/O |
| `filter` | Persist a **new** scoped `.cfdir` | Read-only list (ls does not write `-o`) |
| pack | (non-goal; [perf.md](perf.md)) multi-chunk objects | ls never changes `.cnk` layout |

## Non-goals (Phase25 ls / cat --path surface)

- opening a store from `ls` / hashing chunks to pretend verify
- write mount / bidirectional sync
- extract prune / `--delete`
- `gc --path`
- pack / remote scrub / aws-sdk
- multi-file `cat --path` / directory expand
- changing other command defaults
- default record symlink / follow / fifo·xattr / offline bundle

Pack stance: [perf.md](perf.md) — **Phase25 / toward 1.15.0 still does not
implement pack**.

Compat gate **`check_compat_1_14.sh`** (Phase25-M5; calls 1_13; no absolute
perf SLA). Smoke:
[`scripts/demo_ls_cat_path.sh`](../scripts/demo_ls_cat_path.sh).
