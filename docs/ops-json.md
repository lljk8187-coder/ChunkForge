# Ops JSON field matrix

Stable fields for CLI commands that support **`--format text|json`**.
Default is always **`text`** (human stderr/stdout summaries ≡ prior minor).
With **`--format json`**, each command emits **one JSON object on stdout**
(no duplicate text summary). Exit codes are **format-independent**.

**Field rename → breaking** (requires a **major** bump, or an explicit breaking
note). See [stability.md](stability.md) Breaking-change policy. Additive keys
in a minor are allowed when defaults stay compatible; this matrix lists the
**minimum stable set** scripts may rely on.

Cross-links: [doctor-gc.md](doctor-gc.md) (`gc` / `store scrub` / `store stats` / `doctor`),
[diff.md](diff.md), [archive.md](archive.md), [extract.md](extract.md),
[push.md](push.md), [pull.md](pull.md).

## Matrix (command × minimum stable fields)

| Command | Minimum stable fields | Notes |
|---|---|---|
| **`archive`** (write) | `ok`, `dry_run` (`false`), `files`, `dirs`, `chunks`, `written`, `reused`, `seed_reused_files`, `rechunked_files`, `skipped_symlinks`, `skipped_special`, `excluded` | Phase 13 §3.3 / M5. Default **text** ≡ 1.2.0. `excluded` = regular files rejected by `--path`/`--exclude` (0 when no filter). Empty Dir entries omitted → `dirs` usually 0. |
| **`archive`** (dry-run) | `ok`, `dry_run` (`true`), `files`, `dirs`, `chunks`, `would_write`, `would_reuse`, `seed_reused_files`, `rechunked_files`, `skipped_symlinks`, `skipped_special`, `excluded` | Dry-run uses **`would_write` / `would_reuse`** (not `written` / `reused`). Other counters present in both modes. |
| **`diff`** | `added`, `removed`, `changed`, `meta_changed` (string arrays of paths); `chunks_shared`, `chunks_only_left`, `chunks_only_right` (numbers) | No top-level `ok`. Differences → exit **1** (like `diff(1)`). `--max-paths` does **not** truncate JSON arrays. |
| **`verify`** | `ok` (bool); `kind` (`"cfidx"` \| `"cfdir"`); **cfidx:** `bytes`, `chunks`; **cfdir:** `files`, `chunks` | Success-only object shown here; failure paths bail before JSON. |
| **`doctor`** | `ok`, `listings`, `checked`, `missing`, `deep`, `retries` | When complete: `missing` is **`0`** (number). When gaps: `missing` is a **string array** of hex ids (not also printed as bare lines). |
| **`extract`** (write) | `ok`, `dry_run` (`false`), `skipped`, `wrote`, `dirs` | Always emits `skipped`/`wrote`/`dirs` (`skipped=0` when `--skip-unchanged` off). Path filter does **not** rename fields; counts reflect the filtered set. |
| **`extract`** (dry-run) | `ok`, `dry_run` (`true`), `would_skip`, `would_write`, `would_dirs`, `would_fail` | No chunk gets; exit **0** when listing is valid even if `would_fail > 0`. |
| **`push`** | `ok`, `skipped`, `uploaded`, `failed`, `failed_transient`, `failed_permanent`, `retries`, `unique_chunks`, `listings`, `dry_run` | JSON emitted before non-zero exit on `failed > 0`. With `--path`/`--exclude`, **`unique_chunks` = filtered** unique id count (field name unchanged; ≡ pull). |
| **`pull`** | `ok`, `skipped`, `fetched`, `failed`, `failed_transient`, `failed_permanent`, `retries`, `unique_chunks`, `listings`, `dry_run` | Same shape as push with `fetched` instead of `uploaded`. With `--path`/`--exclude`, **`unique_chunks` = filtered** unique id count (field name unchanged). |
| **`gc`** | `ok`, `dry_run`, `applied`, `listings`, `referenced`, `unreferenced`, `deleted` | Phase 12 §3.2. `unreferenced` = candidate count this run; `deleted` = actual deletes (**0** on dry-run). Both always present. No path listing on json. `--jobs` orthogonal. |
| **`store scrub`** | `ok`, `checked`, `ok_count`, `corrupt`, `unreadable`, `corrupt_ids`, `unreadable_ids` | Phase 12 §3.2. `checked` = total scanned; `ok_count`/`corrupt`/`unreadable` partition; bad ids **only** in arrays. `ok` true iff corrupt+unreadable==0. `--jobs` orthogonal. Phase 15 P1 `--listing` scopes `checked` to listing refs (field names unchanged; **not** remote scrub). |
| **`store stats`** (alias **`du`**) | `ok`, `chunks`, `bytes_on_disk`, `bytes_plaintext` (`number` \| `null`); optional `compression` (`"none"` \| `"zstd"`) | Phase 14 M2 + Phase 16 M4. Read-only; `chunks` = `list_chunk_ids` count; `bytes_on_disk` = sum of `.cnk` `metadata().len()` (no plaintext decode). `bytes_plaintext`: `compression=none` → equals `bytes_on_disk` (cheap); `compression=zstd` → `null` unless opt-in **`--decode`** (full-store `get` sum). Default **text**: `store stats: chunks=N bytes_on_disk=M [bytes_plaintext=P] compression=…` (`bytes_plaintext` printed only when known). **json**: one object; `bytes_plaintext` number or `null`; no text dual-write. Old field names unchanged. **Not** GC / scrub / trim / LRU. |
| **`make`** | `ok`, `bytes`, `chunks`, `new`, `reused` | Phase 15. Default **text** ≡ 1.4.0 stderr `make: wrote … (BYTES bytes, N chunk(s); new=X, reused=Y)`. **json**: one object on stdout; no text dual-write; exit format-independent. `bytes` = input size; `chunks` = chunk count; `new`/`reused` align stderr (`PutOutcome::Written` / `SkippedExists`). Field set **frozen** for scripts. |
| **`cat`** | `ok`, `bytes` | Phase 15. Default **text** ≡ 1.4.0 (still writes `-o` payload; almost no stderr summary on success). **json**: one object on stdout; still writes `-o`; no text dual-write; exit format-independent. `bytes` = written bytes (`index.total_size`). Orthogonal to `--cache` / `--cache-max-bytes` / `--jobs`. Failure paths do not require a full JSON object. Field set **frozen** for scripts. |

## Conventions

| Rule | Detail |
|---|---|
| Default format | **`text`** for every command above |
| JSON shape | Single compact **object** on stdout |
| Exit vs format | Exit code does **not** change with `--format` |
| Breaking | Renaming any field in this matrix → **breaking** (major) |
| Path filter | `extract` / `pull` / `push` `--path`/`--exclude` do **not** rename existing JSON fields; pull/push `unique_chunks` = post-filter set |
| Cache soft budget | **`--cache-max-bytes N`** (with `--cache` on `cat`/`verify`/`extract`/`mount`) = **refuse-fill** when `bytes_on_disk + plaintext_len > N`; still serves primary. **≠ LRU ≠ trim ≠ GC ≠ sync**. Accepts plain decimal **or** human suffixes `K`/`M`/`G`/`Ki`/`Mi`/`Gi` (1024-base; Phase 16). Omit ≡ 1.4 unbounded fill. See [mount.md](mount.md). Smoke: [`scripts/demo_cache_budget_ops_json.sh`](../scripts/demo_cache_budget_ops_json.sh). |
| Read-path `--fallback` | Repeatable on `cat`/`verify`/`extract`/`mount`/`pull`/`doctor`. Missing-only failover; **≠ cache ≠ sync**. Outer Cache wraps the whole Fallback chain. Zero times ≡ 1.5 single origin. Smoke: [`scripts/demo_fallback_bytes_suffix.sh`](../scripts/demo_fallback_bytes_suffix.sh). |

## Responsibility split (no remote scrub)

| Command | Answers |
|---|---|
| `verify` | Listing structure + **referenced** chunk integrity (incl. HTTP `verify_hash`) |
| `doctor` | Are **referenced** chunks **present**? (`has`, optional `--deep` = `get`) |
| `gc` | Which **local** loose chunks are **unreferenced**? (dry-run / `--apply`) |
| `store scrub` | Are **local** loose `.cnk` objects bit-rot free? (full BLAKE3; optional `--listing` scopes refs; not remote) |
| `store stats` / `du` | How many loose chunks / how many **on-disk** bytes? Optional `bytes_plaintext` (none ≡ on_disk; zstd needs `--decode`). Observation only — not trim/LRU. |

There is **no** remote-scrub first-class command. For listing-referenced remote
integrity use **`verify --source`**; for presence use **`doctor`**.

## Read-path `--fallback` (Phase 16 / 1.6.0)

Repeatable **`--fallback <PATH|URL>`** on read commands (`cat` / `verify` /
`extract` / `mount` / `pull` / `doctor`) appends ordered extra origins behind
`--store` / `--source`. Default (flag omitted zero times) ≡ **1.5.0** single
origin. Failover is **Missing-only** (`SourceError::NotFound`); Transient /
Permanent / Corrupt fail fast (no silent switch).

| Rule | Detail |
|---|---|
| Composition | Recommended: **outer Cache wraps the whole Fallback chain** — `CacheSource(Fallback([primary, …fallbacks]), cache)`. One fill budget, one refuse-fill policy |
| `fallback` ≠ `cache` | `--fallback` is **read-only multi-origin**; never writes any origin. `--cache` **writes** the cache store on miss (optional `--cache-max-bytes` refuse-fill) |
| `fallback` ≠ sync / prune | Not bidirectional sync, not watch, not prune / `--delete`, not write-back to primary |
| Push dest | **`--fallback` does not apply** to `push --dest` (write stays single dest) |
| HTTP flags | Templates / retries / SigV4 apply **isomorphically** to each HTTP origin in the chain (primary and HTTP fallbacks) |
| Orthogonal | `--jobs` / `--format` / `--progress` / path filter / cache-max |

Smoke: [`scripts/demo_fallback_bytes_suffix.sh`](../scripts/demo_fallback_bytes_suffix.sh).
See [mount.md](mount.md), [extract.md](extract.md), [pull.md](pull.md).

## Push path filter (Phase 14)

`--path` / `--exclude` / `--exclude-from` on **`push`** (and the same flags on
`pull` / `archive` / `extract`) are **opt-in**. Default (no flags) ≡ **1.3.0**
full tree / full reference set.

| Rule | Detail |
|---|---|
| JSON fields | **Unchanged** names for prior ops commands; Phase 14 added **`store stats`**; Phase 15 adds **`make`/`cat`** only |
| `unique_chunks` | = **post-filter** unique id count (push ≡ pull); field name stable |
| `path` ≠ listing upload | Push still uploads **chunks only**; `.cfdir` / `.cfidx` stay local / out-of-band |
| `path` ≠ sync / ≠ prune | Does **not** delete remote extras; extract path does **not** delete dest extras |
| `.cfidx` + path flags | Clear **non-zero** error (no silent full upload) |

See [push.md](push.md). Smoke: [`scripts/demo_push_path_store_stats.sh`](../scripts/demo_push_path_store_stats.sh).

## Compat gate

Flag presence for ops JSON / 1.1+ CLIs is gated by
[`scripts/check_compat_1_1.sh`](../scripts/check_compat_1_1.sh) (calls
`check_compat_1_0.sh`; no absolute perf SLA). Phase 13 path / `archive --format`
flags are gated by **`check_compat_1_2.sh`** (calls 1_1; no absolute perf SLA).
Phase 14 (`store stats`, `push --path`/`--exclude`/`--exclude-from`) is gated
by **`check_compat_1_3.sh`** (calls 1_2; no absolute perf SLA).
Phase 15 finalizes **`make`/`cat`** in this matrix and soft cache budget
(`--cache-max-bytes`); gated by **`check_compat_1_4.sh`** (calls 1_3; no
absolute perf SLA).
Smoke: [`scripts/demo_cache_budget_ops_json.sh`](../scripts/demo_cache_budget_ops_json.sh).
Phase 16 / **1.6.0** adds `--fallback`, human byte suffixes on
`--cache-max-bytes`, and `store stats` `bytes_plaintext` / `--decode` (all
opt-in; defaults ≡ 1.5). Gated by **`check_compat_1_5.sh`** (calls 1_4; no
absolute perf SLA). Workspace reports **1.6.0**. Smoke:
[`scripts/demo_fallback_bytes_suffix.sh`](../scripts/demo_fallback_bytes_suffix.sh).

## `--progress` ↔ JSON orthogonality (Phase 17 / 1.7)

Opt-in **`--progress`** (default **off** ≡ 1.6) on long ops emits
`progress: op=… done=N/TOTAL` lines on **stderr** only. It does **not** add,
rename, or remove any JSON field in this matrix. With **`--format json`**,
the single JSON object still goes to **stdout**; progress noise stays on
stderr. Exit codes remain format-independent.

Coverage today: `push` / `pull` / `store scrub` / `gc --apply` (Phase 12+) plus
**`archive` / `extract` / `make`** (Phase 17). Default off ≡ prior minor.

## Store compression narrative (Phase 17 / 1.7)

CLI **`--compression none|zstd`** (on **`make`** / **`archive`**) applies only
when **creating** a new local store (`meta.toml` absent). Omit ≡ **`none`**
(≡ 1.6). Existing stores open by `meta.toml`; an explicit flag that conflicts
with meta → clear non-zero error.

| Rule | Detail |
|---|---|
| Ops JSON shape | **`--compression` does not change** archive / make / extract / push / pull / … summary field **names or shapes** in this matrix |
| `store stats` `compression` | Reflects store **meta** (`"none"` \| `"zstd"`); observation only |
| `bytes_plaintext` | Unchanged contract: none ⇒ equals `bytes_on_disk`; zstd ⇒ `null` unless opt-in **`--decode`** |
| Disk ≠ wire ≠ pack | On-disk zstd encoding is **not** HTTP Content-Encoding / wire compression and **not** packfile; `ChunkSource::get` / HTTP PUT body stay **plaintext** |

Smoke: [`scripts/demo_zstd_progress.sh`](../scripts/demo_zstd_progress.sh).
