# `chunkforge store`

Local CAS store subcommands. This page covers **`store create`** (Phase 19),
**`store list`** (Phase 21 / P1), and **`store get`** (Phase26 /
**1.16.0**). See also [doctor-gc.md](doctor-gc.md)
for **`store scrub`** / **`store stats`** / **`du`**, and [ops-json.md](ops-json.md)
for JSON field rows.

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

## `store list` (Phase 21 / 1.11.0 opt-in)

Enumerate every loose chunk id in a local CAS via library
**`Store::list_chunk_ids`**. Read-only observation.

```bash
chunkforge store list --store <DIR> [--format text|json]
```

| Flag | Meaning |
|---|---|
| `--store` | Local CAS directory (required; same convention as other `store` subcommands) |
| `--format` | `text` (default) or `json` |

### Behaviour

| Rule | Detail |
|---|---|
| Success (text) | One lowercase hex id per line, **stably sorted**; empty store → no lines |
| Success (json) | One object on stdout: `{"ok":true,"chunks":N,"ids":["…",…]}`; `ids` sorted; no text dual-write |
| Exit | Format-independent; success → 0 |
| ≠ GC / scrub / trim / LRU | **Does not** delete, rehash, reclaim, or evict — enumeration only |
| ≠ mount path / prune / sync / pack | Store-local listing only; unrelated to FUSE path filters |

### Examples

```bash
chunkforge store create --store ./empty
chunkforge store list --store ./empty          # no lines
chunkforge store list --store ./empty --format json
# → {"ok":true,"chunks":0,"ids":[]}

chunkforge make --store ./s -o out.cfidx ./hello.txt
chunkforge store list --store ./s              # sorted hex ids, one per line
chunkforge store list --store ./s --format json
```

## `store get` (Phase26 / 1.16.0)

Fetch **one** chunk's plaintext from a local store (`Store::get_verify`) and
write it to **`-o`**. Single hex id. Not a tree export.

```bash
chunkforge store get --store <DIR> <HEX_ID> -o <FILE> [--verify] [--format text|json]
```

| Flag | Meaning |
|---|---|
| `--store` | Local CAS directory (required) |
| `<HEX_ID>` | Chunk id, 64 lowercase hex characters (same positional as `store has`) |
| `-o` / `--output` | Plaintext output file (**required**) |
| `--verify` | Opt-in re-hash (`get_verify(id, true)`). **Omit ≡ trust on-disk encoding** (`get_verify(id, false)`; no BLAKE3 re-hash) |
| `--format` | `text` (default) or `json` |

### Behaviour

| Rule | Detail |
|---|---|
| Success (text) | Writes `-o`; stderr `store get: ok id=<hex> bytes=N` |
| Success (json) | Writes `-o`; one stdout object **`{ok,id,bytes}`** (`ok` true, `id` hex, `bytes` plaintext length). No text dual-write. Exit format-independent |
| Missing / bad id | Clear non-zero (`store get: missing …` / `store get: bad chunk id: …`). Corrupt under `--verify` → non-zero |
| Local only | Opens `Store` at `--store`. No HTTP source, no multi-id batch |

### Responsibility nail (CRITICAL)

**`store get` ≠ scrub ≠ cat ≠ extract ≠ recompress ≠ remove.**

| This | Is | Is **not** |
|---|---|---|
| `store get` | Read **one** loose chunk's plaintext to `-o` | Walking the store; rewriting `.cnk` |
| `store scrub` | Rehash every loose chunk (or a listing's refs); no payload file | Single-id byte fetch |
| `cat` | Reassemble a listing (`.cfidx` blob or `.cfdir --path` File) | Raw chunk-id fetch without a listing |
| `extract` | Materialize a tree under `-o` | One chunk file |
| `store recompress` | (non-goal) migrate none↔zstd in place | `store get` never rewrites store encoding |
| `store remove` / trim | (non-goal) delete or evict chunks | `store get` is read-only |

Also **≠** pack **≠** write mount **≠** `gc --path` **≠** prune. Loose `.cnk` layout unchanged.

### Examples

```bash
ID=$(chunkforge store list --store ./s | head -1)
chunkforge store get --store ./s "$ID" -o /tmp/c.bin
chunkforge store get --store ./s --verify "$ID" -o /tmp/c2.bin --format json
# → {"ok":true,"id":"<hex>","bytes":N}
```

Smoke: [`scripts/demo_empty_dir_path_store_get.sh`](../scripts/demo_empty_dir_path_store_get.sh).
Gate: [`check_compat_1_15.sh`](../scripts/check_compat_1_15.sh) (Phase26;
calls 1_14; thin `store get -o` bytes check). `-o` stays required (stdout
output deferred).

