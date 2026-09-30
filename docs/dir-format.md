# ChunkForge `.cfdir` directory archive format (v1 / v2)

Phase 5 multi-file listing; Phase 22 adds opt-in **Symlink** kind +
`format_version=2`. Parallel to [`.cfidx`](index-format.md); **not** a
revision of `.cfidx` and **not** bit-compatible with casync `.catar`.

Little-endian. File extension: **`.cfdir`**.

## Layout

```
Offset  Size  Field
0       8     magic = b"CFDIR\0\0\1"   // last byte = major
8       2     format_version_u16      // 1 = File/Dir only; 2 = may contain Symlink
10      2     flags_u16
12      4     reserved_u32 = 0
16      8     entry_count_u64
24      …     entries (variable)
…       32    trailer_checksum        // blake3(header||body)
```

### Entry (variable)

```
u16     path_len
[u8]    path UTF-8 (path_len bytes)
u8      kind_tag   // 1 = File, 2 = Dir, 3 = Symlink (v2 only)
```

**File** (`kind_tag = 1` / `KIND_FILE`):

```
u32     mode
u64     size
u64     mtime_secs
[u8;32] blob_blake3
u64     chunk_count
N×40    entries[]   // same shape as .cfidx: end_offset_u64 + chunk_id[32]
```

**Dir** (`kind_tag = 2` / `KIND_DIR`):

```
u32     mode
```

**Symlink** (`kind_tag = 3` / `KIND_SYMLINK`; requires `format_version=2`):

```
u32     mode
u16     target_len
[u8]    target UTF-8 (target_len bytes; non-empty logical string)
```

Symlink entries contribute **0 chunks** (`DirArchive::all_chunk_ids` ignores
them). The target is the **as-recorded logical string** (not canonicalized,
not followed at archive or mount time).

Chunk table rules for File match `.cfidx`: last `end_offset == size`; empty
iff `size == 0`.

## Path rules

- UTF-8, relative, `/`-separated
- Reject: empty path, absolute (`/…`), empty segments, `.` / `..` segments,
  backslash, NUL, Windows drive letters (`C:…`)
- Paths within one archive must be unique
- Symlink **targets** are separate from listing paths: relative targets are
  the product path; **absolute targets are refused by the CLI** under
  `--symlinks record` (escape prevention). The library encode path may accept
  absolute target strings for roundtrip tests; product CLI rejects them.

## Versioning

| Rule | Behavior |
|---|---|
| Magic last byte | **major** — incompatible changes |
| `format_version_u16` | **minor** — compatible extensions |
| Read (1.12+) | major ≠ 1 → hard fail; `format_version ∈ {1,2}` accepted |
| Write **default** (no `--symlinks record` / no Symlink entries) | major=1, **`format_version=1`**, `reserved=0` (≡ 1.11 / Phase 5) |
| Write with ≥1 Symlink | major=1, **`format_version=2`** |
| v1 body with `KIND_SYMLINK` | clear decode / validate error |
| Unknown kind | clear error |

Trailer checksum covers `header || body` (same spirit as `.cfidx`).

Implemented by `chunkforge-index` (`DirArchive::encode` / `DirArchive::decode`;
constants `DIR_FORMAT_VERSION_V1=1`, `DIR_FORMAT_VERSION_V2=2`,
`KIND_SYMLINK=3`).

## Archive CLI notes (Phase5-M2 + Phase22)

- P0 `chunkforge archive` records **regular files** by default (optional empty
  `Dir` entries **omitted** ≡ 1.14). Opt-in **`archive --empty-dirs`** records
  truly empty leaf directories as `DirEntryKind::Dir` (mode from metadata;
  relative path; same PathFilter as files). **≠** prune **≠** write mount.

- **Default `--symlinks skip`** (≡ **1.11.0**): symlinks are skipped with a
  stderr warning (not followed, not recorded); listing stays **`format_version=1`**.
- **Opt-in `--symlinks record`**: write `DirEntryKind::Symlink` (target as-is;
  **not** followed / **not** `canonicalize`); ≥1 Symlink ⇒ **`format_version=2`**.
  Directory symlinks are **not** walked into. Absolute / empty targets → clear
  non-zero.
- **fifo / socket / device**: still skipped with a stderr warning.
- Narrative: **`archive --symlinks record` ≠ write mount ≠ follow dir symlink
  ≠ pack ≠ offline bundle ≠ prune ≠ `gc --path` ≠ default record**.

## Empty Dir + path filter (Phase26)

Shared helper `filter_dir_archive` (`ls --path`, `filter`, `mount` path,
path-scoped `diff`) decides which explicit Dir rows survive a PathFilter.
Archive bytes are unchanged; this is a **read/filter keep** rule only.

| Filter | Explicit Dir |
|---|---|
| Empty path 四件套 | **Identity** — every Dir already in the listing is kept (≡ 1.15 full listing) |
| Non-empty | Keep the Dir if it is an **ancestor** of a kept File or Symlink **or** `PathFilter::allows(dir path)` (path-matched **leaf** empty Dir from `--empty-dirs`) |
| Unrelated empty Dir | Dropped (fails `allows` and is not an ancestor) |
| Parent missing as an entry | **Not** synthesized (DirFs may still imply parents from File path prefixes) |

So `archive --empty-dirs` then `ls --path <empty_leaf>` / `filter --path <empty_leaf>` keeps that leaf Dir (`dir\t…`; filtered listing non-empty). Default archive **without** `--empty-dirs` still writes no empty Dir rows, so path scope on those listings stays ≡ 1.15.

**`filter_dir_archive` leaf-Dir ≠ prune ≠ `gc --path`.** Keeping the Dir does not delete destination files and does not add a path flag to `gc`. It lines listing consumers up with extract, which already materializes a Dir when `allows` is true. **≠** write mount **≠** pack.

## Extract / verify / mount notes (Phase5-M3 + Phase22)

- `chunkforge extract` materializes File / Dir / **Symlink** (Unix
  `std::os::unix::fs::symlink`); `--force` only overwrites an existing
  **same-type symlink** (never silently turns a file/dir into a symlink).
  Still **no prune** / no `--delete`.
- `chunkforge verify` magic-dispatches: `.cfidx` single-blob vs `.cfdir` tree
  (structure + per-file `blob_blake3`; Symlink = non-empty target + path
  rules; 0 chunks). Optional **`--format json`** may include additive
  `symlinks` count.
- `chunkforge mount` / `DirFs`: Symlink nodes expose `readlink`; still
  **read-only** (write-side FUSE symlink → `EROFS`). Path filter can keep
  Symlink paths like Files.
- `--jobs N` applies to chunk fetches on extract/verify (default 1 = serial).

## Push / doctor / gc (Phase5-M5)

- `chunkforge push` / `doctor` / `gc` accept `.cfdir` alongside `.cfidx` (magic
  dispatch via listing header).
- Reference set for a `.cfdir` = all chunk ids across **File** entries
  (`DirArchive::all_chunk_ids`); **Symlink does not add chunk ids**.
- `push` still uploads **chunks only** — the `.cfdir` listing stays local (same
  as Phase 4 for `.cfidx`).
- See [archive.md](archive.md), [push.md](push.md), [doctor-gc.md](doctor-gc.md),
  [extract.md](extract.md), [mount.md](mount.md).
