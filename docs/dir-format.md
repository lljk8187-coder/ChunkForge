# ChunkForge `.cfdir` directory archive format (v1)

Phase 5 multi-file listing. Parallel to [`.cfidx`](index-format.md); **not** a
revision of `.cfidx` and **not** bit-compatible with casync `.catar`.

Little-endian. File extension: **`.cfdir`**.

## Layout

```
Offset  Size  Field
0       8     magic = b"CFDIR\0\0\1"   // last byte = major
8       2     format_version_u16 = 1
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
u8      kind_tag   // 1 = File, 2 = Dir
```

**File** (`kind_tag = 1`):

```
u32     mode
u64     size
u64     mtime_secs
[u8;32] blob_blake3
u64     chunk_count
N×40    entries[]   // same shape as .cfidx: end_offset_u64 + chunk_id[32]
```

**Dir** (`kind_tag = 2`):

```
u32     mode
```

Chunk table rules match `.cfidx`: last `end_offset == size`; empty iff `size == 0`.

## Path rules

- UTF-8, relative, `/`-separated
- Reject: empty path, absolute (`/…`), empty segments, `.` / `..` segments,
  backslash, NUL, Windows drive letters (`C:…`)
- Paths within one archive must be unique

## Versioning

| Rule | Behavior |
|---|---|
| Magic last byte | **major** — incompatible changes |
| `format_version_u16` | **minor** — compatible extensions |
| Read | major ≠ 1 → hard fail (“please upgrade chunkforge”) |
| Write (Phase 5 / v1) | major=1, `format_version=1`, `reserved=0` |

Trailer checksum covers `header || body` (same spirit as `.cfidx`).

Implemented by `chunkforge-index` (`DirArchive::encode` / `DirArchive::decode`).

## Archive CLI notes (Phase5-M2)

- P0 `chunkforge archive` records **regular files** only (optional empty `Dir` entries omitted).
- **Symlinks**: skipped with a stderr warning (not followed, not recorded). Recording symlink targets is deferred (no `Symlink` kind in `.cfdir` v1 yet).
- **fifo / socket / device**: skipped with a stderr warning.

## Extract / verify notes (Phase5-M3)

- `chunkforge extract --store|--source … archive.cfdir -o out-dir` materializes regular files (and explicit `Dir` entries). Parents are created as needed. If a destination path already exists → non-zero exit (no `--force` yet).
- `chunkforge verify` magic-dispatches: `.cfidx` single-blob (unchanged) vs `.cfdir` tree (structure + per-file `blob_blake3`; missing chunk → non-zero with chunk id in the message). Optional **`--format json`** (default **text** ≡ 0.9.0) emits one JSON object on stdout (`ok`/`kind`/`bytes|files`/`chunks`); exit code is format-independent.
- `--jobs N` applies to chunk fetches on extract/verify (default 1 = serial).

## Push / doctor / gc (Phase5-M5)

- `chunkforge push` / `doctor` / `gc` accept `.cfdir` alongside `.cfidx` (magic
  dispatch via listing header).
- Reference set for a `.cfdir` = all chunk ids across file entries
  (`DirArchive::all_chunk_ids`).
- `push` still uploads **chunks only** — the `.cfdir` listing stays local (same
  as Phase 4 for `.cfidx`).
- See [archive.md](archive.md), [push.md](push.md), [doctor-gc.md](doctor-gc.md).

