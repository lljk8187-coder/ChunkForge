# Doctor / GC

Phase 3 optional CLI utilities for store hygiene. **`doctor` is implemented (M4)**; **`gc` is deferred to M5**.

## `chunkforge doctor`

Check that one or more `.cfidx` files are readable and that every referenced chunk is present in a given `--store` / `--source`.

```bash
# Local store — exit 0 when complete
chunkforge doctor --store ./store hello.cfidx

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
| Index readable | Decode + `validate`; corrupt index → non-zero |
| Chunk presence | Default: `ChunkSource::has`; `--deep` → `get` (body discarded) |
| HTTP base probe | One HEAD (GET on 405/501) against `--source` base; `--no-probe` skips |
| Local `meta.toml` | If origin is a local/`file://` store, print magic/version/compression to stderr |
| Exit code | All present → **0**; any missing → **non-zero** and missing ids on **stdout** (one per line) |

## `chunkforge gc` (M5 — not yet)

Local-only dry-run of unreferenced loose `.cnk` files; `--apply` to delete. Not implemented in this milestone.
