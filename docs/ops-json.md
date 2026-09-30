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
[store.md](store.md) (`store create` / `store list` / `store get`), [diff.md](diff.md), [archive.md](archive.md),
[extract.md](extract.md), [push.md](push.md), [pull.md](pull.md),
[filter.md](filter.md) (`filter`), [ls.md](ls.md) (`ls` / `cat --path`).

## Matrix (command × minimum stable fields)

| Command | Minimum stable fields | Notes |
|---|---|---|
| **`archive`** (write) | `ok`, `dry_run` (`false`), `files`, `dirs`, `chunks`, `written`, `reused`, `seed_reused_files`, `rechunked_files`, `skipped_symlinks`, `skipped_special`, `excluded`; **additive Phase22:** `recorded_symlinks` | Phase 13 §3.3 / M5 + Phase22. Default **text** ≡ 1.2.0. `excluded` = path-filter rejects (0 when no filter). Empty Dir entries omitted by default → `dirs` usually 0 (≡ 1.14); **`--empty-dirs`** can make `dirs` > 0. `recorded_symlinks` = Symlink entries written under `--symlinks record` (0 under default skip ≡ 1.11). |
| **`archive`** (dry-run) | `ok`, `dry_run` (`true`), `files`, `dirs`, `chunks`, `would_write`, `would_reuse`, `seed_reused_files`, `rechunked_files`, `skipped_symlinks`, `skipped_special`, `excluded`; **additive Phase22:** `recorded_symlinks` | Dry-run uses **`would_write` / `would_reuse`** (not `written` / `reused`). Other counters present in both modes (incl. `recorded_symlinks`). |
| **`diff`** | `added`, `removed`, `changed`, `meta_changed` (string arrays of paths); `chunks_shared`, `chunks_only_left`, `chunks_only_right` (numbers) | No top-level `ok`. Differences → exit **1** (like `diff(1)`). `--max-paths` does **not** truncate JSON arrays. Opt-in **`--progress`** (Phase19-M3) → **stderr only** (`progress: op=diff done=N/TOTAL`; TOTAL = filtered File+Symlink path union); orthogonal to `--format json` (progress never enters the JSON object). Default **off** ≡ 1.8. |
| **`verify`** | `ok` (bool); `kind` (`"cfidx"` \| `"cfdir"`); **cfidx:** `bytes`, `chunks`; **cfdir:** `files`, `chunks`; **additive Phase22 cfdir:** `symlinks`; **additive when `--cache`:** `cache_hits`, `cache_miss_fills`, `cache_miss_refused` | Success-only object shown here; failure paths bail before JSON. Without `--cache`, the three `cache_*` keys are **omitted** (1.7 baseline shape). With `--cache`, present even if `--cache-stats` is off. **≠** LRU. Orthogonal to **`--progress`** (stderr only; Phase18-M5). Phase 20 path filter (`--path`/`--path-from`/`--exclude`/`--exclude-from`): **field names unchanged**; `files`/`chunks` **may shrink** under a filter; no flags ≡ 1.9 full tree; `.cfidx` + any path flag → non-zero (before JSON). Phase22: cfdir JSON includes additive `symlinks` count (0 when none). |
| **`doctor`** | `ok`, `listings`, `checked`, `missing`, `deep`, `retries`; **additive when `--cache`:** `cache_hits`, `cache_miss_fills`, `cache_miss_refused` | When complete: `missing` is **`0`** (number). When gaps: `missing` is a **string array** of hex ids (not also printed as bare lines). Without `--cache`, omit `cache_*`. Counters follow `CacheSource` get-path observation (`--deep` exercises `get`; default `has` may leave zeros). **≠** LRU. Phase 20 path filter: **field names unchanged**; `checked`/`missing` **may shrink** under a filter; no flags ≡ 1.9 full set; `.cfidx` + any path flag → non-zero. **≠** `gc --path`. |
| **`extract`** (write) | `ok`, `dry_run` (`false`), `skipped`, `wrote`, `dirs`; **additive Phase22:** `wrote_symlinks`, `symlinks` (same count); **additive when `--cache`:** `cache_hits`, `cache_miss_fills`, `cache_miss_refused` | Always emits `skipped`/`wrote`/`dirs` (`skipped=0` when `--skip-unchanged` off). Phase22 additive symlink counts (0 when listing has no Symlink). Path filter does **not** rename fields; counts reflect the filtered set. Without `--cache`, omit `cache_*`. |
| **`extract`** (dry-run) | `ok`, `dry_run` (`true`), `would_skip`, `would_write`, `would_dirs`, `would_fail`; **additive Phase23-M6:** `would_symlinks` (**always** present; **0** when listing has no Symlink / none would write) | No chunk gets; exit **0** when listing is valid even if `would_fail > 0`. Dry-run does **not** open `--cache`, so `cache_*` keys stay **omitted** even if `--cache` was passed (no `CacheStatsRef`). `would_write` still **includes** Symlink would-writes (≡ 1.12); `would_symlinks` is an extra counter for symmetry with write-path `wrote_symlinks` (absolute/empty/conflict → `would_fail`, not `would_symlinks`). **`would_symlinks` ≠ prune ≠ sync ≠ pack ≠ write mount**. |
| **`push`** | `ok`, `skipped`, `uploaded`, `failed`, `failed_transient`, `failed_permanent`, `retries`, `unique_chunks`, `listings`, `dry_run` | JSON emitted before non-zero exit on `failed > 0`. With `--path`/`--exclude`, **`unique_chunks` = filtered** unique id count (field name unchanged; ≡ pull). |
| **`pull`** | `ok`, `skipped`, `fetched`, `failed`, `failed_transient`, `failed_permanent`, `retries`, `unique_chunks`, `listings`, `dry_run`; **additive when `--cache`:** `cache_hits`, `cache_miss_fills`, `cache_miss_refused` | Same shape as push with `fetched` instead of `uploaded`. With `--path`/`--exclude`, **`unique_chunks` = filtered** unique id count (field name unchanged). Without `--cache`, omit `cache_*`. Opt-in **`--verify`** (Phase18-M1) runs post-pull listing verify against `--store` on **stderr** only — it does **not** add/rename JSON fields (orthogonal; dry-run/failed pull skips verify). Opt-in **`--compression none|zstd`** (Phase19-M2): create-time only for a **new** `--store` (omit ≡ **none** ≡ 1.8; existing store opens by meta / explicit conflict → non-zero; dry-run never creates). Does **not** rename JSON fields. |
| **`gc`** | `ok`, `dry_run`, `applied`, `listings`, `referenced`, `unreferenced`, `deleted` | Phase 12 §3.2. `unreferenced` = candidate count this run; `deleted` = actual deletes (**0** on dry-run). Both always present. No path listing on json. `--jobs` orthogonal. |
| **`store scrub`** | `ok`, `checked`, `ok_count`, `corrupt`, `unreadable`, `corrupt_ids`, `unreadable_ids` | Phase 12 §3.2. `checked` = total scanned; `ok_count`/`corrupt`/`unreadable` partition; bad ids **only** in arrays. `ok` true iff corrupt+unreadable==0. `--jobs` orthogonal. Phase 15 P1 `--listing` scopes `checked` to listing refs (field names unchanged; **not** remote scrub). |
| **`store stats`** (alias **`du`**) | `ok`, `chunks`, `bytes_on_disk`, `bytes_plaintext` (`number` \| `null`); optional `compression` (`"none"` \| `"zstd"`) | Phase 14 M2 + Phase 16 M4. Read-only; `chunks` = `list_chunk_ids` count; `bytes_on_disk` = sum of `.cnk` `metadata().len()` (no plaintext decode). `bytes_plaintext`: `compression=none` → equals `bytes_on_disk` (cheap); `compression=zstd` → `null` unless opt-in **`--decode`** (full-store `get` sum). Default **text**: `store stats: chunks=N bytes_on_disk=M [bytes_plaintext=P] compression=…` (`bytes_plaintext` printed only when known). **json**: one object; `bytes_plaintext` number or `null`; no text dual-write. Old field names unchanged. **Not** GC / scrub / trim / LRU. |
| **`store create`** | `ok`, `store`, `compression` (`"none"` \| `"zstd"`) | Phase 19 M1. Creates empty local CAS (`Store::create`); default **text**. **json**: one object on stdout; no text dual-write; exit format-independent. Field set **orthogonal** to other store / ops commands (does **not** rename prior fields). Omit `--compression` ≡ `"none"` ≡ 1.8 create default. Existing `meta.toml` → clear non-zero (**≠** recompress / trim). See [store.md](store.md). |
| **`store list`** | `ok`, `chunks`, `ids` (sorted string array of hex ids) | Phase 21 M6 / P1 O2. Read-only via `Store::list_chunk_ids`; default **text**: one hex id per line (**stably sorted**); empty → no lines. **json**: one object; no text dual-write; exit format-independent. Field set **orthogonal** (does **not** rename prior fields). **≠** GC / scrub / trim / LRU. See [store.md](store.md). |
| **`chunk-id`** | `ok`, `chunks`[{`offset`,`length`,`id`}] | Phase25-M6 / P1 O2. Default **text**: `offset\tlength\tid` lines. **json**: one object; no text dual-write. No store write. |
| **`store has`** | `ok`, `present`, `id` | Phase25-M6 / P1 O2. Default **text**: `present\t<id>` (exit 0) / missing → non-zero. **json**: one object; missing → `ok=false`, `present=false`, exit non-zero. |
| **`store get`** | `ok`, `id`, `bytes` | Phase26-M4 (toward **1.16.0**; workspace still **1.15.0**). New command field set; **additive** — does not rename prior fields. Local store: `--store` + hex id + required **`-o`**. Default trusts on-disk encoding (`get_verify(..., false)`). Optional **`--verify`** re-hashes; **does not** add or rename JSON fields. Default **text**: stderr `store get: ok id=<hex> bytes=N` and writes `-o`. **json**: one object **`{ok,id,bytes}`** on stdout; still writes `-o`; no text dual-write; exit format-independent. `bytes` = plaintext length written. Missing id / bad hex → clear non-zero (no success object). **`store get` ≠ scrub ≠ cat ≠ extract ≠ recompress ≠ remove**. See [store.md](store.md). |
| **`make`** (write) | `ok`, `bytes`, `chunks`, `new`, `reused`; **additive with `--seed`:** `seed_reused` (`bool`) | Phase 15 + Phase24-M6. Default **text** ≡ 1.4.0 stderr `make: wrote … (BYTES bytes, N chunk(s); new=X, reused=Y)`. **json**: one object on stdout; no text dual-write; exit format-independent. `bytes` = input size; `chunks` = chunk count; `new`/`reused` align stderr (`PutOutcome::Written` / `SkippedExists`). Omit `--seed` ⇒ field set unchanged (no `seed_reused`; ≡ 1.13). With **`--seed`**: additive `seed_reused` (`true` = copied prior chunk table / skipped FastCDC; `false` = rechunked). Reuse requires prior chunks `has()` in store else clear non-zero. **≠** pack / **≠** recompress / **≠** path. |
| **`make`** (dry-run) | `ok`, `dry_run` (`true`), `bytes`, `chunks`, `would_write`, `would_reuse`; **additive with `--seed`:** `seed_reused` | Phase22-M7 / P1 + Phase24-M6. Plan-only: FastCDC (rechunk path) + `has()` accounting; no store create/put; no `.cfidx` write. Uses **`would_write` / `would_reuse`** (not `new` / `reused`). Missing store → all unique chunks `would_write` on rechunk; seed Reuse with missing chunks → clear non-zero. With `--seed`, additive `seed_reused`. Omit `--seed` ⇒ no `seed_reused` (≡ 1.13). **≠** pack / **≠** recompress / **≠** path. |
| **`cat`** | `ok`, `bytes`; **additive when `--cache`:** `cache_hits`, `cache_miss_fills`, `cache_miss_refused` | Phase 15 + Phase18-M3 + Phase25 `cat --path`. Default **text** ≡ 1.4.0 (still writes `-o` payload; almost no stderr summary on success). **json**: one object on stdout; still writes `-o`; no text dual-write; exit format-independent. `bytes` = written bytes (`.cfidx` = `index.total_size`; `.cfdir --path` = matched File size). Without `--cache`, omit `cache_*` (1.7 baseline). With `--cache`, three numeric fields from the same `CacheStatsRef` (even if `--cache-stats` off). Orthogonal to `--cache-max-bytes` / `--jobs` / `--cache-stats` / `--progress` / `--path`. Failure paths do not require a full JSON object. Prior field names **frozen** (`.cfdir --path` does **not** rename). **`cat --path` ≠ extract ≠ prune ≠ sync**. See [ls.md](ls.md). |
| **`filter`** | `ok`, `dry_run`, `input`, `output`, `files`, `dirs`, `symlinks`, `excluded` | Phase24 / **1.14.0** (new command field set; **additive** — does not rename prior fields). Default **text** ≡ stderr summary. **json**: one object on stdout; no text dual-write; exit format-independent. `dry_run` true under `--dry-run` (no `-o` write). `excluded` = input File+Symlink leaves that failed PathFilter (Dirs that drop as non-ancestors not counted — same leaf accounting as archive). Empty path 四件套 ⇒ `excluded=0` (identity). **`filter` ≠ prune ≠ `gc --path` ≠ sync ≠ write mount ≠ pack ≠ `archive --path`**. See [filter.md](filter.md). |
| **`ls`** | `ok`, `entries` (array of `{kind, path, size?, target?, chunks?}`) | Phase25 / **1.15.0** (new command field set; **additive** — does not rename prior fields). Default **text** ≡ tab columns on stdout. **json**: one object on stdout; no text dual-write; exit format-independent. Each entry: `kind` = `"file"`\|`"dir"`\|`"symlink"`; `path` always; `size` on File; `target` on Symlink; `chunks` (hex string array) only when `--chunks` and kind=file. **Never** opens a store. **`ls` ≠ mount ≠ extract ≠ verify ≠ pack ≠ filter**. See [ls.md](ls.md). |

## Conventions

| Rule | Detail |
|---|---|
| Default format | **`text`** for every command above |
| JSON shape | Single compact **object** on stdout |
| Exit vs format | Exit code does **not** change with `--format` |
| Breaking | Renaming any field in this matrix → **breaking** (major) |
| Path filter | `extract` / `pull` / `push` / `archive` / `diff` / `doctor` / `verify` `--path`/`--path-from`/`--exclude`/`--exclude-from` do **not** rename existing JSON fields; pull/push `unique_chunks` = post-filter set; doctor `checked` / verify `files`/`chunks` may shrink. **`path-from` ≠ prune ≠ gc-path ≠ sync ≠ pack**. **`mount` path** (Phase21) is the same PathFilter but **outside** this JSON matrix (no mount JSON). **`mount path` ≠ write mount ≠ prune ≠ gc-path ≠ sync ≠ pack**. **`filter`** (Phase24) is a **new** command with its own field set (`ok`/`dry_run`/`input`/`output`/`files`/`dirs`/`symlinks`/`excluded`); same PathFilter semantics; **`filter` ≠ prune ≠ `gc --path` ≠ sync ≠ write mount ≠ pack ≠ `archive --path`**. **`ls`** (Phase25) is a **new** command with field set `ok`/`entries`[{`kind`,`path`,`size?`,`target?`,`chunks?`}]; same PathFilter on `.cfdir`; **`ls` ≠ mount ≠ extract ≠ verify ≠ pack ≠ filter**. **`cat --path`** (Phase25) does **not** rename `ok`/`bytes`. Phase26 leaf-Dir keep does **not** rename `ls` / `filter` fields; path-scoped `entries` / `dirs` **may include** an explicit Dir the filter allows (only when that Dir was already in the listing). **`filter_dir_archive` leaf-Dir ≠ prune ≠ `gc --path`**. **`store get`** (Phase26) is a **new** command with field set **`{ok,id,bytes}`**; **`store get` ≠ scrub ≠ cat ≠ extract ≠ recompress** |
| Cache soft budget | **`--cache-max-bytes N`** (with `--cache` on `cat`/`verify`/`extract`/`mount`) = **refuse-fill** when `bytes_on_disk + plaintext_len > N`; still serves primary. **≠ LRU ≠ trim ≠ GC ≠ sync**. Accepts plain decimal **or** human suffixes `K`/`M`/`G`/`Ki`/`Mi`/`Gi` (1024-base; Phase 16). Omit ≡ 1.4 unbounded fill. See [mount.md](mount.md). Smoke: [`scripts/demo_cache_budget_ops_json.sh`](../scripts/demo_cache_budget_ops_json.sh). |
| Cache observation JSON | Phase18-M3 / G3: on `cat` / `verify` / `extract` (write) / `pull` / `doctor`, **`--cache` + `--format json`** adds `cache_hits` / `cache_miss_fills` / `cache_miss_refused` (numbers from the same `CacheStatsRef` as `--cache-stats`). **No `--cache` → omit** the three keys (do not emit `null`). Orthogonal to `--cache-stats` (stderr) and `--progress` (stderr). **≠ LRU ≠ trim**. `mount` has no `--format json` (stderr stats only, M2). |
| Read-path `--fallback` | Repeatable on `cat`/`verify`/`extract`/`mount`/`pull`/`doctor`. Missing-only failover; **≠ cache ≠ sync**. Outer Cache wraps the whole Fallback chain. Zero times ≡ 1.5 single origin. Smoke: [`scripts/demo_fallback_bytes_suffix.sh`](../scripts/demo_fallback_bytes_suffix.sh). |

## Responsibility split (no remote scrub)

| Command | Answers |
|---|---|
| `verify` | Listing structure + **referenced** chunk integrity (incl. HTTP `verify_hash`) |
| `doctor` | Are **referenced** chunks **present**? (`has`, optional `--deep` = `get`) |
| `gc` | Which **local** loose chunks are **unreferenced**? (dry-run / `--apply`) |
| `store scrub` | Are **local** loose `.cnk` objects bit-rot free? (full BLAKE3; optional `--listing` scopes refs; not remote) |
| `store stats` / `du` | How many loose chunks / how many **on-disk** bytes? Optional `bytes_plaintext` (none ≡ on_disk; zstd needs `--decode`). Observation only — not trim/LRU. |
| `store create` | Create an **empty** local CAS (`meta.toml` + `chunks/`). Opt-in `--compression`; omit ≡ none. **≠** recompress / trim / pack. Existing store → non-zero. |
| `store list` | Which loose chunk **ids** are present? (sorted hex; observation only — **≠** GC / scrub / trim / LRU) |
| `store get` | Plaintext of **one** local chunk id (`-o`; json `{ok,id,bytes}`). **≠** scrub / **≠** cat / **≠** extract / **≠** recompress / **≠** remove |
| `filter` | Persist a path-scoped **subset listing** from an existing `.cfdir` (no store / no source-tree walk). **≠** prune / **≠** `gc --path` / **≠** sync / **≠** write mount / **≠** pack / **≠** `archive --path` |
| `ls` | Which paths are in this listing? (File/Dir/Symlink inventory; optional `--chunks` ids from decode only; **no store**). **≠** mount / **≠** extract / **≠** verify / **≠** pack / **≠** filter |
| `cat --path` | Reassemble **one** `.cfdir` File to `-o` (field names `ok`/`bytes` frozen). **≠** extract whole tree / **≠** prune / **≠** sync |

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
Phase 17 / **1.7.0** adds create-time `--compression` and
`archive`/`extract`/`make --progress` (all opt-in; defaults ≡ 1.6; see
sections below). Gated by **`check_compat_1_6.sh`** (calls 1_5; no absolute
perf SLA). Workspace reports **1.7.0**. Smoke:
[`scripts/demo_zstd_progress.sh`](../scripts/demo_zstd_progress.sh).
Phase 18 / **1.8.0** adds `pull --verify`, `--cache-stats` / ops-json
`cache_*`, and `cat`/`verify --progress` (all opt-in; defaults ≡ 1.7; see
sections below). Gated by **`check_compat_1_7.sh`** (calls 1_6; no absolute
perf SLA). Workspace reports **1.8.0**. Smoke:
[`scripts/demo_pull_verify_cache_stats.sh`](../scripts/demo_pull_verify_cache_stats.sh).
Phase 19 / **1.9.0** adds **`store create`**, **`pull --compression`**
(create-time only; omit ≡ none ≡ 1.8), **`diff --progress`**, and P1 honest
**`make --jobs`** (all opt-in; defaults ≡ 1.8; see sections below). Gated by
**`check_compat_1_8.sh`** (calls 1_7; no absolute perf SLA). Workspace
reports **1.9.0**. Smoke:
[`scripts/demo_store_create_pull_compression.sh`](../scripts/demo_store_create_pull_compression.sh).
Phase 20 / **1.10.0** adds **`--path-from`** on
archive/extract/push/pull/diff/doctor/verify + **`doctor`/`verify` path
scope** (+ P1 push local/`file://` dest). Defaults stay ≡ **1.9.0** (no path
flags ⇒ full set). JSON field **names** unchanged; filtered counts may shrink.
Gated by **`check_compat_1_9.sh`** (calls 1_8; no absolute perf SLA). Workspace
reports **1.10.0**. Smoke:
[`scripts/demo_path_from_doctor_verify.sh`](../scripts/demo_path_from_doctor_verify.sh).
Phase 21 / **1.11.0** adds **`mount` path** quartet + P1 **`store list`**
JSON (`ok` / `chunks` / `ids`) + P1 **`push --compression`** (create-time;
omit ≡ none; no new push JSON fields). **`mount` still has no `--format
json`** (session-typed FUSE; no natural ops-json object / no mount
`--progress` done/TOTAL). Gated by **`check_compat_1_10.sh`** (calls 1_9; no
absolute perf SLA). Workspace reports **1.11.0**. Smoke:
[`scripts/demo_mount_path.sh`](../scripts/demo_mount_path.sh).
Phase 22 / **1.12.0** adds additive archive `recorded_symlinks`, extract
`wrote_symlinks`/`symlinks`, verify cfdir `symlinks`, and **`make` dry-run**
`dry_run`/`would_write`/`would_reuse` (write path `new`/`reused` unchanged).
Default skip path field names unchanged. Gated by **`check_compat_1_11.sh`**
(calls 1_10; no absolute perf SLA). Workspace reports **1.12.0**. Smoke:
[`scripts/demo_symlink.sh`](../scripts/demo_symlink.sh).
Phase 23 / **1.13.0** adds extract dry-run **`would_symlinks`** (always
emitted, incl. 0; P1 / M6). `would_write` semantics unchanged (still includes
symlink would-writes ≡ 1.12). Gated by **`check_compat_1_12.sh`** (calls
1_11; no absolute perf SLA). Workspace reports **1.13.0**. Smoke:
[`scripts/demo_diff_tree_symlink.sh`](../scripts/demo_diff_tree_symlink.sh).
**`would_symlinks` ≠ prune ≠ sync ≠ pack ≠ write mount**.
Phase 24 / **1.14.0** adds **`filter`** (new command field set:
`ok`/`dry_run`/`input`/`output`/`files`/`dirs`/`symlinks`/`excluded`) and
make `--seed` additive **`seed_reused`**. Gated by **`check_compat_1_13.sh`**
(calls 1_12; no absolute perf SLA). Workspace reports **1.14.0**. Smoke:
[`scripts/demo_filter_listing.sh`](../scripts/demo_filter_listing.sh).
**`filter` ≠ prune ≠ `gc --path` ≠ sync ≠ write mount ≠ pack ≠ `archive --path`**.
**`make --seed` ≠ pack ≠ recompress ≠ path**.
Phase 25 / **1.15.0** adds **`ls`** (new command field set:
`ok`/`entries`[{`kind`,`path`,`size?`,`target?`,`chunks?`}]) and documents
**`cat --path`** (same `ok`/`bytes` names; `.cfdir` single File), plus thin
O2 **`chunk-id`/`store has --format json`**. Gated by
**[`check_compat_1_14.sh`](../scripts/check_compat_1_14.sh)** (calls 1_13; no
absolute perf SLA). Workspace reports **1.15.0**. Smoke:
[`scripts/demo_ls_cat_path.sh`](../scripts/demo_ls_cat_path.sh).
**`ls` ≠ mount ≠ extract ≠ verify ≠ pack ≠ filter**.
**`cat --path` ≠ extract ≠ prune ≠ sync**.
Phase 26 (toward **1.16.0**; workspace still reports **1.15.0**) pins
**`store get`** JSON **`{ok,id,bytes}`**. `ls` / `filter` field names stay
frozen; a path-scoped listing may list an extra explicit Dir when that Dir
was already stored and the filter allows it (**leaf-Dir**).
**`check_compat_1_15.sh` is not this milestone** (M5). Smoke:
[`scripts/demo_empty_dir_path_store_get.sh`](../scripts/demo_empty_dir_path_store_get.sh).
**`filter_dir_archive` leaf-Dir ≠ prune ≠ `gc --path`**.
**`store get` ≠ scrub ≠ cat ≠ extract ≠ recompress**.



## Cache observation JSON (Phase18-M3 / 1.8 opt-in)

When a read command opens **`--cache`** and emits **`--format json`**, the single
stdout object gains three **additive** numeric fields from the process-local
`CacheSource` counters (same handle as **`--cache-stats`**):

| Field | Meaning |
|---|---|
| `cache_hits` | `get` served from cache |
| `cache_miss_fills` | miss that filled the cache store |
| `cache_miss_refused` | miss that served primary but **refused** cache fill (soft budget) |

Rules:

| Rule | Detail |
|---|---|
| Trigger | **`--cache` + `--format json`** (not gated on `--cache-stats`) |
| No `--cache` | **Omit** the three keys — keep the 1.7 baseline object shape (no `null` pollution) |
| Old fields | **Never** renamed or removed |
| `--cache-stats` | stderr `cache: hits=…` line; **orthogonal** to JSON fields |
| `--progress` | stderr only; **orthogonal** to JSON fields |
| Soft budget | Counters observe refuse-fill; they are **not** an LRU / trim / eviction API |
| Coverage | `cat` / `verify` / `extract` (write path) / `pull` / `doctor`. **`mount` has no ops-json** (session-typed; Phase21 path flags do not add JSON) |

## `--progress` ↔ JSON orthogonality (Phase 17 / 1.7.0 + Phase18)

Opt-in **`--progress`** (default **off** ≡ prior minor) on long ops emits
`progress: op=… done=N/TOTAL` lines on **stderr** only. It does **not** add,
rename, or remove any JSON field in this matrix. With **`--format json`**,
the single JSON object still goes to **stdout**; progress noise stays on
stderr. Exit codes remain format-independent. Orthogonal to `--jobs` /
`--cache` / `--fallback` / `--cache-stats`.

Coverage: `push` / `pull` / `store scrub` / `gc --apply` (Phase 12+) plus
**`archive` / `extract` / `make`** (Phase 17) plus **`cat` / `verify`**
(Phase18-M4/M5; per listing chunk) plus P1 **`doctor`** (Phase18) plus
**`diff`** (Phase19-M3; filtered File+Symlink path union; Phase23 honesty). Default off ≡ prior minor.

## Store compression narrative (Phase 17 / 1.7.0 + Phase 19)

CLI **`--compression none|zstd`** (on **`make`** / **`archive`** / **`store create`**
/ **`pull`**) applies only when **creating** a new local store (`meta.toml`
absent). Omit ≡ **`none`** (≡ 1.6 / 1.8 create default). Existing stores open
by `meta.toml`; an explicit flag that conflicts with meta → clear non-zero
error. **`store create` ≠ recompress ≠ trim**; repeating create on an existing
path is non-zero.

| Rule | Detail |
|---|---|
| Ops JSON shape | **`--compression` does not change** archive / make / extract / push / pull / … summary field **names or shapes** in this matrix |
| `store create` JSON | Additive command row: `ok` / `store` / `compression` only; orthogonal to other commands |
| `store stats` `compression` | Reflects store **meta** (`"none"` \| `"zstd"`); observation only |
| `bytes_plaintext` | Unchanged contract: none ⇒ equals `bytes_on_disk`; zstd ⇒ `null` unless opt-in **`--decode`** |
| Disk ≠ wire ≠ pack | On-disk zstd encoding is **not** HTTP Content-Encoding / wire compression and **not** packfile; `ChunkSource::get` / HTTP PUT body stay **plaintext** |

Smoke: [`scripts/demo_zstd_progress.sh`](../scripts/demo_zstd_progress.sh);
Phase 19: [`scripts/demo_store_create_pull_compression.sh`](../scripts/demo_store_create_pull_compression.sh).
