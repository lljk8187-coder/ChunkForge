# Doctor / GC

Phase 3 optional CLI utilities for store hygiene. **`doctor` (M4)** and **`gc` (M5)** are implemented.

## `chunkforge doctor`

Check that one or more `.cfidx` / `.cfdir` listings are readable and that every referenced chunk is present in a given `--store` / `--source`.

```bash
# Local store — exit 0 when complete
chunkforge doctor --store ./store hello.cfidx release.cfdir

# Deliberately missing chunk → non-zero + missing ids on stdout
chunkforge doctor --store ./store hello.cfidx
# → <64-hex-id>
# → error: doctor: 1 missing chunk …

# HTTP source (uses ChunkSource::has → HEAD, with GET fallback)
chunkforge doctor --source http://127.0.0.1:8765 hello.cfidx

# Template flags (same as cat/verify/mount; HTTP only)
chunkforge doctor \
  --source 'https://minio.example/mybucket' \
  --prefix 'data/' \
  --url-template '{base}/{prefix}{path}' \
  --header 'Authorization: Bearer {env:TOKEN}' \
  hello.cfidx other.cfidx

# --deep: presence via get (discard body) instead of has
chunkforge doctor --source http://127.0.0.1:8765 --deep hello.cfidx

# Skip the one-shot HTTP base HEAD/GET probe
chunkforge doctor --source http://127.0.0.1:8765 --no-probe hello.cfidx
```

| Check | Behaviour |
|---|---|
| Listing readable | Decode + `validate` for `.cfidx` or `.cfdir`; corrupt listing → non-zero |
| Chunk presence | Default: `ChunkSource::has`; `--deep` → `get` (body discarded) |
| HTTP base probe | One HEAD (GET on 405/501) against `--source` base; `--no-probe` skips |
| Local `meta.toml` | If origin is a local/`file://` store, print magic/version/compression to stderr |
| Exit code | All present → **0**; any missing → **non-zero** and missing ids on **stdout** (one per line) |

## `chunkforge gc`

Local-only garbage collection of loose `.cnk` files that are **not** referenced by any of the given `.cfidx` / `.cfdir` listings.

**Default is dry-run** (print paths + count; delete nothing). Pass `--apply` to actually delete. No remote GC / object-storage lifecycle.

```bash
# Dry-run: list unreferenced loose chunks
chunkforge gc --store ./store hello.cfidx release.cfdir
# → <store>/chunks/<2hex>/<62hex>.cnk   (one path per line on stdout)
# → gc: dry-run: N unreferenced chunk(s) …

# Actually delete unreferenced chunks
chunkforge gc --store ./store hello.cfidx release.cfdir --apply
# → same paths on stdout, then files removed
# → gc: deleted N unreferenced chunk(s) …
```

| Rule | Behaviour |
|---|---|
| Reference set | Union of chunk ids from all given `.cfidx` / `.cfdir` listings |
| Scan | Walk local `store/chunks/**/*.cnk` via `Store::list_chunk_ids()` (layout-conforming only) |
| Dry-run (default) | Print absolute paths of unreferenced `.cnk` files to **stdout**; summary on stderr; **no deletes** |
| `--apply` | Serial `remove` of those files; referenced chunks retained |
| Remote | **Not supported** — `--store` is local only |
| Exit code | 0 on success (including “nothing to reclaim”); non-zero on I/O / bad listing |

## `.cfdir` notes (Phase5-M5)

- `doctor` / `gc` magic-dispatch each positional arg: `.cfidx` (single blob) or
  `.cfdir` (all file-entry chunk ids via `DirArchive::all_chunk_ids`).
- Mixing both kinds in one invocation is supported; the keep / check set is the
  **union**.
- Deep doctor checks (`--deep`) work the same for both listing kinds.

