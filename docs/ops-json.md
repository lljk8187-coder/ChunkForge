# Ops JSON field matrix

Stable fields for CLI commands that support **`--format text|json`**.
Default is always **`text`** (human stderr/stdout summaries ≡ prior minor).
With **`--format json`**, each command emits **one JSON object on stdout**
(no duplicate text summary). Exit codes are **format-independent**.

**Field rename → breaking** (requires a **major** bump, or an explicit breaking
note). See [stability.md](stability.md) Breaking-change policy. Additive keys
in a minor are allowed when defaults stay compatible; this matrix lists the
**minimum stable set** scripts may rely on.

Cross-links: [doctor-gc.md](doctor-gc.md) (`gc` / `store scrub` / `doctor`),
[diff.md](diff.md), [extract.md](extract.md), [push.md](push.md),
[pull.md](pull.md).

## Matrix (command × minimum stable fields)

| Command | Minimum stable fields | Notes |
|---|---|---|
| **`diff`** | `added`, `removed`, `changed`, `meta_changed` (string arrays of paths); `chunks_shared`, `chunks_only_left`, `chunks_only_right` (numbers) | No top-level `ok`. Differences → exit **1** (like `diff(1)`). `--max-paths` does **not** truncate JSON arrays. |
| **`verify`** | `ok` (bool); `kind` (`"cfidx"` \| `"cfdir"`); **cfidx:** `bytes`, `chunks`; **cfdir:** `files`, `chunks` | Success-only object shown here; failure paths bail before JSON. |
| **`doctor`** | `ok`, `listings`, `checked`, `missing`, `deep`, `retries` | When complete: `missing` is **`0`** (number). When gaps: `missing` is a **string array** of hex ids (not also printed as bare lines). |
| **`extract`** (write) | `ok`, `dry_run` (`false`), `skipped`, `wrote`, `dirs` | Always emits `skipped`/`wrote`/`dirs` (`skipped=0` when `--skip-unchanged` off). |
| **`extract`** (dry-run) | `ok`, `dry_run` (`true`), `would_skip`, `would_write`, `would_dirs`, `would_fail` | No chunk gets; exit **0** when listing is valid even if `would_fail > 0`. |
| **`push`** | `ok`, `skipped`, `uploaded`, `failed`, `failed_transient`, `failed_permanent`, `retries`, `unique_chunks`, `listings`, `dry_run` | JSON emitted before non-zero exit on `failed > 0`. |
| **`pull`** | `ok`, `skipped`, `fetched`, `failed`, `failed_transient`, `failed_permanent`, `retries`, `unique_chunks`, `listings`, `dry_run` | Same shape as push with `fetched` instead of `uploaded`. |
| **`gc`** | `ok`, `dry_run`, `applied`, `listings`, `referenced`, `unreferenced`, `deleted` | Phase 12 §3.2. `unreferenced` = candidate count this run; `deleted` = actual deletes (**0** on dry-run). Both always present. No path listing on json. `--jobs` orthogonal. |
| **`store scrub`** | `ok`, `checked`, `ok_count`, `corrupt`, `unreadable`, `corrupt_ids`, `unreadable_ids` | Phase 12 §3.2. `checked` = total scanned; `ok_count`/`corrupt`/`unreadable` partition; bad ids **only** in arrays. `ok` true iff corrupt+unreadable==0. `--jobs` orthogonal. |

## Conventions

| Rule | Detail |
|---|---|
| Default format | **`text`** for every command above |
| JSON shape | Single compact **object** on stdout |
| Exit vs format | Exit code does **not** change with `--format` |
| Breaking | Renaming any field in this matrix → **breaking** |
| Out of scope here | `archive` / `make` / `cat` have **no** `--format json` in 1.2 P0 |

## Responsibility split (no remote scrub)

| Command | Answers |
|---|---|
| `verify` | Listing structure + **referenced** chunk integrity (incl. HTTP `verify_hash`) |
| `doctor` | Are **referenced** chunks **present**? (`has`, optional `--deep` = `get`) |
| `gc` | Which **local** loose chunks are **unreferenced**? (dry-run / `--apply`) |
| `store scrub` | Are **local** loose `.cnk` objects bit-rot free? (full BLAKE3; no listing) |

There is **no** remote-scrub first-class command. For listing-referenced remote
integrity use **`verify --source`**; for presence use **`doctor`**.
