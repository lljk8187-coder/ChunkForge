# `chunkforge store`

Local CAS store subcommands. This page covers **`store create`** (Phase 19).
See also [doctor-gc.md](doctor-gc.md) for **`store scrub`** / **`store stats`** /
**`du`**, and [ops-json.md](ops-json.md) for JSON field rows.

## `store create` (Phase 19 / 1.9.0 opt-in)

Create an **empty** local CAS store by calling library **`Store::create`**.
Writes `meta.toml` + `chunks/` under `--store`.

```bash
chunkforge store create --store <DIR> [--compression none|zstd] [--format text|json]
```

| Flag | Meaning |
|---|---|
| `--store` | Directory to create (required) |
| `--compression` | `none` \| `zstd` (case-insensitive). **Omit ≡ `none`** (≡ **1.8.0** create default) |
| `--format` | `text` (default) or `json` |

### Behaviour

| Rule | Detail |
|---|---|
| Success | Creates empty store; text → stderr `store create: ok store=… compression=…`; json → stdout `{"ok":true,"store":"…","compression":"none\|zstd"}` |
| Already exists | If `meta.toml` already present → **clear non-zero** (does **not** overwrite) |
| ≠ recompress | **`store create` ≠ `store recompress`** — there is no recompress / in-place migrate command |
| ≠ trim / LRU / pack | Observation and lifecycle only; does not change defaults for make/archive/pull omit paths |
| Disk ≠ wire | On-disk zstd is **not** HTTP Content-Encoding / wire compression |

### Examples

```bash
# Empty none store (≡ 1.8 create default)
chunkforge store create --store ./store-none

# Empty zstd store, then pull into it (omit pull --compression; meta wins)
chunkforge store create --store ./store-z --compression zstd --format json
chunkforge pull --store ./store-z --source ./store-src ./blob.cfidx
chunkforge store stats --store ./store-z --format json   # compression=zstd

# Repeat create → non-zero
chunkforge store create --store ./store-z --compression zstd
# error: store already exists …
```

Smoke: [`scripts/demo_store_create_pull_compression.sh`](../scripts/demo_store_create_pull_compression.sh).
