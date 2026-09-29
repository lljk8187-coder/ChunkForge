# ChunkForge `.cfidx` index format (v1)

Phase 1 native index. Semantically aligned with casync/desync (index + chunk store +
reassembly); **not** bit-compatible with `.caibx`.

Little-endian. File extension: **`.cfidx`**.

## Layout

```
Offset  Size  Field
0       8     magic = b"CFIDX\0\0\1"   // last byte = major
8       2     format_version_u16 = 1
10      2     flags_u16               // bit0: chunks_compressed_in_store (hint)
12      4     reserved_u32 = 0
16      8     chunk_size_min_u64
24      8     chunk_size_avg_u64
32      8     chunk_size_max_u64
40      8     total_size_u64          // plaintext blob byte length
48      8     chunk_count_u64
56      32    blob_blake3             // BLAKE3 of entire plaintext blob
88      N*40  entries[]
88+N*40 32    trailer_checksum        // blake3(header||entries) first 32B
```

### Entry (40 bytes, fixed)

```
offset 0   u64     end_offset   // exclusive end in blob; first chunk start = 0
offset 8   [u8;32] chunk_id     // BLAKE3 raw bytes (not hex)
```

- Length of chunk `i` = `end_offset[i] - end_offset[i-1]` (`prev=0` for `i=0`).
- Last `end_offset` must equal `total_size`.
- `chunk_count == 0` iff `total_size == 0` (empty file).

## Versioning

| Rule | Behavior |
|---|---|
| Magic last byte | **major** — incompatible changes (hash algorithm, entry width) |
| `format_version_u16` | **minor** — compatible extensions |
| Read | major ≠ expected → **hard fail** (“please upgrade chunkforge”) |
| Write (Phase 1) | major=1, `format_version=1`, `reserved=0` |

Trailer checksum covers `header || entries` and fails fast on truncate / bit flip.

Implemented by the `chunkforge-index` crate (`Index::encode` / `Index::decode`).
