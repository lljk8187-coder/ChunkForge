# Doctor / GC / Scrub

Phase 3 optional CLI utilities for store hygiene, plus Phase 7 CAS bitrot scrub.
**`doctor`**, **`gc`**, and **`store scrub`** are implemented.

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
| `--format text\|json` | Default **text** ≡ 0.9.0. **json**: one object on stdout (`ok`, `listings`, `checked`, `missing`, `deep`, `retries`); when chunks are missing, `missing` is an array of hex ids (not also printed as bare lines). Exit code is format-independent. |

```bash
# Machine-readable presence report (Phase 10 M6 O1)
chunkforge doctor --store ./store --format json hello.cfidx
# → {"ok":true,"listings":1,"checked":N,"missing":0,"deep":false,"retries":0}
```

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

# Parallel deletes on --apply (default --jobs 1 ≡ 1.1.0 serial; symmetric to store scrub)
chunkforge gc --store ./store --jobs 4 --apply hello.cfidx release.cfdir

# Machine-readable report (Phase 12 M2); jobs orthogonal to format
chunkforge gc --store ./store --format json hello.cfidx release.cfdir
# → {"ok":true,"dry_run":true,"applied":false,"listings":2,"referenced":N,"unreferenced":M,"deleted":0}

chunkforge gc --store ./store --apply --format json --jobs 4 hello.cfidx
# → {"ok":true,"dry_run":false,"applied":true,"listings":1,"referenced":N,"unreferenced":M,"deleted":M}
```

| Rule | Behaviour |
|---|---|
| Reference set | Union of chunk ids from all given `.cfidx` / `.cfdir` listings |
| Scan | Walk local `store/chunks/**/*.cnk` via `Store::list_chunk_ids()` (layout-conforming only) |
| Dry-run (default) | Print absolute paths of unreferenced `.cnk` files to **stdout** (ordered/serial); summary on stderr; **no deletes** |
| `--apply` | `Store::remove` each unreferenced id; referenced chunks retained |
| `--jobs N` | Bounded concurrency for `--apply` deletes (default **1** = serial ≡ 1.1.0); dry-run path listing stays ordered; result set (which ids) identical for any `N >= 1`; same helper as `store scrub --jobs`; orthogonal to `--format` |
| `--format text\|json` | Default **text** ≡ 1.1.0 (paths on stdout + stderr summary). **json**: one object on stdout — `ok` (bool), `dry_run` (bool, `!apply`), `applied` (bool), `listings` (usize), `referenced` (usize), `unreferenced` (candidate count this run), `deleted` (actual deletes; **0** on dry-run). No path listing and no duplicate stderr summary. Exit code is format-independent. |
| Remote | **Not supported** — `--store` is local only |
| Exit code | 0 on success (including “nothing to reclaim”); non-zero on I/O / bad listing |


## `chunkforge store scrub`

Local-only **bitrot / integrity** check of every loose `.cnk` under `--store`.
Does **not** take a listing: it walks `Store::list_chunk_ids()` and re-verifies
plaintext BLAKE3 via `get_verify` (must equal the chunk id).

**Read-only** — never deletes. Pair with `doctor` (presence of referenced chunks)
and `gc` (reclaim unreferenced). See also [`diff.md`](diff.md) responsibility table.

```bash
# Healthy store → exit 0
chunkforge store scrub --store ./store
# → scrub: ok=N corrupt=0 unreadable=0

# Parallel workers (default --jobs 1 ≡ serial)
chunkforge store scrub --store ./store --jobs 4

# Machine-readable (Phase 12 M3); default text ≡ 1.1.0
chunkforge store scrub --store ./store --format json
chunkforge store scrub --store ./store --jobs 4 --format json

# Empty store → ok=0 corrupt=0 unreadable=0, exit 0
chunkforge store scrub --store ./empty-store

# Flip one byte in a .cnk → corrupt=1, exit non-zero
# → scrub: corrupt <64-hex-id>
# → scrub: ok=… corrupt=1 unreadable=0
# → error: scrub: 1 bad chunk(s) …
```

| Rule | Behaviour |
|---|---|
| Scope | All layout-conforming loose chunks under local `--store` (no listing required) |
| Check | `get_verify(id, true)` — decompress (if any) then plaintext BLAKE3 ≡ id |
| Outcome | success → `ok++`; hash mismatch / corrupt payload → `corrupt++` + `scrub: corrupt <id>`; I/O / decode / other read failure → `unreadable++` + `scrub: unreadable <id>` |
| Summary | One line: `scrub: ok=… corrupt=… unreadable=…` |
| `--jobs N` | Bounded concurrency (default **1** = serial); same helper as other CLI commands; orthogonal to `--format` |
| `--format text\|json` | Default **text** ≡ 1.1.0 (`scrub: ok=…` summary + per-bad-id lines on stdout). **json**: one object on stdout — `ok` (bool; true iff corrupt+unreadable==0), `checked` (total scanned = ok_count+corrupt+unreadable), `ok_count` (healthy; aligns with text `ok=`), `corrupt` / `unreadable` (counts), `corrupt_ids` / `unreadable_ids` (hex string arrays; bad ids **only** here — no text lines). No duplicate text summary. Exit code is format-independent. |
| Repair | **None** — report only; do not auto-delete (re-pull / replace bad objects separately) |
| Exit code | corrupt+unreadable == 0 → **0**; else **non-zero** (same for text and json) |

## Presence vs scrub vs GC

| Tool | Question it answers |
|---|---|
| **`doctor`** | Are chunks **referenced by listings** present? (`has`, optional `--deep` = `get`) |
| **`store scrub`** | Are **objects already in the local CAS** bit-rot free? (re-BLAKE3; no listing) |
| **`gc`** | Which loose chunks are **unreferenced** and can be reclaimed? (dry-run / `--apply`) |

`doctor --deep` is still presence-oriented (fetch/discard), **not** a full-store
scrub. Use `store scrub` when you want every on-disk `.cnk` rehashed.

There is **no** remote scrub command. For listing-referenced remote integrity,
use **`verify --source`** (default hash check after GET; optional
`--http-retries` — see [http-retry.md](http-retry.md)).

## `.cfdir` notes (Phase5-M5)

- `doctor` / `gc` magic-dispatch each positional arg: `.cfidx` (single blob) or
  `.cfdir` (all file-entry chunk ids via `DirArchive::all_chunk_ids`).
- Mixing both kinds in one invocation is supported; the keep / check set is the
  **union**.
- Deep doctor checks (`--deep`) work the same for both listing kinds.

