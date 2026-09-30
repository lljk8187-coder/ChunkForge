# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Phase 20 toward **1.10.0** (workspace / CLI still **1.9.0** until M7). Additive
opt-in only; defaults ≡ **1.9.0**.

### Added

- **`--path-from`** (Phase 20 M1–M2): repeatable UTF-8 include-prefix file
  (discipline ≡ `--exclude-from`; library `load_path_file`; merged OR with
  `--path`) on **archive / extract / push / pull / diff** (+ doctor/verify in
  M3). Missing / bad UTF-8 → clear non-zero. See `docs/archive.md` /
  `docs/pull.md` / `docs/push.md` / `docs/diff.md`.
- **`doctor` / `verify` path scope** (Phase 20 M3): `--path` / `--exclude` /
  `--exclude-from` / `--path-from`; File-only filtered refs; default no flags
  ≡ 1.9 full set; `.cfidx` + any path flag → non-zero; JSON field **names**
  unchanged (`checked` / `files` / `chunks` may shrink). See
  `docs/doctor-gc.md` / `docs/ops-json.md`.
- **Docs + `demo_path_from_doctor_verify.sh`** (Phase 20 M4): ops-json /
  stability / archive / pull / push / diff / doctor-gc / extract / perf /
  README Phase 20 narrative; local smoke for path-from archive, doctor/verify
  subset, quiet full default, exclude-from+path-from, missing file non-zero,
  `.cfidx`+path non-zero, `gc` has no `--path`.

### Pending (later Phase 20 milestones)

- **`check_compat_1_9.sh`** (M5) — not in tree yet at M4.
- Version bump **1.10.0** (M7).
- P1: `push` local/`file://` dest / docs brush (M6; optional).

### Not delivered / deferred (Phase 20)

- **packfile** / multi-chunk objects — still deferred (`docs/perf.md`)
- **Write mount** / COW / writable FUSE — non-goal
- Full **`aws-sdk-*`** / multipart / IMDS / SSO / ListObjects — non-goal
- **Remote scrub** / remote GC — deferred
- **Extract prune** / `--delete` — non-goal
- **`gc --path`** — **hard ban** (mis-delete risk)
- Bidirectional sync / watch dirs — non-goal
- Cache LRU / store trim / **default** zstd / HTTP wire compression /
  `store recompress` / `push --fallback` — non-goal

### Compatibility

- CLI defaults match **1.9.0**: no `--path-from` / no doctor·verify path flags
  ⇒ full set; create compression **none**; `jobs=1`, `http-retries=0`, SigV4
  **off**, ops default **text**, progress **off**, mount prefetch depth **1**
- `.cfidx` v1 / `.cfdir` v1 on-wire bytes unchanged
- Loose `chunks/<2hex>/<62hex>.cnk` layout unchanged
- Additive opt-in only; JSON field names unchanged
- **`path-from` ≠ prune ≠ gc-path ≠ sync ≠ pack**

## [1.9.0] — 2026-09-30

Phase 19 closeout — `store create`; `pull --compression` (create-time; omit ≡
none ≡ 1.8); `diff --progress` (default off); docs +
`demo_store_create_pull_compression.sh`; `check_compat_1_8.sh`; P1 honest
`make --jobs` (post-chunk put / on-disk encoding parallel; FastCDC stays
serial; default **1**). Defaults remain ≡ **1.8.0**. No pack / write mount /
aws-sdk / remote scrub / extract prune / bidirectional sync / push listing /
LRU / store trim / **default** zstd / HTTP wire compression / `store
recompress` / `push --fallback`.

### Added

- **`store create`** (Phase 19 M1): `chunkforge store create --store … [--compression none|zstd] [--format text|json]`; calls `Store::create`; existing → non-zero; omit ≡ none ≡ 1.8; json `ok` / `store` / `compression`. **≠** recompress / trim. See `docs/store.md`.
- **`pull --compression`** (Phase 19 M2): create-time only (same as make/archive/`store create`); omit ≡ none ≡ 1.8; existing by meta / conflict → non-zero; dry-run never creates; does **not** rename pull JSON fields. See `docs/pull.md`.
- **`diff --progress`** (Phase 19 M3): stderr `progress: op=diff done=N/TOTAL` (filtered File-path union); default off ≡ 1.8; orthogonal to `--format json`. See `docs/diff.md`.
- **Docs + `demo_store_create_pull_compression.sh`** (Phase 19 M4): ops-json / stability / pull / store / diff / perf / README Phase 19 narrative; local smoke for create → pull zstd, omit≡none, pull `--compression zstd`, `diff --progress`, quiet defaults, repeat-create non-zero.
- **`check_compat_1_8.sh`** (Phase 19 M5): calls `check_compat_1_7.sh` + asserts `store create` / pull `--compression` / diff `--progress`; thin non-goals (no prune / pack / LRU / aws-sdk / default zstd / recompress / push `--fallback`).
- **`make --jobs`** (Phase 19 M6 / P1 O1, path A): opt-in; default **1** ≡ 1.8 serial; FastCDC cut-points stay **serial**; after chunking, `put_with_id` runs via `parallel::map_indexed` (speeds store put / on-disk zstd encoding only — **not** parallel FastCDC). Help text is honest. See README Phase 19.
- **P1 docs brush** (Phase 19 M6 / O2): README / stability / remote-layout Phase 19·1.9 narrative; responsibility note **`store create` ≠ recompress ≠ default zstd ≠ pack**.
- Workspace version **1.9.0** (Phase 19 M7 closeout).

### Not delivered / deferred (Phase 19)

- **packfile** / multi-chunk objects — deferred (see `docs/perf.md`)
- **Write mount** / COW / writable FUSE — non-goal
- Full **`aws-sdk-*`** / multipart / IMDS / SSO / ListObjects — non-goal
- **Remote scrub** / remote GC — deferred (use `verify --source` / `doctor` /
  local `store scrub --listing`)
- **Extract prune** / `--delete` — non-goal
- Bidirectional sync / watch dirs — non-goal
- Byte-range HTTP resume / `push` listing upload — non-goal
- Cache LRU / store trim — non-goal
- **Default** store zstd / HTTP Content-Encoding / wire compression /
  `store recompress` — non-goal
- **`push --fallback`** / multi dest — non-goal (write side stays single dest)

### Compatibility

- CLI defaults match **1.8.0**: no `store create` side effects on old paths; no
  pull `--compression` ⇒ create **none**; no `diff --progress` ⇒ quiet;
  `make --jobs` default **1**; create compression **none**; no `--verify` on
  pull ⇒ quiet; no `--cache-stats` ⇒ no cache noise; no cat/verify/doctor
  `--progress` ⇒ quiet; no `--fallback` ⇒ single origin; no
  `--cache-max-bytes` ⇒ unbounded cache fill; plain integer cache-max still
  accepted; no `--decode` ⇒ zstd stats stay cheap; no diff path flags ⇒ full
  listing; `jobs=1`, `http-retries=0`, SigV4 **off**, ops default **text**,
  mount prefetch depth **1** ≡ 1.8.0 / 1.7.0 / …
- `.cfidx` v1 / `.cfdir` v1 on-wire bytes unchanged
- Loose `chunks/<2hex>/<62hex>.cnk` layout unchanged
- Additive opt-in only; JSON field names unchanged (only additive where noted);
  disk zstd orthogonal to HTTP plaintext body; no silent break of 1.8.0
  behaviour

## [1.8.0] — 2026-09-30

Phase 18 closeout — `pull --verify` (symmetric to push; default off);
`--cache-stats` + ops-json additive `cache_*` (≠ LRU); `cat`/`verify
--progress` (default off); docs + `demo_pull_verify_cache_stats.sh`;
`check_compat_1_7.sh`; P1 `doctor --progress`. Defaults remain ≡ **1.7.0**.
No pack / write mount / aws-sdk / remote scrub / extract prune /
bidirectional sync / push listing / LRU / store trim / **default** zstd /
HTTP wire compression. P1 `make --jobs` not delivered.

### Added

- **`pull --verify`** (Phase 18 M1): after a successful pull, treat local
  `--store` as `ChunkSource` and verify each listing (symmetric to
  `push --verify` on dest). Dry-run / failed pull skip verify. Default
  **off** ≡ 1.7. Does **not** reshape pull ops-json fields (stderr only).
  See `docs/pull.md`.
- **`--cache-stats` stderr** (Phase 18 M2): on cached read commands
  (`cat` / `verify` / `extract` / `mount` / `pull` / `doctor`); requires
  `--cache`; emits `cache: hits=H miss_fills=F miss_refused=R`. Observation
  only — **≠** LRU / trim / eviction / sync / fallback. Default quiet ≡ 1.7.
- **Ops-json additive `cache_*`** (Phase 18 M3): when `--cache` +
  `--format json`, add `cache_hits` / `cache_miss_fills` /
  `cache_miss_refused` from the same `CacheStatsRef` as `--cache-stats`.
  Without `--cache`, **omit** the three keys (no `null`). Old field names
  frozen. Orthogonal to `--cache-stats` / `--progress`.
- **`cat --progress`** (Phase 18 M4): reuse `ProgressReporter`; stderr
  `progress: op=cat done=N/TOTAL` per listing chunk; orthogonal to
  `--format json` / `--jobs` / `--cache` / `--fallback` / `--cache-stats`.
  Default **off** ≡ 1.7.
- **`verify --progress` + docs + `demo_pull_verify_cache_stats.sh`**
  (Phase 18 M5): `progress: op=verify done=N/TOTAL`; ops-json / stability /
  pull / perf / remote-layout narrative; smoke for `pull --verify`,
  `--cache`+`--cache-stats`, `cat`/`verify --progress`, and default-quiet
  path.
- **`check_compat_1_7.sh` + 1.7 regression gate** (Phase 18 M6 / G6): runs
  `check_compat_1_6.sh` (keeps 1_0…1_6 independently runnable), then asserts
  `pull --help` contains `--verify`, at least one of `cat`/`verify`
  advertises `--cache-stats`, and both `cat`/`verify --help` contain
  `--progress`; thin non-goals: no `--delete`/prune, no pack, no
  `--cache-lru` / `store trim`, no `aws-sdk` in `Cargo.lock`, create
  compression still opt-in / no default zstd. Asserts
  `demo_pull_verify_cache_stats.sh` present + executable (does not
  force-run). No absolute perf SLA.
- **P1 `doctor --progress`** (Phase 18 M6 / O1): opt-in; default **off** ≡
  1.7; stderr `progress: op=doctor done=N/TOTAL` per checked chunk;
  orthogonal to `--format json` / `--jobs` / `--cache` / `--fallback` /
  `--cache-stats`.
- **P1 docs brush** (Phase 18 M6 / O2): `docs/remote-layout.md` Phase 18
  table — `--cache-stats` ≠ LRU ≠ sync ≠ fallback; README / stability /
  ops-json Phase 18 closeout narrative.
- Workspace version **1.8.0** (Phase 18 M7 closeout).

### Not delivered / deferred (Phase 18)

- **`make --jobs`** — P1 O4 **not** delivered (FastCDC streaming chunking is
  inherently serial per file; not forced as fake jobs)
- **packfile** / multi-chunk objects — deferred (see `docs/perf.md`)
- **Write mount** / COW / writable FUSE — non-goal
- Full **`aws-sdk-*`** / multipart / IMDS / SSO / ListObjects — non-goal
- **Remote scrub** / remote GC — deferred (use `verify --source` / `doctor` /
  local `store scrub --listing`)
- **Extract prune** / `--delete` — non-goal
- Bidirectional sync / watch dirs — non-goal
- Byte-range HTTP resume / `push` listing upload — non-goal
- Cache LRU / store trim — non-goal
- **Default** store zstd / HTTP Content-Encoding / wire compression /
  `store recompress` — non-goal

### Compatibility

- CLI defaults match **1.7.0**: no `--verify` on pull ⇒ quiet; no
  `--cache-stats` ⇒ no cache noise; no cat/verify/doctor `--progress` ⇒
  quiet; create compression **none**; no `--fallback` ⇒ single origin; no
  `--cache-max-bytes` ⇒ unbounded cache fill; plain integer cache-max still
  accepted; no `--decode` ⇒ zstd stats stay cheap; no diff path flags ⇒ full
  listing; `jobs=1`, `http-retries=0`, SigV4 **off**, ops default **text**,
  mount prefetch depth **1** ≡ 1.7.0 / 1.6.0 / …
- `.cfidx` v1 / `.cfdir` v1 on-wire bytes unchanged
- Loose `chunks/<2hex>/<62hex>.cnk` layout unchanged
- Additive opt-in only; JSON field names unchanged (only additive `cache_*`
  when `--cache`); disk zstd orthogonal to HTTP plaintext body; no silent
  break of 1.7.0 behaviour

## [1.7.0] — 2026-09-30

Phase 17 closeout — CLI create-time `--compression none|zstd` (default
**none** ≡ 1.6); `archive` / `extract` / `make --progress` (default off);
docs + `demo_zstd_progress.sh`; `check_compat_1_6.sh`; P1 CacheSource
observation counters (`hits` / `miss_fills` / `miss_refused` — **≠** LRU).
Defaults remain ≡ **1.6.0**. No pack / write mount / aws-sdk / remote scrub /
extract prune / bidirectional sync / push listing / LRU / store trim /
**default** zstd / HTTP wire compression. P1 `cat`/`verify --progress` not
delivered.

### Added

- **CLI create-time `--compression none|zstd`** (Phase 17 M1–M2): on `make` /
  `archive`; omit ≡ **`none`** ≡ 1.6 create. Applied when `meta.toml` is
  absent; existing stores open by meta (explicit conflict → clear non-zero).
  `chunkforge-cli` enables the store `zstd` feature. Disk encoding is **not**
  HTTP Content-Encoding / wire compression and **not** pack — `get` / HTTP PUT
  bodies stay **plaintext**.
- **`archive` / `extract` / `make --progress`** (Phase 17 M3–M4): reuse
  `ProgressReporter`; stderr `progress: op=archive|extract|make done=N/TOTAL`;
  orthogonal to `--format json` (JSON → stdout). Default **off** ≡ 1.6.
- **Docs + `demo_zstd_progress.sh`** (Phase 17 M5): ops-json / stability /
  perf / remote-layout / archive / extract; smoke for zstd create +
  archive/extract/make `--progress` + default-none path.
- **`check_compat_1_6.sh` + 1.6 regression gate** (Phase 17 M6 / G6): runs
  `check_compat_1_5.sh` (keeps 1_0…1_5 independently runnable), then asserts
  `make`/`archive --help` contain `--compression` and
  `archive`/`extract`/`make --help` contain `--progress`; thin non-goals: no
  `--delete`/prune, no pack, no `--cache-lru` / `store trim`, no `aws-sdk` in
  `Cargo.lock`. Asserts `demo_zstd_progress.sh` present + executable (does
  not force-run). No absolute perf SLA.
- **P1 CacheSource observation counters** (Phase 17 M6 / O1): atomic `hits` /
  `miss_fills` / `miss_refused` on `get` path; exposed via getters. Soft
  budget remains refuse-fill — counters are **not** LRU / trim.
- Workspace version **1.7.0** (Phase 17 M7 closeout).

### Not delivered / deferred (Phase 17)

- **`cat` / `verify --progress`** — P1 O3 **not** delivered (schedule)
- **packfile** / multi-chunk objects — deferred (see `docs/perf.md`)
- **Write mount** / COW / writable FUSE — non-goal
- Full **`aws-sdk-*`** / multipart / IMDS / SSO / ListObjects — non-goal
- **Remote scrub** / remote GC — deferred (use `verify --source` / `doctor` /
  local `store scrub --listing`)
- **Extract prune** / `--delete` — non-goal
- Bidirectional sync / watch dirs — non-goal
- Byte-range HTTP resume / `push` listing upload — non-goal
- Cache LRU / store trim — non-goal
- **Default** store zstd / HTTP Content-Encoding / wire compression /
  `store recompress` — non-goal

### Compatibility

- CLI defaults match **1.6.0**: create compression **none**; no `--progress`
  ⇒ quiet stderr on archive/extract/make; no `--fallback` ⇒ single origin; no
  `--cache-max-bytes` ⇒ unbounded cache fill; plain integer cache-max still
  accepted; no `--decode` ⇒ zstd stats stay cheap; no diff path flags ⇒ full
  listing; `jobs=1`, `http-retries=0`, SigV4 **off**, ops default **text**,
  mount prefetch depth **1** ≡ 1.6.0 / 1.5.0 / …
- `.cfidx` v1 / `.cfdir` v1 on-wire bytes unchanged
- Loose `chunks/<2hex>/<62hex>.cnk` layout unchanged
- Additive opt-in only; JSON field names unchanged; disk zstd orthogonal to
  HTTP plaintext body; no silent break of 1.6.0 behaviour

## [1.6.0] — 2026-09-29

Phase 16 closeout — `FallbackSource` / CLI `--fallback` (Missing-only;
Cache wraps whole chain); human `--cache-max-bytes` suffixes; `store stats`
`bytes_plaintext` / `--decode`; `demo_fallback_bytes_suffix.sh`;
`check_compat_1_5.sh`; P1 `diff --path`/`--exclude`/`--exclude-from` + thin
docs. Defaults remain ≡ **1.5.0**. No pack / write mount / aws-sdk / remote
scrub / extract prune / bidirectional sync / push listing / LRU / store trim.
P1 O3 `archive`/`extract`/`make --progress` not delivered.

### Added

- **`FallbackSource` + CLI `--fallback`** (Phase 16 M1–M2): Missing-only
  ordered failover behind primary; Transient/Corrupt fail fast; outer Cache
  wraps the whole chain. Zero times ≡ 1.5 single origin. On `cat` / `verify` /
  `extract` / `mount` / `pull` / `doctor`. **≠ cache ≠ sync ≠ prune**.
- **Human `--cache-max-bytes` suffixes** (Phase 16 M3): `1M` / `64Mi` /
  `K`/`G`/`Ki`/`Gi` (1024-base) alongside plain integers; refuse-fill unchanged.
- **`store stats` / `du` `bytes_plaintext` + `--decode`** (Phase 16 M4):
  `compression=none` ⇒ equals `bytes_on_disk`; zstd ⇒ `null` unless `--decode`.
  Observation only — **≠ trim ≠ LRU**.
- **Docs + `demo_fallback_bytes_suffix.sh`** (Phase 16 M5): ops-json /
  mount/extract/pull/stability/perf; smoke for failover + suffixes +
  `bytes_plaintext`.
- **`check_compat_1_5.sh` + 1.5 regression gate** (Phase 16 M6): runs
  `check_compat_1_4.sh` (keeps 1_0…1_4 independently runnable), then asserts
  1.6 help flags (`--fallback` on read commands; `cat`/`mount
  --cache-max-bytes`; `store stats` `--decode` / `bytes_plaintext`); thin
  non-goals: no `--delete`/prune, no pack, no `--cache-lru` / `store trim`,
  no `aws-sdk` in `Cargo.lock`. Asserts `demo_fallback_bytes_suffix.sh`
  present + executable (does not force-run). No absolute perf SLA.
- **P1 `diff --path` / `--exclude` / `--exclude-from`** (Phase 16 M6 / O1):
  narrow both sides with `PathFilter` before compare; default no flags ≡ 1.5
  full diff; JSON field names unchanged. **Not** sync / prune.
- **P1 O2 thin docs**: `docs/remote-layout.md` Phase tags → through 1.6;
  README / stability document `fallback` ≠ `cache` ≠ sync.
- Workspace version **1.6.0** (Phase 16 M7 closeout).

### Not delivered / deferred (Phase 16)

- **O3 `archive` / `extract` / `make --progress`** — P1 **not** delivered
  (ProgressReporter reuse deferred; default remains off ≡ 1.5)
- **packfile** / multi-chunk objects — deferred (see `docs/perf.md`)
- **Write mount** / COW / writable FUSE — non-goal
- Full **`aws-sdk-*`** / multipart / IMDS / SSO / ListObjects — non-goal
- **Remote scrub** / remote GC — deferred (use `verify --source` / `doctor` /
  local `store scrub --listing`)
- **Extract prune** / `--delete` — non-goal
- Bidirectional sync / watch dirs — non-goal
- Byte-range HTTP resume / `push` listing upload — non-goal
- Cache LRU / store trim — non-goal
- Transient auto-switch to next `--fallback` / `--fallback` on `push --dest` —
  non-goal

### Compatibility

- CLI defaults match **1.5.0**: no `--fallback` ⇒ single origin; no
  `--cache-max-bytes` ⇒ unbounded cache fill; plain integer cache-max still
  accepted; no `--decode` ⇒ zstd stats stay cheap; no diff path flags ⇒ full
  listing; `jobs=1`, `http-retries=0`, SigV4 **off**, ops default **text**,
  `--progress` **off**, mount prefetch depth **1** ≡ 1.5.0 / 1.4.0 / …
- `.cfidx` v1 / `.cfdir` v1 on-wire bytes unchanged
- Loose `chunks/<2hex>/<62hex>.cnk` layout unchanged
- Additive opt-in only; JSON field names unchanged (only additive
  `bytes_plaintext`); no silent break of 1.5.0 behaviour

## [1.5.0] — 2026-09-29

Phase 15 closeout — `CacheSource` soft budget / `--cache-max-bytes`
(refuse-fill ≠ LRU); `make`/`cat --format text|json`; ops-json finalize;
`demo_cache_budget_ops_json.sh`; `check_compat_1_4.sh`; P1
`store scrub --listing`. Defaults remain ≡ **1.4.0**. No pack / write mount /
aws-sdk / remote scrub / extract prune / bidirectional sync / push listing /
LRU / store trim. P1 `bytes_plaintext` not delivered.

### Added

- **`CacheSource` soft budget** (Phase 15 M1):
  `CacheSource::with_max_bytes(primary, cache, max_bytes: Option<u64>)`;
  `None` / `new` ≡ 1.4 unbounded fill. On miss, if
  `cache.stats().bytes_on_disk + plaintext.len() > max` → skip `put`, still
  return primary plaintext. Never evicts / removes. No `ChunkSource` /
  `ChunkSink` signature change.
- **CLI `--cache-max-bytes`** (Phase 15 M2): `cat` / `verify` / `extract` /
  `mount` (every command that already has `--cache`). Pure integer bytes
  (`u64`); no KiB suffix. Requires `--cache` — without it → clear non-zero
  error. With `--cache` + max → `with_max_bytes(..., Some(N))`; `--cache`
  alone → `new` / `None` ≡ 1.4 unbounded. Orthogonal to jobs / format /
  prefetch / retries / SigV4. No LRU / eviction / prune. See `docs/mount.md` /
  `docs/extract.md`.
- **`make --format text|json`** (Phase 15 M3): Shared `CliFormat`; default
  **text** ≡ 1.4.0 stderr summary. **json**: one stdout object
  `{ok, bytes, chunks, new, reused}` (no text dual-write); exit
  format-independent. See `docs/ops-json.md`.
- **`cat --format text|json`** (Phase 15 M4): Shared `CliFormat`; default
  **text** ≡ 1.4.0 (still writes `-o`; almost silent on success). **json**:
  one stdout object `{ok, bytes}` (`bytes` = written / `index.total_size`);
  still writes `-o`; no text dual-write; exit format-independent. Orthogonal
  to `--cache` / `--cache-max-bytes` / `--jobs`. See `docs/ops-json.md`.
- **Ops-json finalize + `demo_cache_budget_ops_json`** (Phase 15 M5):
  `docs/ops-json.md` make/cat rows **final**; Out of scope make/cat removed;
  explicit `--cache-max-bytes` = refuse-fill (**≠ LRU ≠ trim ≠ GC ≠ sync**).
  Smoke `scripts/demo_cache_budget_ops_json.sh` (make/cat json parse +
  first-fill / second-miss no disk growth). README Phase 15 / **1.5.0**;
  `docs/perf.md` notes **1.5.0** still does not implement pack.
- **`check_compat_1_4.sh` + 1.4 regression gate** (Phase 15 M6): runs
  `check_compat_1_3.sh` (keeps 1_0…1_3 independently runnable), then asserts
  1.5 help flags (`make`/`cat --format`; `cat`/`verify`/`extract`/`mount
  --cache-max-bytes`); thin non-goals: no `--delete`/prune, no pack, no
  `--cache-lru` / `store trim`. Asserts `demo_cache_budget_ops_json.sh`
  present + executable (does not force-run). No absolute perf SLA.
- **P1 `store scrub --listing <index>`** (Phase 15 M6): local referenced-id
  rehash only; default no flag ≡ 1.4 full-store; **not** remote scrub.
- Workspace version **1.5.0** (Phase 15 M7 closeout).

### Not delivered / deferred (Phase 15)

- **`bytes_plaintext` (store stats)** — P1 **not** delivered (decode cost;
  do not alter `.cnk`; deferred)
- **packfile** / multi-chunk objects — deferred (see `docs/perf.md`)
- **Write mount** / COW / writable FUSE — non-goal
- Full **`aws-sdk-*`** / multipart / IMDS / SSO / ListObjects — non-goal
- **Remote scrub** / remote GC — deferred (use `verify --source` / `doctor`)
- **Extract prune** / `--delete` — non-goal
- Bidirectional sync / watch dirs — non-goal
- Byte-range HTTP resume / `push` listing upload — non-goal
- Cache LRU / store trim — non-goal

### Compatibility

- CLI defaults match **1.4.0**: no `--cache-max-bytes` ⇒ unbounded cache fill;
  `make`/`cat --format` default **text**; `jobs=1`, `http-retries=0`, SigV4
  **off**, ops commands default **text**, `--progress` **off**, mount prefetch
  default depth **1** ≡ 1.4.0 / 1.3.0 / 1.2.0 / 1.1.0 / 1.0.0
- `.cfidx` v1 / `.cfdir` v1 on-wire bytes unchanged
- Loose `chunks/<2hex>/<62hex>.cnk` layout unchanged
- Additive opt-in only; `--cache-max-bytes` = refuse-fill ≠ LRU ≠ trim ≠ GC ≠
  sync; no silent break of 1.4.0 behaviour

## [1.4.0] — 2026-09-29

Phase 14 closeout — `Store::stats` / `store stats` (alias `du`) + json;
`push --path`/`--exclude`; `--exclude-from` (archive/extract/pull/push);
ops-json store-stats + push path notes; `demo_push_path_store_stats.sh`;
`check_compat_1_3.sh`; defaults remain ≡ **1.3.0**. No pack / write mount /
aws-sdk / remote scrub / extract prune / bidirectional sync / push listing
upload. P1 `make`/`cat --format json` not delivered.

### Added

- **`Store::stats`** (Phase 14 M1): `chunkforge-store` →
  `StoreStats { chunks, bytes_on_disk, compression }` — aggregates via
  `list_chunk_ids` + per-`.cnk` `metadata().len()` (no plaintext decode).
- **`store stats` / `du` + `--format text|json`** (Phase 14 M2): default
  **text** one stdout line `store stats: chunks=N bytes_on_disk=M
  compression=none|zstd`; **json** `{ok, chunks, bytes_on_disk, compression}`;
  no text dual-write; exit format-independent; read-only (no `--apply` /
  delete / trim / LRU). See `docs/ops-json.md` / `docs/doctor-gc.md`.
- **`push --path` / `--exclude`** (Phase 14 M3): symmetric to pull; only
  matching `.cfdir` **File** entry chunk ids uploaded (Dir never contributes;
  listing **not** uploaded). Default (no flags) ≡ **1.3.0** full reference
  set. `.cfidx` + any path/exclude → clear non-zero error. Orthogonal to
  `--dry-run` / `--format` / `--jobs` / `--progress` / retries / SigV4 /
  `--verify`. JSON field names unchanged; `unique_chunks` = filtered unique
  id count. See `docs/push.md`.
- **`--exclude-from <file>`** (Phase 14 M4): repeatable on **archive /
  extract / pull / push**. UTF-8, one `ExcludePat` per line; blank and `#`
  lines skipped; trim. File lines ∪ CLI `--exclude` → one `PathFilter` (no
  `ignore`/`globset`; no `--path-from`). Illegal pattern → same error as
  `--exclude`. No flags ≡ **1.3.0**.
- **Ops-json + `demo_push_path_store_stats`** (Phase 14 M5): `docs/ops-json.md`
  confirms **`store stats`** row + push path-filter notes (`unique_chunks`=
  filtered; path ≠ listing upload ≠ sync/prune). Smoke
  `scripts/demo_push_path_store_stats.sh` (full-tree archive → store stats
  json → push `--path` PUT < full → `--exclude-from`; local put_stub).
  README Phase 14 / **1.4.0**; `docs/perf.md` notes **1.4.0** still does not
  implement pack.
- **`check_compat_1_3.sh` + 1.3 regression gate** (Phase 14 M6): runs
  `check_compat_1_2.sh` (keeps 1_0 / 1_1 / 1_2 independently runnable), then
  asserts 1.4 help flags (`push --path`/`--exclude`; `store stats`/`du
  --format`; archive/extract/pull/push `--exclude-from`); thin non-goals: no
  `--delete`/prune on extract, no pack / `--pack*`. Asserts
  `demo_push_path_store_stats.sh` present + executable (does not re-run full
  HTTP stub demo). No absolute perf SLA.
- Workspace version **1.4.0** (Phase 14 M7 closeout).

### Not delivered / deferred (Phase 14)

- **`make --format text|json`** / **`cat --format text|json`** (P1) — not
  delivered
- **packfile** / multi-chunk objects — deferred (see `docs/perf.md`)
- **Write mount** / COW / writable FUSE — non-goal
- Full **`aws-sdk-*`** / multipart / IMDS / SSO / ListObjects — non-goal
- **Remote scrub** / remote GC — deferred (use `verify --source` / `doctor`)
- **Extract prune** / `--delete` — non-goal
- Bidirectional sync / watch dirs — non-goal
- Byte-range HTTP resume / `push` listing upload — non-goal
- Cache LRU / store trim — non-goal

### Compatibility

- CLI defaults match **1.3.0**: no `--path`/`--exclude`/`--exclude-from` ⇒
  full tree / full reference set; `archive --format` default **text**;
  `jobs=1`, `http-retries=0`, SigV4 **off**, ops commands default **text**,
  `--progress` **off**, mount prefetch default depth **1** ≡ 1.3.0 / 1.2.0 /
  1.1.0 / 1.0.0
- `.cfidx` v1 / `.cfdir` v1 on-wire bytes unchanged
- Loose `chunks/<2hex>/<62hex>.cnk` layout unchanged
- Additive opt-in only; push path ≠ listing upload ≠ sync; store stats ≠
  trim; no silent break of 1.3.0 behaviour

## [1.3.0] — 2026-09-29

Phase 13 closeout — `PathFilter`; `archive`/`extract`/`pull --path`/`--exclude`;
`archive --format json`; ops-json archive row; `demo_path_filter.sh`;
`check_compat_1_2.sh`; defaults remain ≡ **1.2.0**. No pack / write mount /
aws-sdk / remote scrub / extract prune / bidirectional sync / `push --path`.

### Added

- **`PathFilter`** (Phase 13 M1): `chunkforge-index::PathFilter` / `ExcludePat`
  — `--path` prefix include (OR) + `--exclude` exact / trailing-`/` directory /
  edge `*` wildcards (`*.o`, `temp*`); no `ignore`/`globset`; illegal middle
  `*` / `**` → `Error::InvalidExcludePattern`. Unit tests in-crate.
- **`archive --path` / `--exclude` + `--format json`** (Phase 13 M2):
  repeatable path/exclude → `PathFilter`; walk order type-skip then filter;
  excluded files not chunked / not in `.cfdir`; orthogonal to seed / dry-run /
  jobs. Default **`--format text`** ≡ 1.2.0 stderr summary (includes
  `excluded=`); **`json`**: one stdout object (`ok` / `dry_run` / `files` /
  `dirs` / `chunks` / `written`|`would_write` / `reused`|`would_reuse` /
  `seed_reused_files` / `rechunked_files` / `skipped_symlinks` /
  `skipped_special` / `excluded`); no text dual-write; exit format-independent.
  See `docs/archive.md`.
- **`extract --path` / `--exclude` (non-prune)** (Phase 13 M3): reads full
  listing, materializes only matching File + necessary parent dirs; **never**
  deletes filtered-out listing paths or extra dest files; **no** `--delete`.
  Orthogonal to `--force` / `--skip-unchanged` / `--skip-trust-mtime` /
  `--dry-run` / `--format` / `--jobs`. Default (no path/exclude) ≡ 1.2.0 full
  tree. See `docs/extract.md`.
- **`pull --path` / `--exclude`** (Phase 13 M4): fetches only chunk ids from
  matching **File** entries; does **not** download or rewrite the listing.
  Orthogonal to `--dry-run` / `--format` / `--jobs` / `--progress` / retries /
  SigV4. Default ≡ 1.2.0 full reference set. JSON field names unchanged;
  `unique_chunks` = filtered unique id count. See `docs/pull.md`.
- **Ops-json archive + `demo_path_filter`** (Phase 13 M5): `docs/ops-json.md`
  adds **archive** write/dry-run rows; smoke
  `scripts/demo_path_filter.sh` (exclude → archive json → extract `--path`
  non-prune → pull `--path` subset). README Phase 13 / **1.3.0**;
  `docs/perf.md` notes **1.3.0** still does not implement pack.
- **`check_compat_1_2.sh` + 1.2 regression gate** (Phase 13 M6): runs
  `check_compat_1_1.sh` (keeps 1_0 / 1_1 independently runnable), then asserts
  1.3 help flags (`archive --format` + `--path`/`--exclude`, `extract --path`,
  `pull --path`); thin non-goals: no `--delete`/prune on extract, no pack
  subcommand / `--pack*`. Invokes `demo_path_filter.sh`. No absolute perf SLA.
  **P1 O3** thin docs: `docs/remote-layout.md` Phase tag → through 1.3;
  README responsibility table notes path filter ≠ prune.
- Workspace version **1.3.0** (Phase 13 M7 closeout).

### Not delivered / deferred (Phase 13)

- **`push --path` / `--exclude`** (P1) — not delivered
- **`store du` / `store stats --format json`** (P1) — not delivered
- **`--exclude-from <file>`** (P1) — not delivered
- **packfile** / multi-chunk objects — deferred (see `docs/perf.md`)
- **Write mount** / COW / writable FUSE — non-goal
- Full **`aws-sdk-*`** / multipart / IMDS / SSO / ListObjects — non-goal
- **Remote scrub** / remote GC — deferred (use `verify --source` / `doctor`)
- **Extract prune** / `--delete` — non-goal
- Bidirectional sync / watch dirs — non-goal
- Byte-range HTTP resume / `push` listing upload — non-goal

### Compatibility

- CLI defaults match **1.2.0**: no `--path`/`--exclude` ⇒ full tree / full
  reference set; `archive --format` default **text**; `jobs=1`,
  `http-retries=0`, SigV4 **off**, ops commands default **text**, `--progress`
  **off**, mount prefetch default depth **1** ≡ 1.2.0 / 1.1.0 / 1.0.0
- `.cfidx` v1 / `.cfdir` v1 on-wire bytes unchanged
- Loose `chunks/<2hex>/<62hex>.cnk` layout unchanged
- Additive opt-in only; path filter ≠ prune ≠ sync; no silent break of 1.2.0
  behaviour

## [1.2.0] — 2026-09-29

Phase 12 closeout — `gc --jobs`; `gc`/`store scrub --format json`;
`docs/ops-json.md` field matrix; `check_compat_1_1.sh`; opt-in `--progress`;
defaults remain ≡ **1.1.0**. No pack / write mount / aws-sdk / remote scrub /
`archive --format json` / extract prune / bidirectional sync.

### Added

- **`gc --jobs N`** (Phase 12 M1): default **1** ≡ 1.1.0 serial; rejects
  `jobs=0` via shared `parse_jobs`. Dry-run path listing stays ordered/serial;
  `--apply` uses `parallel::map_indexed` for per-id `Store::remove` (result set
  ≡ jobs=1). Symmetric to `store scrub --jobs`. See `docs/doctor-gc.md`.
- **`gc --format text|json`** (Phase 12 M2): default **text** ≡ 1.1.0 path list
  + stderr summary. JSON: one object on stdout (`ok` / `dry_run` / `applied` /
  `listings` / `referenced` / `unreferenced` / `deleted`); no path listing and
  no duplicate stderr summary; exit codes format-independent; `--jobs`
  orthogonal. Shared `CliFormat`. See `docs/doctor-gc.md`.
- **`store scrub --format text|json`** (Phase 12 M3): default **text** ≡ 1.1.0
  (`scrub: ok=… corrupt=… unreadable=…` + per-bad-id lines). JSON: one object
  on stdout (`ok` / `checked` / `ok_count` / `corrupt` / `unreadable` /
  `corrupt_ids` / `unreadable_ids`); bad ids only in arrays (no text lines);
  exit codes format-independent; `--jobs` orthogonal. Shared `CliFormat`.
  See `docs/doctor-gc.md`.
- **Ops JSON field matrix + `demo_ops_maint`** (Phase 12 M4):
  `docs/ops-json.md` (eight-command minimum stable fields; default text;
  field rename → breaking) linked from `docs/stability.md`;
  `scripts/demo_ops_maint.sh` (gc dry-run json + scrub json + `--jobs 4`,
  local only); README Phase 12 / **1.2.0**; `docs/doctor-gc.md` aligned with
  §3.2; `docs/perf.md` notes **1.2.0** still does not implement pack.
- **`check_compat_1_1.sh` + 1.1 regression gate** (Phase 12 M5):
  `scripts/check_compat_1_1.sh` runs `check_compat_1_0.sh` (keeps 1_0
  independently runnable), then asserts 1.1/1.2 help flags
  (`extract --skip-trust-mtime`/`--format`, `push`/`pull --format`,
  `mount --prefetch-chunks`, `gc --jobs`/`--format`, `store scrub --format`);
  thin non-goal: no pack subcommand / `--pack*` in CLI help (aws-sdk already
  gated by 1_0). No absolute perf SLA; does not run `bench_loose_http.sh`.
- **Opt-in `--progress`** (Phase 12 M6 / O1): explicit flag; **default off**
  ≡ 1.1.0. Covers **`push` / `pull` / `store scrub`** (per chunk) and **`gc
  --apply`** (per delete). Stderr lines `progress: op=<name> done=N/TOTAL`
  (total known). Orthogonal to `--format json` (progress → stderr; JSON →
  stdout only). Hand-rolled — **no** `indicatif` / tracing / otel. Help lists
  `--progress` on those commands.
- Workspace version **1.2.0** (Phase 12 M7 closeout).

### Not delivered / deferred (Phase 12)

- **`archive --format json`** (P1) — not delivered
- **packfile** / multi-chunk objects — deferred (see `docs/perf.md`)
- **Write mount** / COW / writable FUSE — non-goal
- Full **`aws-sdk-*`** / multipart / IMDS / SSO / ListObjects — non-goal
- **Remote scrub** / remote GC — deferred (use `verify --source` / `doctor`)
- **Extract prune** / `--delete` — non-goal
- Bidirectional sync / watch dirs — non-goal
- Byte-range HTTP resume / `push` listing upload — non-goal

### Compatibility

- CLI defaults match **1.1.0**: `jobs=1` (incl. `gc`), `http-retries=0`,
  SigV4 **off**, `diff`/`verify`/`doctor`/`extract`/`push`/`pull`/`gc`/
  `store scrub` default **text**, `--progress` **off**, mount prefetch
  default depth **1** ≡ 1.1.0 / 1.0.0
- `.cfidx` v1 / `.cfdir` v1 on-wire bytes unchanged
- Loose `chunks/<2hex>/<62hex>.cnk` layout unchanged
- Additive opt-in only; no silent break of 1.1.0 behaviour

## [1.1.0] — 2026-09-29

Phase 11 closeout — `extract --skip-trust-mtime`; `extract`/`push`/`pull
--format json`; P1 `mount --prefetch-chunks N`; defaults remain ≡ **1.0.0**.
No pack / write mount / aws-sdk / remote scrub / extract prune.

### Added

- **`extract --skip-trust-mtime`** (Phase 11 M1 / Phase 10 P1 O2 make-up):
  requires `--skip-unchanged`; opt-in; default **off** ≡ 1.0.0 content path.
  Library `judge_extract_unchanged_opts` — size+mtime hit skips content
  BLAKE3 (symmetric to `archive --seed-trust-mtime`); docs warn about
  forged / clock-drift / `cp -p` mtimes. See `docs/extract.md`.
- **`extract --format text|json`** (Phase 11 M2): default **text** ≡ 1.0.0
  stderr summary. JSON: one object on stdout (`ok`/`skipped`/`wrote`/`dirs`,
  or dry-run `would_*` + `dry_run`); no duplicate stderr summary; exit codes
  format-independent. Shared `CliFormat`. See `docs/extract.md`.
- **`push` / `pull --format text|json`** (Phase 11 M3): default **text** ≡
  1.0.0 stderr summary. JSON: one object on stdout with failure-class fields
  (`ok`/`skipped`/`uploaded|fetched`/`failed`/`failed_transient`/
  `failed_permanent`/`retries`/`unique_chunks`/`listings`/`dry_run`); no
  duplicate stderr summary; exit codes format-independent. Field rename is
  breaking. See `docs/push.md` / `docs/pull.md`.
- **Ops JSON demo + README Phase 11** (Phase 11 M4): `scripts/demo_ops_json.sh`
  — local smoke for `extract --skip-trust-mtime` + `extract`/`push`/`pull
  --format json` (put_stub; no internet). README Phase 11 / **1.1.0** section.
- **`mount --prefetch-chunks N`** (Phase 11 M6 / P1 **O1** / Phase 10 O3
  make-up): prefetch depth (default **1** ≡ 1.0.0; hard cap **≤2**; clap
  `1..=2`; library clamp). `--no-prefetch` still wins.
  `BlobFs`/`DirFs::with_prefetch_chunks`,
  `PrefetchCache::enabled_with_max_chunks`. Docs: `docs/mount.md`.
- Workspace version **1.1.0** (Phase 11 M7 closeout).

### Not delivered / deferred (Phase 11)

- **P1 O2** `gc --jobs` — not delivered
- **packfile** / multi-chunk objects — deferred (see `docs/perf.md`)
- **Write mount** / COW / writable FUSE — non-goal
- Full **`aws-sdk-*`** / multipart / IMDS / SSO / ListObjects — non-goal
- **Remote scrub** / remote GC — deferred (use `verify --source` / `doctor`)
- **Extract prune** / `--delete` — non-goal
- Bidirectional sync / watch dirs — non-goal
- Byte-range HTTP resume / `push` listing upload — non-goal

### Compatibility

- CLI defaults match **1.0.0**: `jobs=1`, `http-retries=0`, SigV4 **off**,
  `diff`/`verify`/`doctor`/`extract`/`push`/`pull` default **text**,
  `extract` without `--skip-unchanged` / `--skip-trust-mtime` / `--dry-run`
  ≡ full / conflict / content-path semantics of 1.0.0; mount prefetch default
  depth **1** ≡ 1.0.0
- `.cfidx` v1 / `.cfdir` v1 on-wire bytes unchanged
- Loose `chunks/<2hex>/<62hex>.cnk` layout unchanged
- `--no-prefetch` restores 0.9.0 on-demand mount get behaviour

## [1.0.0] — 2026-09-29

Phase 10 closeout — FUSE sequential prefetch (default **on** / `--no-prefetch`),
`docs/stability.md`, `scripts/check_compat_1_0.sh` + `demo_mount_prefetch.sh`,
P1 O1 `verify`/`doctor --format json`; format/trait/loose layout frozen;
default jobs=1 / retries=0 / SigV4 off / diff text / extract without flags ≡
**0.9.0**.

### Added

- **FUSE sequential chunk prefetch** (Phase 10 M1–M2 / Phase 9 O2 make-up):
  process-local prefetch of the next chunk after a forward sequential `read`
  (conservative: at most one subsequent chunk / ≤512KiB). Default **on**.
  `chunkforge mount --no-prefetch` disables prefetch (≡ 0.9.0 on-demand `get`).
  Seek / cross-file / backward / non-contiguous reads cold-start the window.
  Still **RO**; does not change `ChunkSource`/`ChunkSink` signatures. Docs:
  `docs/mount.md`.
- **1.0 stability freeze** (Phase 10 M3): `docs/stability.md` — frozen surface
  (`.cfidx`/`.cfdir` v1 bytes, `ChunkSource`/`ChunkSink` signatures, loose CAS +
  HTTP layout, CLI defaults vs 0.9.0, mount prefetch default), breaking-change
  policy, promises / non-promises. README: Phase 10 / **1.0.0** status,
  **Non-goals (Phase 10 / 1.0)**.
- **Compat gate + prefetch demo** (Phase 10 M4): `scripts/check_compat_1_0.sh`
  (key CLI flags, no `aws-sdk`, fuse prefetch lib tests, demo subset) and
  `scripts/demo_mount_prefetch.sh` (get-count unit tests; optional real mount).
- **`verify` / `doctor --format text|json`** (Phase 10 M6 / P1 **O1**): default
  **text** ≡ 0.9.0 stderr/stdout behaviour. JSON: one object on stdout
  (`verify`: `ok`/`kind`/`bytes|files`/`chunks`; `doctor`: `ok`/`listings`/
  `checked`/`missing`/`deep`/`retries`; missing ids only inside JSON). Exit
  codes format-independent. Shared `CliFormat` (renamed from `DiffFormat`;
  `diff --format` unchanged). See `docs/doctor-gc.md`.
- Workspace version **1.0.0** (Phase 10 M7 closeout).

### Not delivered / deferred (Phase 10)

- **P1 O2** `extract --skip-trust-mtime` — not delivered
- **P1 O3** `mount --prefetch-chunks N` — not delivered (fixed conservative
  prefetch; only `--no-prefetch` ship)
- **packfile** / multi-chunk objects — deferred (see `docs/perf.md`)
- **Write mount** / COW / writable FUSE — non-goal
- Full **`aws-sdk-*`** / multipart / IMDS / SSO / ListObjects — non-goal
- **Remote scrub** / remote GC — deferred (use `verify --source` / `doctor`)
- **Extract prune** / `--delete` — non-goal
- Bidirectional sync / watch dirs — non-goal
- Byte-range HTTP resume / `push` listing upload — non-goal

### Compatibility

- CLI defaults match **0.9.0**: `jobs=1`, `http-retries=0`, SigV4 **off**,
  `diff` default **text**, `extract` without `--skip-unchanged` / `--dry-run`
  ≡ full / conflict semantics of 0.9.0
- `.cfidx` v1 / `.cfdir` v1 on-wire bytes unchanged
- Loose `chunks/<2hex>/<62hex>.cnk` layout unchanged
- `--no-prefetch` restores 0.9.0 on-demand mount get behaviour

## [0.9.0] — 2026-09-29

Phase 9 closeout: incremental extract (`--skip-unchanged` / `--dry-run`), loose
HTTP perf baseline (`docs/perf.md` + `scripts/bench_loose_http.sh`), and SigV4
shared-credentials fallback, plus `scripts/demo_extract_skip.sh`. `.cfidx` /
`.cfdir` v1 bytes and `ChunkSource` / `ChunkSink` signatures stay frozen;
default extract (no new flags) matches **0.8.0**. **O2 FUSE sequential prefetch
was not delivered.** No full AWS SDK, multipart, packfile, write mount,
bidirectional sync, extract prune, video analysis, remote scrub, byte-range
resume, or push listing upload.

### Added

- **`extract --skip-unchanged`** (Phase 9 M1): opt-in skip when dest size + content
  BLAKE3 already match listing `blob_blake3` (no chunk fetch/write; mode untouched).
  Default **off** ≡ 0.8.0. Match takes priority over `--force`. Helper
  `judge_extract_unchanged` / `UnchangedVerdict` in `chunkforge-index`.
- **`extract --dry-run`** (Phase 9 M3 / G2–G3): plan only — creates/modifies **no**
  target paths under `-o` (output root included); never fetches chunks. Without
  `--skip-unchanged`, does not open `--store`/`--source` (`would_write=` = all
  listing files; existing conflicts without `--force` → `would_fail=`). With
  `--skip-unchanged`, only reads local dests for size+BLAKE3 judgment (no chunk
  get). Stderr:
  `extract: dry-run: would_skip=… would_write=… would_dirs=… would_fail=…`.
  Exit **0** when the listing is valid (even if `would_fail>0`); invalid listing
  → non-zero. Mismatch + `--force` → `would_write`.
- **Docs + demo** (Phase 9 M4 / G5): `docs/extract.md` (`--skip-unchanged` /
  `--force` / `--dry-run` overlap; explicitly **no prune**);
  `scripts/demo_extract_skip.sh`; README Phase 9 section
- **Loose HTTP perf baseline** (Phase 9 M6 / P1 **O1**): `docs/perf.md` (how to
  measure local loose push/pull; **pack promotion checklist**; pack **not**
  implemented this phase) + `scripts/bench_loose_http.sh` (local `put_stub`
  wall-clock / chunk/s for `--jobs 1`, optional `--also-jobs-4`; machine-parseable
  one-line stdout; product defaults remain jobs=1 / retries=0)
- **SigV4 shared credentials fallback** (Phase 9 M6 / P1 **O3**): when
  `--aws-sigv4` and env access/secret are missing, read
  `~/.aws/credentials` (or `AWS_SHARED_CREDENTIALS_FILE`), profile
  `AWS_PROFILE` or `default`. Still **no** IMDS / SSO / `aws-sdk-*`. See
  `docs/sigv4.md`

### Not delivered / deferred (Phase 9)

- **O2 FUSE sequential prefetch** — not delivered this milestone (RO mount
  unchanged; no readahead / prefetch cache)
- Full **`aws-sdk-*`** / credential provider chain / IMDS / SSO / ListObjects
- Complete S3 **multipart** upload API (single-object PUT only)
- **Packfile** / multi-chunk single object (loose `.cnk` layout unchanged;
  O1 baseline documents promotion criteria only)
- Write mount / COW / writable FUSE
- Bidirectional sync / watch directories / conflict resolution
- **Extract prune** / `--delete` of extra files under `-o` (`extract` ≠ sync)
- Video analysis / GPU·LLM / P2P
- **Remote scrub** / remote GC / lifecycle (use `verify --source` for listing
  ref integrity; `store scrub` / `gc` stay **local** `--store` only)
- Byte-range / partial-chunk resume (retries **whole chunks** only)
- `push` still does **not** upload listings

### Non-goals (Phase 9)

- No rewrite of `.cfidx` / `.cfdir` v1 byte layouts
- No change to `ChunkSource` / `ChunkSink` method signatures
- No full AWS SDK / multipart / packfile; HTTP surface remains **ureq** (+
  optional minimal SigV4 with env / shared-file creds)
- No write mount; no bidirectional sync; no extract prune; no video analysis;
  no remote scrub; no byte-range resume; no push listing upload
- No FUSE sequential prefetch in this release (O2 deferred)

### Notes

- Without new extract flags, extract / HTTP / push / pull / verify / doctor /
  diff behaviour matches **0.8.0** (default no skip, no dry-run, retries=0,
  SigV4 off, diff text)
- Default HTTP chunk layout remains byte-compatible with **0.8.0** / **0.7.0**
- Responsibilities: `verify --source` = remote listing-ref integrity (not
  remote scrub); `doctor` = presence; `gc` / `store scrub` = local only;
  `diff` = listing/tree compare (report only); `extract --skip-unchanged` =
  incremental materialize (not sync / prune)

## [0.8.0] — 2026-09-29

Phase 8 closeout: bounded HTTP retries (`--http-retries`), error-class push/pull
summaries, `diff --format json`, and optional minimal in-process SigV4
(`--aws-sigv4`), plus `scripts/demo_http_retry.sh`. `.cfidx` / `.cfdir` v1 bytes
and `ChunkSource` / `ChunkSink` signatures stay frozen; default retries=0 and
diff text match **0.7.0**. No full AWS SDK, multipart, packfile, write mount,
bidirectional sync, video analysis, remote scrub, or byte-range resume.

### Added

- **`--http-retries N`** (Phase 8 M2 / G1–G4): extra HTTP attempts for transient
  failures (default **0** ≡ 0.7.0 single attempt) on `push` / `pull` / `verify` /
  `doctor` / `cat` / `extract` (and `mount` via shared HTTP args). Optional
  `--http-retry-backoff-ms` (default 100; exponential + jitter, capped at 2s).
  Local `--store` / `file://` ignore the flags. Shared `RetryPolicy` in
  `chunkforge-remote` for `HttpChunkSource` / `HttpChunkSink`. `push` / `pull`
  summaries include `retries=`; `doctor` ok line too — see `docs/http-retry.md`
- **HTTP error classification** (Phase 8 M3): `ErrorClass` +
  `classify_http_status` / `classify_source_error` / `classify_sink_error` (no
  `ChunkSource`/`ChunkSink` signature change). push/pull summaries add
  `failed_transient=` / `failed_permanent=` (Missing+Corrupt → permanent).
  401 vs 503 distinguishable; hash / Corrupt failures are never retried
- **`diff --format text|json`** (Phase 8 M4 / G5): default **`text`** ≡ 0.7.0
  path lists + `diff:` summary; **`json`** emits one object with stable
  `added` / `removed` / `changed` / `meta_changed` arrays plus
  `chunks_shared` / `chunks_only_left` / `chunks_only_right`. Exit codes
  unchanged and format-independent — see `docs/diff.md`
- **Minimal in-process AWS SigV4** (Phase 8 M6 / P1): `chunkforge-remote` signs
  GET/HEAD/PUT with AWS4-HMAC-SHA256 (`hmac` + `sha2`; **no** `aws-sdk-*`).
  Payload hash = `hex(SHA256(body))` (empty → empty hash; never
  `UNSIGNED-PAYLOAD`). CLI `--aws-sigv4` (default **off** ≡ 0.7.0); credentials
  from `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` (required when flag on),
  optional `AWS_SESSION_TOKEN` / `AWS_REGION` (default `us-east-1` + warning).
  Conflicts with `--header Authorization:…` → clear error. Golden-vector unit
  tests + stub header assertions — see `docs/sigv4.md`
- **`scripts/demo_http_retry.sh`** + docs: local smoke (`put_stub.py
  --fail-transient` → `--http-retries 0` fails on 503; `≥2`/`3` succeeds;
  summary contains `retries=`); `docs/http-retry.md`; cross-links in
  remote-layout / sigv4 / diff

### Not delivered / deferred (Phase 8)

- Full **`aws-sdk-*`** / credential provider chain / IMDS / SSO / ListObjects
- Complete S3 **multipart** upload API (single-object PUT only)
- **Packfile** / multi-chunk single object (loose `.cnk` layout unchanged)
- Write mount / COW / writable FUSE
- Bidirectional sync / watch directories / conflict resolution
- Video analysis / GPU·LLM / P2P
- **Remote scrub** / remote GC / lifecycle (use `verify --source` for listing
  ref integrity; `store scrub` / `gc` stay **local** `--store` only)
- Byte-range / partial-chunk resume (Phase 8 retries **whole chunks** only)
- `push` still does **not** upload listings

### Non-goals (Phase 8)

- No rewrite of `.cfidx` / `.cfdir` v1 byte layouts
- No change to `ChunkSource` / `ChunkSink` method signatures
- No full AWS SDK / multipart / packfile; HTTP surface remains **ureq** (+
  optional minimal SigV4)
- No write mount; no bidirectional sync; no video analysis; no remote scrub;
  no byte-range resume

### Notes

- Without new flags, HTTP / push / pull / verify / doctor / diff text behaviour
  matches **0.7.0** (default `--http-retries 0`, `--format text`, SigV4 off)
- Default HTTP chunk layout remains byte-compatible with **0.7.0** / **0.6.0**
- Responsibilities: `verify --source` = remote listing-ref integrity (not
  remote scrub); `doctor` = presence; `gc` / `store scrub` = local only;
  `diff` = listing/tree compare (report only)

## [0.7.0] — 2026-09-29

Phase 7 closeout: listing diff (`chunkforge diff`), local CAS bitrot scrub
(`store scrub`), `archive --seed-trust-mtime`, and `extract --force`, plus
`scripts/demo_diff_scrub.sh`. `.cfidx` / `.cfdir` v1 bytes and `ChunkSource` /
`ChunkSink` signatures stay frozen; no AWS SDK, in-process SigV4, write mount,
packfile, bidirectional sync, video analysis, or remote scrub.

### Added

- **`chunkforge diff`** (Phase 7 P0): listing↔listing compare of two `.cfdir`
  archives — path-level **added** / **removed** / **changed** (content) /
  **meta_changed** (same blake3, mode/mtime differ); chunk-set stats
  `chunks_shared` / `chunks_only_left` / `chunks_only_right`; stable stdout
  summary line; exit **0** when identical, **1** when any path or chunk-set
  difference; `--max-paths N` caps path lists — see `docs/diff.md`
- **`diff --tree <src-dir> <listing.cfdir>`**: tree↔listing compare (ephemeral
  in-memory `DirArchive` from regular files); read-only — does **not** write
  store or `.cfdir`
- **`chunkforge store scrub`** (Phase 7 P0): traverse local loose `.cnk`,
  re-BLAKE3 via `get_verify`; report `ok=` / `corrupt=` / `unreadable=`;
  default read-only (does not delete); non-zero when corrupt/unreadable > 0;
  optional `--jobs` — see `docs/doctor-gc.md` scrub section
- **`archive --seed-trust-mtime`** (Phase 7 P1): with `--seed`, size+mtime
  match → reuse without content BLAKE3 (default **off** ≡ 0.6.0 content path);
  help + `docs/archive.md` warn about forged/incorrect mtimes
- **`extract --force`**: overwrite existing regular files at destination
  (type mismatches still error); without `--force` behaviour matches **0.6.0**
- **`scripts/demo_diff_scrub.sh`** + docs: local ~10 min smoke (two archives →
  `diff` / `diff --tree` → healthy scrub → flip one `.cnk` byte → scrub
  corrupt); `docs/diff.md`; doctor-gc scrub responsibility table

### Not delivered / deferred (Phase 7)

- In-process **SigV4** / complete **`aws-sdk-*`** / S3 multipart upload API
- Packfile / multi-chunk single object (loose `.cnk` layout unchanged)
- Write mount / COW / writable FUSE
- Bidirectional sync / watch directories / conflict resolution
- Video analysis / GPU·LLM / P2P
- **Remote scrub** / remote GC / lifecycle (scrub and `gc` stay **local**
  `--store` only)
- `push` still does **not** upload listings

### Non-goals (Phase 7)

- No rewrite of `.cfidx` / `.cfdir` v1 byte layouts
- No change to `ChunkSource` / `ChunkSink` method signatures
- No process-in SigV4 / `aws-sdk-*` / multipart; HTTP surface remains **ureq**
- No packfile; no write mount; no bidirectional sync; no video analysis;
  no remote scrub

### Notes

- Without new flags, `archive` / `extract` / verify / doctor / gc / push /
  pull behaviour matches **0.6.0**
- Default HTTP chunk layout remains byte-compatible with **0.6.0** / **0.5.0**
- Responsibilities: `verify` = listing + refs; `doctor` = presence; `gc` =
  unreferenced reclaim; `store scrub` = CAS bitrot; `diff` = listing/tree
  compare (report only)

## [0.6.0] — 2026-09-29

Phase 6 closeout: incremental directory archive (`archive --seed`), CAS fill
(`chunkforge pull`), per-file `archive --jobs`, and `scripts/demo_seed.sh`.
`.cfidx` / `.cfdir` v1 bytes and `ChunkSource` / `ChunkSink` signatures stay
frozen; no AWS SDK, in-process SigV4, write mount, packfile, or bidirectional
sync.

### Added

- **`archive --seed <prior.cfdir>`** (Phase 6 P0): incremental directory
  archive — reuse unchanged files' chunk tables via content BLAKE3 (size
  fast-reject); stderr `seed_reused_files=` / `rechunked_files=`; missing
  reused chunks force rechunk + `seed_missing_chunks=`; dry-run prints
  `would_seed_reuse=` / `would_rechunk=`; output remains full `.cfdir` v1 —
  see `docs/archive.md`
- **`archive --jobs N`** (Phase 6 P1): per-file parallel chunking (default
  **1** ≡ serial); seed map read-only; local store puts stay atomic /
  race-safe
- **`chunkforge pull`** (Phase 6 P1): fill a local CAS `--store` with missing
  chunks referenced by `.cfidx` / `.cfdir` listings from `--source`
  (path / `file://` / `http(s)://`); `--jobs` / `--dry-run`; stderr
  `skipped=` / `fetched=` / `failed=`; does not extract trees or upload/download
  listings — see `docs/pull.md`
- **`scripts/demo_seed.sh`** + docs: local ~10 min smoke (archive → change one
  file → `--seed` → verify / extract / optional `pull` via `put_stub`);
  `docs/archive.md` seed section; `docs/pull.md`

### Not delivered / deferred (Phase 6)

- **`--seed-trust-mtime`**: mtime-only “unchanged” shortcut not shipped; seed
  always content-BLAKE3 (size fast-reject)
- In-process **SigV4** / complete **`aws-sdk-*`** / S3 multipart upload API
- Write mount / COW / writable FUSE
- Bidirectional sync / watch directories / conflict resolution
- Packfile / multi-chunk single object (loose `.cnk` layout unchanged)
- Video analysis / GPU·LLM / P2P
- `push` still does **not** upload listings

### Non-goals (Phase 6)

- No rewrite of `.cfidx` / `.cfdir` v1 byte layouts
- No change to `ChunkSource` / `ChunkSink` method signatures
- No process-in SigV4 / `aws-sdk-*` / multipart; HTTP surface remains **ureq**
- No `--seed-trust-mtime`; no write mount; no bidirectional sync; no packfile;
  no video analysis

### Notes

- Without `--seed`, `archive` (+ `--dry-run`) behaviour matches **0.5.0**
- Default HTTP chunk layout remains byte-compatible with **0.5.0** / **0.4.0**

## [0.5.0] — 2026-09-29

Phase 5 closeout: directory-tree archive (`.cfdir` v1) with `archive` /
`extract` / tree `verify`, read-only FUSE directory mount (`DirFs`), and
`.cfdir`-aware `push` / `doctor` / `gc`, plus `push --verify` and
`archive --dry-run`. Single-blob `.cfidx` v1 stays frozen; no AWS SDK,
in-process SigV4, write mount, or seed archive.

### Added

- **`.cfdir` v1 + `DirArchive`**: parallel multi-file listing format
  (`CFDIR\0\0\x01`); encode/decode + path validation in `chunkforge-index`;
  `.cfidx` v1 bytes unchanged — see `docs/dir-format.md`
- **`chunkforge archive`**: recurse a source directory, FastCDC + BLAKE3 per
  regular file, write chunks into `--store` (dedup), emit `.cfdir`; stderr
  stats `files` / `chunks` / `new=` / `reused=`; symlinks and special files
  skipped with a warning (P0)
- **`archive --dry-run`**: stats only — no store writes and no `.cfdir` output
- **`chunkforge extract`** + **`verify` magic dispatch**: materialize tree
  (create parents; refuse existing targets); `verify` auto-detects `.cfidx`
  vs `.cfdir` (structure + per-file `blob_blake3` + missing chunk fails with
  id); `--jobs` on extract/verify chunk fetches (default 1)
- **Read-only FUSE directory mount (`DirFs`)**: `mount` magic-dispatches
  `.cfdir` → directory tree vs `.cfidx` → single blob; still forced `RO`
- **`push` / `doctor` / `gc` accept `.cfdir`**: reference set is the union of
  chunk ids (`DirArchive::all_chunk_ids`); listings are never uploaded
- **`push --verify`**: after a successful push (`failed=0`), treat `--dest`
  (same HTTP templates) as `ChunkSource` and verify each listing; skipped on
  `--dry-run` or when push already failed (clears Phase 4 O3)
- Docs: `docs/dir-format.md`, `docs/archive.md`; mount / push / doctor-gc
  updates; `scripts/demo_archive.sh`

### Not delivered / deferred (Phase 5)

- **`archive --seed prior.cfdir`** (spec O3): incremental seed / skip re-chunk
  when `blob_blake3` unchanged — deferred (Phase 5.5 / later)
- In-process **SigV4** / complete **`aws-sdk-*`** / S3 multipart upload API
- Write mount / COW / writable FUSE; bidirectional sync / watch directories
- casync `.catar` / `.caibx` bit-compat; video analysis / GPU·LLM / P2P

### Non-goals (Phase 5)

- No rewrite or deprecation of `.cfidx` v1 (single-blob index stays frozen)
- No casync `.catar` bit-compat (semantic alignment only; format is native)
- No write mount / COW; directory mount stays `RO`
- No bidirectional sync / conflict resolution
- No process-in SigV4 / `aws-sdk-*` / multipart; HTTP surface remains **ureq**
- No seed archive in this release; no remote GC / packfiles

### Notes

- Default HTTP chunk layout remains byte-compatible with **0.4.0** / **0.3.0**
  when no template flags are set — `.cfdir` push keys match subsequent
  `verify --source` / `push --verify`
- `make` / single-file `mount` / `.cfidx` `push` behaviour matches **0.4.0**
  when no new flags are used

## [0.4.0] — 2026-09-29

Phase 4 closeout: per-chunk HTTP PUT write path (`ChunkSink` / `HttpChunkSink` /
`chunkforge push`) plus bounded concurrency `--jobs` on read and push commands,
without an AWS SDK, in-process SigV4, or S3 multipart upload API.

### Added

- **`ChunkSink`** write trait in `chunkforge-store` (`has` + `put` → `PutOutcome::{Written,SkippedExists}`); `Store` implements it; **not** folded into read-only `ChunkSource`
- **`HttpChunkSink`** in `chunkforge-remote`: single-object PUT isomorphic with `HttpChunkSource` URL/header templates (`{base}` `{path}` `{2hex}` `{62hex}` `{id}`/`{hex}` `{prefix}` `{env:NAME}`); default `{base}/{path}`; plaintext body; optional `verify_hash`; 2xx / 409 success; ureq only
- **`chunkforge push`**: `--store` + one or more `.cfidx` + `--dest` HTTP(S); merge referenced ids → remote `has` skip / `put`; `--dry-run`; stderr stats `skipped=` / `uploaded=` / `failed=`; non-zero on failures; does **not** upload indexes
- **`--jobs N`** on `cat` / `verify` / `doctor` / `push`: bounded concurrency via `std::thread::scope` in the CLI orchestration layer. Default **`1`** preserves 0.3.0 serial behaviour; errors still include the chunk id. FUSE mount concurrency unchanged; no tokio
- Docs: `docs/push.md`; `docs/remote-layout.md` PUT section; `scripts/demo_push.sh` + `scripts/put_stub.py`

### Not delivered (Phase 4)

- **`push --verify`** (spec O3): not implemented in 0.4.0 — verify after push
  with a separate `chunkforge verify --source <dest> …`. **Delivered in 0.5.0.**

### Non-goals (Phase 4)

- No complete S3 multipart upload API (Initiate / UploadPart / Complete / Abort)
- No `aws-sdk-*` / `aws-config` / ListObjects / credential-provider chain
- No in-process SigV4 (GET or PUT); template headers / external presign / open write endpoints only
- No directory-tree archive / multi-blob container / casync `.catar`; `.cfidx` v1 stays single-blob
- No bidirectional sync / watch directories / conflict resolution
- No write mount / COW / writable FUSE (FUSE stays `RO`)
- No remote GC / bucket lifecycle; no packfile bundling; no P2P / GPU·LLM / video analysis
- macOS / Windows not acceptance platforms

### Notes

- Default HTTP layout remains byte-compatible with **0.3.0** / **0.2.0** when no template flags are set — push keys match subsequent `verify --source`
- Local store may use optional zstd; push decompresses to plaintext before PUT (remote layout = plaintext chunks)

## [0.3.0] — 2026-09-29

Phase 3 closeout: object-store–friendly read paths (URL/header templates + S3 path conventions) plus `doctor` and local `gc`, without an AWS SDK or in-process SigV4.

### Added

- **HTTP URL / header templates** on `HttpChunkSource`: `--url-template` / `--prefix` / `--header` (closed placeholders `{base}` `{path}` `{2hex}` `{62hex}` `{id}`/`{hex}` `{prefix}` `{env:NAME}`); default `{base}/{path}` ≡ Phase 2 layout
- **S3-compatible path conventions** (read-only): default key `{prefix}chunks/{2hex}/{62hex}.cnk`; path-style preferred; virtual-host documented; no bucket field parsing — see `docs/remote-layout.md`
- CLI wiring: `cat` / `verify` / `mount` / `doctor` accept template flags for `http(s)://` sources (non-HTTP + templates → readable error)
- **`chunkforge doctor`**: `.cfidx` readability + chunk presence via `ChunkSource::has` (optional `--deep` uses `get`); missing ids on stdout + non-zero exit; optional HTTP base probe; local `meta.toml` summary
- **`chunkforge gc`**: local dry-run of unreferenced loose `.cnk` (merge chunk ids from given indexes; `Store::list_chunk_ids`); `--apply` deletes serially; **no remote GC**
- Docs: `docs/remote-layout.md` (placeholders, path-style / virtual-host, non-goals); `docs/doctor-gc.md`

### Non-goals (Phase 3)

- No `aws-sdk-*` / full S3 SDK; no in-process SigV4 (even GET-only)
- No upload / multipart / PUT / POST; no bidirectional sync; FUSE stays read-only
- No remote GC / object-storage lifecycle; no auto batch-presign per chunk

### Notes

- Default HTTP layout remains byte-compatible with **0.2.0** when no template flags are set
- Presigned URLs: query may appear in `url_template`; per-object differing signatures are out of scope

## [0.2.0] — 2026-09-29

Phase 2 closeout: read-only FUSE mount + remote chunk fetch skeleton on the Phase 1 local CAS.

### Added

- `ChunkSource` + `SourceError` in `chunkforge-store` (`impl` for `Store`); `CacheSource<P, S>` overlays a local cache store on miss
- `chunkforge-remote`: `HttpChunkSource` (ureq) + `FileUrlSource`; layout docs in `docs/remote-layout.md`
- CLI `cat` / `verify`: `--source` / `--cache` (Phase 1 `--store` retained as a local-path synonym)
- `chunkforge-fuse`: read-only single-blob FUSE library (`BlobFs`, `read_range`, `MountOption::RO` hard-coded); unit tests without `/dev/fuse`
- CLI `mount` (`--source` / `--store` / `--cache` / `--name`); `docs/mount.md`; `scripts/demo_mount.sh`; cargo feature `fuse` (default on Linux)
- Workspace crates: `chunkforge-remote`, `chunkforge-fuse`

### Non-goals (Phase 2)

- No bidirectional sync, write mount / COW write-back, full S3 SDK, or P2P
- No restic-style backup product; no directory-tree archive; macOS/Windows not acceptance platforms

### Notes

- Real FUSE mounts stay `#[ignore]` in CI (need fuse3 + `/dev/fuse`); library FS logic is always tested
- Optional `doctor` / `gc` deferred (not required for 0.2.0)

## [0.1.0] — 2026-09-29

Phase 1 MVP closeout: local content-addressed chunking with make / cat / verify.

### Added

- Workspace crates: `chunkforge-chunk`, `chunkforge-index`, `chunkforge-store`, `chunkforge-cli`
- FastCDC v2020 chunking + BLAKE3 content addressing (default 16KiB / 64KiB / 256KiB)
- Native `.cfidx` v1 index format (not casync bit-compatible)
- Local CAS store (`chunks/<2hex>/<62hex>.cnk`) with atomic put and optional zstd
- CLI: `make`, `cat`, `verify`, `chunk-id`, `store has`
- Fixtures + offline `scripts/gen_large.sh` and incremental dedup demo (`scripts/demo_dedup.sh`)
- GitHub Actions CI: fmt, clippy (`-D warnings`), `cargo test --workspace`

### Non-goals (Phase 1)

- Not a restic/syncthing replacement; no GPU/LLM; no FUSE; no remote/network store;
  no casync binary drop-in; no full directory-tree archive

[1.9.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v1.9.0
[1.8.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v1.8.0
[1.7.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v1.7.0
[1.6.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v1.6.0
[1.5.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v1.5.0
[1.4.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v1.4.0
[1.3.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v1.3.0
[1.2.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v1.2.0
[1.1.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v1.1.0
[1.0.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v1.0.0
[0.9.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.9.0
[0.8.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.8.0
[0.7.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.7.0
[0.6.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.6.0
[0.5.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.5.0
[0.4.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.4.0
[0.3.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.3.0
[0.2.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.2.0
[0.1.0]: https://github.com/lljk8187-coder/ChunkForge/releases/tag/v0.1.0
