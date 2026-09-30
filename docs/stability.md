# ChunkForge 1.0 stability surface

Phase 10 freezes the **1.0 commitments** below: what callers and scripts may
rely on across minor releases, what counts as a breaking change, and what this
project does **not** promise. **ChunkForge 1.0.0** is released (annotated tag
`v1.0.0`). **1.1.0** adds opt-in flags only (`extract --skip-trust-mtime`,
`extract`/`push`/`pull --format json`, `mount --prefetch-chunks N`); defaults
and the frozen surface stay ≡ **1.0.0**. **1.2.0** adds further opt-in only
(`gc --jobs`, `gc`/`store scrub --format json`, ops-json matrix,
`check_compat_1_1`, `--progress`); defaults stay ≡ **1.1.0**. The workspace
reports **1.2.0**. **1.3.0** adds further opt-in only
(`archive`/`extract`/`pull --path`/`--exclude`, `archive --format json`,
`check_compat_1_2`); defaults stay ≡ **1.2.0** (no path flags ⇒ full tree;
archive format default **text**). The workspace reports **1.3.0**. **1.4.0**
adds further opt-in only: `store stats`/`du` `--format json`,
`push --path`/`--exclude`/`--exclude-from`, `--exclude-from` on
archive/extract/pull, `check_compat_1_3`; defaults stay ≡ **1.3.0** (no new
flags ⇒ full reference set; jobs=1, retries=0, text, progress off). The
workspace reports **1.4.0**. **1.5.0** adds further opt-in only:
`--cache-max-bytes` (refuse-fill, not LRU), `make`/`cat --format json`,
ops-json make/cat rows finalized, `check_compat_1_4` (+ P1
`store scrub --listing`); defaults stay ≡ **1.4.0** (no max ⇒ unbounded
cache fill; make/cat default **text**; jobs=1, retries=0, text, progress
off). The workspace reports **1.5.0**. See [ops-json.md](ops-json.md) for
the expanded matrix (incl. **archive** / **store stats** / **make** /
**cat**).

**1.6.0** adds further opt-in only: repeatable **`--fallback`**
(Missing-only read failover; outer Cache wraps the whole chain),
human-friendly **`--cache-max-bytes`** suffixes (`1M` / `64Mi` / …; plain
integers still accepted ≡ 1.5), **`store stats` `bytes_plaintext`** (+ opt-in
**`--decode`** for zstd), **`check_compat_1_5`**, and P1 **`diff --path` /
`--exclude` / `--exclude-from`**. Defaults stay ≡ **1.5.0** (no `--fallback`
⇒ single origin; no cache-max ⇒ unbounded fill; no `--decode` ⇒ zstd does not
force full-store `get`; no diff path flags ⇒ full listing). The workspace
reports **1.6.0**.

**1.7.0** adds further **opt-in** only: create-time
**`--compression none|zstd`** on `make` / `archive` (default **omit ≡ `none`**
≡ 1.6 create), and **`archive` / `extract` / `make --progress`** (default
**off** ≡ 1.6). Disk zstd is orthogonal to HTTP **plaintext** PUT/GET bodies
and is **not** pack / wire Content-Encoding / LRU. Defaults stay ≡ **1.6.0**
(no compression flag ⇒ create `none`; no `--progress` ⇒ quiet stderr; jobs=1,
retries=0, SigV4 off, text, mount prefetch depth 1, no `--fallback` ⇒ single
origin). The workspace reports **1.7.0**.

**1.8.0** adds further **opt-in** only: **`pull --verify`** (post-success
verify against local `--store`; dry-run / failed pull skip; default **off** ≡
1.7), **`--cache-stats`** stderr observation + ops-json additive **`cache_*`**
fields when `--cache` (≠ LRU / trim), **`cat` / `verify --progress`**
(default **off** ≡ 1.7; reuse `ProgressReporter`; per listing chunk), and
P1 **`doctor --progress`**. Defaults stay ≡ **1.7.0** (no `--verify` on pull
⇒ quiet; no `--cache-stats` ⇒ no cache noise; no cat/verify/doctor
`--progress` ⇒ quiet; create compression **none**; jobs=1, retries=0, SigV4
off, text, mount prefetch depth 1, no `--fallback` ⇒ single origin). The
workspace reports **1.8.0**.

**1.9.0** adds further **opt-in** only: **`store create`**
(`--compression none|zstd`, default omit ≡ **none** ≡ 1.8 create;
**≠** recompress / trim / default zstd / pack), **`pull --compression`**
(create-time only for a new `--store`; omit ≡ none ≡ 1.8; existing store
opens by meta / explicit conflict → non-zero; dry-run never creates),
**`diff --progress`** (default **off** ≡ 1.8; stderr only; orthogonal to
`--format json`; TOTAL = filtered File-path union), and P1 honest
**`make --jobs`** (default **1** ≡ 1.8; FastCDC cut-points stay serial;
post-chunk store put / on-disk zstd encoding only — **not** parallel
FastCDC). Defaults stay ≡ **1.8.0** (no `store create` side effects on old
paths; omit pull `--compression` ⇒ create **none**; no `diff --progress` ⇒
quiet; `make --jobs` default **1**; create compression **none**; jobs=1,
retries=0, SigV4 off, text, mount prefetch depth 1, no `--fallback` ⇒ single
origin). The workspace reports **1.9.0**. **`check_compat_1_8.sh`** gates
1.9 flags (calls 1_7).

**1.10.0** adds further **opt-in** only: repeatable **`--path-from FILE`**
(UTF-8 one include prefix per line; discipline ≡ `--exclude-from`; library
`load_path_file`; merged OR with `--path`) on **archive / extract / push /
pull / diff / doctor / verify**, and full path quartet on **`doctor` /
`verify`** (default no flags ≡ **1.9** full set; `.cfidx` + any path flag →
non-zero; JSON field names unchanged, counts may shrink). P1: **`push`
local/`file://` dest** via Store as `ChunkSink` (single dest; create
compression **none**). **Hard ban:** **`gc --path`** (shrinking the keep-set
would mis-delete). Defaults stay ≡ **1.9.0**. Responsibility:
**`path-from` ≠ prune ≠ gc-path ≠ sync ≠ pack**. **`push` local/`file://`
dest ≠ `--fallback` / multi-dest**. The workspace reports **1.10.0**.
Gate **`check_compat_1_9.sh`** gates 1.10 flags (calls 1_8).

**1.11.0** adds further **opt-in** only: **`mount` path quartet**
(`--path` / `--exclude` / `--exclude-from` / `--path-from`) on read-only
`.cfdir` DirFs (library `filter_dir_archive`; empty filter ≡ identity ≡
**1.10** full tree; `.cfidx` + any path flag → non-zero). P1: **`push
--compression`** (local/`file://` dest **create**; omit ≡ none ≡ 1.10) and
**`store list`** (sorted hex / `--format json`; **≠** GC/scrub/trim/LRU).
**Hard ban unchanged:** **`gc --path`**, write mount, prune, pack, default
zstd, push `--fallback`, mount `--progress`. Responsibility:
**`mount path` ≠ write mount ≠ prune ≠ gc-path ≠ sync ≠ pack**. Defaults stay
≡ **1.10.0** (no mount path flags ⇒ full tree). The workspace reports
**1.11.0**. Gate **`check_compat_1_10.sh`** gates 1.11 flags (calls 1_9).

**1.12.0** (Phase22 closeout) adds further **opt-in** only: **`archive
--symlinks skip|record`** (default **`skip`** ≡ **1.11** skip+warn + default
write `format_version=1`); `--symlinks record` writes `DirEntryKind::Symlink`
/ `KIND_SYMLINK=3` and bumps listing to `format_version=2` when ≥1 Symlink is
present. Decode accepts `{1,2}`. Extract materializes Symlink; DirFs exposes
`readlink`; still **RO**. Path filter treats Symlink paths like Files.
Absolute targets → clear non-zero; **not** followed. P1: **`make --dry-run`**
(plan-only; omit ≡ real write; **≠** seed / pack / recompress). **Hard ban
unchanged** plus: **no default record**, no follow-walk, no fifo/xattr, no
offline bundle, no pack, no write mount, no prune, no `gc --path`.
Responsibility nail: **`archive --symlinks record` ≠ write mount ≠ follow
dir symlink ≠ pack ≠ offline bundle ≠ prune ≠ `gc --path` ≠ default
record**. Defaults stay ≡ **1.11.0**. The workspace reports **1.12.0**. Gate
**`check_compat_1_11.sh`** gates 1.12 flags (calls 1_10).

**1.13.0** (Phase23 closeout) adds further **opt-in** only: **`diff --tree
--symlinks skip|record`** (default **`skip`** ≡ **1.12.0** tree skip+warn;
**`record`** → ephemeral `DirEntryKind::Symlink` on the tree side;
absolute/empty target → non-zero; **not** followed; clap **`requires =
"tree"`**). P1: extract dry-run additive **`would_symlinks`** (`would_write`
still includes symlink ≡ 1.12; **≠** prune / sync / pack / write mount).
**Hard ban unchanged** plus: **no default record**, no follow-walk, no
fifo/xattr, no offline bundle, no pack, no write mount, no prune, no
`gc --path`. Responsibility nail: **`diff --tree --symlinks record` ≠ write
mount ≠ follow ≠ pack ≠ sync ≠ prune ≠ `gc --path` ≠ default record**.
Defaults stay ≡ **1.12.0**. The workspace reports **1.13.0**. Gate
**`check_compat_1_12.sh`** gates 1.13 flags (calls 1_11; asserts `diff
--symlinks` default skip ≡ 1.12 + thin archive→diff record identical; no
absolute perf SLA). See [diff.md](diff.md) and
[`scripts/demo_diff_tree_symlink.sh`](../scripts/demo_diff_tree_symlink.sh).

**1.14.0** (Phase24 closeout) adds further **opt-in** only: first-class
**`chunkforge filter`** — persist a path-scoped subset of an existing
`.cfdir` via library `filter_dir_archive` → `DirArchive::encode` → `-o`.
Empty path 四件套 ≡ **identity**. Path 四件套 + `--dry-run` / `--force` /
`--format text|json` (fields: `ok` / `dry_run` / `input` / `output` /
`files` / `dirs` / `symlinks` / `excluded`). Symlink keep → encode **v2**;
all Symlinks filtered → encode **v1**. **Does not** open a store, walk a
source tree, prune, or rewrite the input in place. P1: **`make --seed
<PRIOR.cfidx>`** / **`--seed-trust-mtime`** (omit `--seed` ≡ 1.13; Reuse
copies prior chunk table / skips FastCDC; missing store chunks → clear
non-zero; additive ops-json `seed_reused`; **≠** pack / **≠** recompress /
**≠** path) + mount CLI help File+Symlink honesty (runtime unchanged).
**Hard ban unchanged:** **`gc --path`**, write mount, prune, pack, default
zstd, push `--fallback`, mount `--progress`, default record. Responsibility
nail: **`filter` ≠ prune ≠ `gc --path` ≠ sync ≠ write mount ≠ pack ≠
`archive --path`** (latter needs a source-tree walk). Warning: feeding a
filtered listing to `gc` uses **that listing's** refs — still **no**
`gc --path`. Docs: [filter.md](filter.md) / [ops-json.md](ops-json.md);
smoke: [`scripts/demo_filter_listing.sh`](../scripts/demo_filter_listing.sh).
Gate **`check_compat_1_13.sh`** (calls 1_12; asserts `filter` + thin Symlink
keep; no absolute perf SLA). Defaults of existing commands stay ≡
**1.13.0**. The workspace reports **1.14.0**.

Cross-links: [index-format.md](index-format.md), [dir-format.md](dir-format.md),
[mount.md](mount.md), [filter.md](filter.md), [perf.md](perf.md), [sigv4.md](sigv4.md),
[remote-layout.md](remote-layout.md), [ops-json.md](ops-json.md).

## Frozen surface (1.0 commitments)

| Surface | Commitment |
|---|---|
| **`.cfidx` v1** | Byte layout frozen (`CFIDX\0\0\x01`, `format_version=1`). Incompatible changes → **major** (2.x). See [index-format.md](index-format.md). |
| **`.cfdir` v1** | Byte layout frozen (`CFDIR\0\0\x01`, `format_version=1`). Default write path (no Symlink) stays v1. Phase22 opt-in **v2** (`format_version=2` + `KIND_SYMLINK`) is a **minor** compatible extension (decode accepts 1\|2). Incompatible changes → **major** (2.x). See [dir-format.md](dir-format.md). |
| **`ChunkSource`** | Method signatures frozen: `has` / `get` only (plaintext). No silent add of `put` / `list` on this trait. |
| **`ChunkSink`** | Method signatures frozen: `has` / `put` → `PutOutcome::{Written,SkippedExists}`. Source and Sink stay separate. |
| **Loose CAS layout** | On-disk / default HTTP object path `chunks/<2hex>/<62hex>.cnk` is the frozen narrative. Default HTTP GET/PUT = `{base}/{path}` (see [remote-layout.md](remote-layout.md)). A future pack layout must be dual-mode and either a **major** bump or an explicit opt-in layout version. |
| **CLI defaults vs 0.9.0** | `jobs=1`; `http-retries=0`; SigV4 **off**; `diff` default **text**; `extract` **without** `--skip-unchanged` / `--dry-run` ≡ full / conflict semantics of **0.9.0**. |
| **Mount prefetch** | Default **prefetch on** (conservative). `--no-prefetch` ≡ 0.9.0 on-demand `get` (RO-compatible; result bytes unchanged). See [mount.md](mount.md). |

Opt-in flags and additive behaviour (e.g. `--skip-unchanged`, `--format json`,
`--no-prefetch`, `--skip-trust-mtime`, `--prefetch-chunks N`, `gc --jobs`,
`--progress`, `--path` / `--exclude` / `--exclude-from`, `archive --format json`,
`store stats`/`du`, `push --path`, `--cache-max-bytes`, `make`/`cat
--format json`, `--fallback`, cache-max human suffixes, `store stats`
`bytes_plaintext` / `--decode`, `--compression`, `archive`/`extract`/`make --progress`,
`pull --verify`, `--cache-stats` / ops-json `cache_*`, `cat`/`verify --progress`,
`store create`, `pull --compression`, `diff --progress`, `make --jobs`,
`--path-from`, `doctor`/`verify` path scope, `mount` path quartet,
`archive --symlinks record`, `diff --tree --symlinks record`) may ship in
**minor** releases when defaults stay compatible. **1.1.0**, **1.2.0**,
**1.3.0**, **1.4.0**, **1.5.0**, **1.6.0**, **1.7.0**, **1.8.0** (Phase18),
**1.9.0** (Phase19), **1.10.0** (Phase20), **1.11.0** (Phase21),
**1.12.0** (Phase22 symlink opt-in), **1.13.0** (Phase23
`diff --tree --symlinks` + P1 `would_symlinks`), and **1.14.0** (Phase24
`chunkforge filter` + P1 `make --seed` / mount help Symlink honesty) are such
minors: all new
flags default off / text / jobs=1 / depth 1 / no path filter / no cache-max /
no `--fallback` / create compression **none** / progress **off** / no pull
`--verify` / no `--cache-stats` / no `store create` side effects on old paths /
omit pull `--compression` ≡ create none / no `diff --progress` ≡ prior release /
`make --jobs` default **1** / no `--path-from` / no doctor·verify path flags ≡
1.9 full set / no mount path flags ≡ 1.10 full tree / **`--symlinks skip`** ≡
1.11 skip+warn + default write v1 / **`diff --tree --symlinks skip`** ≡
1.12 tree skip+warn (no silent default record). Soft budget is **refuse-fill only** (≠ LRU ≠
trim ≠ GC ≠ sync). Cache observation counters are **observation only** (≠ LRU).
**`store create` ≠ recompress ≠ default zstd ≠ pack**.
**`path-from` ≠ prune ≠ gc-path ≠ sync ≠ pack**.
**`mount path` ≠ write mount ≠ prune ≠ gc-path ≠ sync ≠ pack**.
**`archive --symlinks record` ≠ write mount ≠ follow ≠ pack ≠ offline bundle ≠
prune ≠ `gc --path` ≠ default record**.
**`diff --tree --symlinks record` ≠ write mount ≠ follow ≠ pack ≠ sync ≠
prune ≠ `gc --path` ≠ default record**.
**`filter` ≠ prune ≠ `gc --path` ≠ sync ≠ write mount ≠ pack ≠ `archive --path`**.
**`make --seed` ≠ pack ≠ recompress ≠ path**.
Omit make `--seed` ≡ 1.13 make; **`filter`** is additive (new subcommand).

## Breaking-change policy

Treat as **breaking** (require a **major** bump, or an explicit breaking note in
the release notes / CHANGELOG):

- Removing a subcommand or a documented public flag
- Changing a product **default** (jobs, retries, SigV4, diff format, extract
  without flags, mount prefetch default in a way that changes result bytes)
- Renaming stable JSON field names (when a command exposes `--format json`)
- Changing listing (`.cfidx` / `.cfdir` v1) on-wire bytes incompatibly
- Changing `ChunkSource` / `ChunkSink` method signatures incompatibly
- Replacing loose `chunks/<2hex>/<62hex>.cnk` as the only supported layout
  without dual-mode / opt-in

Additive opt-in flags, new subcommands that do not alter existing defaults, and
documentation-only updates are **minor** (or patch).

## 1.0 promises

At 1.0, ChunkForge promises:

1. The **frozen surface** in the table above
2. **Read-only** FUSE mount of `.cfidx` (single blob) and `.cfdir` (tree),
   including sequential prefetch with `--no-prefetch` to match 0.9.0 on-demand
   gets ([mount.md](mount.md))
3. **Loose CAS** store + HTTP templates (`ureq`); optional minimal SigV4
   (`--aws-sigv4`, env + shared credentials file) — **no** `aws-sdk-*`
   ([sigv4.md](sigv4.md))
4. Stable one-way ops: `make` / `archive` / `extract` / `cat` / `verify` /
   `push` / `pull` / `diff` / `doctor` / `gc` / `store scrub` /
   `store stats`/`du` / `mount` with documented defaults (1.4 adds stats as
   opt-in observation; defaults unchanged)

## Non-promises / not guaranteed

| Not promised | Notes |
|---|---|
| Absolute throughput SLA | [perf.md](perf.md) is a measurement recipe, not a CI gate |
| True mount in fuse-less CI | Real mount tests may stay `#[ignore]`; prefetch algebra is unit-tested |
| Cross-OS first-class support | Linux + fuse3 is the acceptance platform |
| casync `.catar` / `.caibx` bit-compat | Semantic alignment only; native formats |
| Remote scrub / remote GC | Use `verify --source` for referenced remote integrity; `doctor` for presence; local `store scrub` / `gc` only |
| Packfile / multi-chunk objects | Not implemented; promotion checklist stays in [perf.md](perf.md); **1.5.0**–**1.14.0** still do not implement pack |
| Write mount / COW / bidirectional sync | FUSE stays RO; `diff` / extract skip ≠ sync; cache-max ≠ sync |
| Cache LRU / auto trim / `store trim` | Soft budget is **refuse-fill only**; never evicts `.cnk` |
| Full AWS SDK, multipart, IMDS/SSO, byte-range resume, push listing upload | Explicit non-goals |
| Default store zstd / HTTP Content-Encoding / `store recompress` | Create default stays **none**; HTTP body stays plaintext; no recompress |

## Command responsibilities (no remote scrub, no sync)

| Command | Role |
|---|---|
| `verify` | Listing structure + referenced chunk integrity (incl. HTTP `verify_hash`); optional path filter (Phase20; ≠ prune) |
| `doctor` | Presence check (optional `--deep` = `get`; optional path filter Phase20; ≠ `gc --path`) |
| `gc` | Local unreferenced loose chunks (dry-run / `--apply`); **no** `--path` (hard ban) |
| `store scrub` | Local loose-chunk full BLAKE3 rehash |
| `store stats` / `du` | Local chunk count + on-disk bytes (observation; not trim) |
| `diff` | Listing↔listing (+ `--tree`); File+Symlink compare; optional `--path`/`--exclude`/`--exclude-from` (narrow before compare; default ≡ full); Phase23 opt-in **`diff --tree --symlinks skip\|record`** (default skip ≡ 1.12; record → ephemeral Symlink; **≠** write mount **≠** follow **≠** pack **≠** sync **≠** prune **≠** `gc --path` **≠** default record); not sync |
| `extract --skip-unchanged` / `--dry-run` / `--skip-trust-mtime` | Incremental / plan-only materialize; mtime trust is opt-in; **no** prune |
| `diff` / `verify` / `doctor` / `extract` / `push` / `pull` / `gc` / `store scrub` / `make` / `cat --format json` | Ops JSON (default **text**); field rename is breaking — see [Ops JSON field matrix](ops-json.md) |
| `mount` (+ prefetch / `--no-prefetch` / `--prefetch-chunks N` / `--cache-max-bytes` / path quartet / Symlink `readlink`) | Read-only FUSE; sequential prefetch is RO UX only (default depth 1 ≡ 1.0.0); `--cache-max-bytes` = refuse-fill (≠ LRU); Phase21 path flags subset DirFs visibility (default ≡ 1.10 full tree; **≠** write mount **≠** prune **≠** `gc --path`); Phase22 Symlink nodes + `readlink` still **RO** (**record ≠ write mount ≠ follow**) |
| `archive --symlinks skip\|record` | Default **skip** ≡ 1.11 skip+warn + write v1; **record** → Symlink kind / v2 when ≥1; absolute target → non-zero; **≠** write mount **≠** follow **≠** pack **≠** prune **≠** `gc --path` **≠** default record |
| `filter` | Persist path-scoped subset of an existing `.cfdir` (`filter_dir_archive` → encode → `-o`); empty 四件套 ≡ identity; path 四件套 + `--dry-run` / `--force` / `--format`; Symlink keep → v2 / all filtered → v1; **≠** prune **≠** `gc --path` **≠** sync **≠** write mount **≠** pack **≠** `archive --path` (no source-tree walk). Warning: filtered listing → `gc` uses that listing's refs — still **no** `gc --path`. See [filter.md](filter.md) |
| `cat` / `verify` / `extract` / `mount --cache-max-bytes` | Soft fill budget with `--cache`; human suffixes (`1M` …) accepted (Phase 16); omit ≡ 1.4 unbounded; **≠ LRU ≠ trim ≠ GC ≠ sync** |
| `cat` / `verify` / `extract` / `mount` / `pull` / `doctor --fallback` | Ordered Missing-only failover behind primary; **≠ cache fill ≠ sync ≠ prune ≠ write-back**; zero times ≡ 1.5 single origin |
| `push` local / `file://` `--dest` | Single Store as `ChunkSink` (open or create **none**); **≠** `--fallback` / multi-dest; HTTP knobs with local dest → non-zero |
| `store stats` `bytes_plaintext` / `--decode` | Observation: none ⇒ plaintext ≡ on_disk; zstd needs `--decode`; **≠ trim ≠ LRU** |
| `make` / `archive --compression` | Create-time store meta only (`none`\|`zstd`; omit ≡ **none** ≡ 1.6); existing store opens by meta; **≠ wire compression ≠ pack ≠ LRU** |
| `archive` / `extract` / `make --progress` | Opt-in stderr `progress: op=…`; default **off** ≡ 1.6; **orthogonal** to `--format json` |
| `pull --verify` | Opt-in post-success verify of each listing against local `--store` (symmetric to `push --verify`); dry-run / failed pull **skip**; default **off** ≡ 1.7; **≠** sync |
| `--cache-stats` / ops-json `cache_*` | Opt-in CacheSource observation (`hits` / `miss_fills` / `miss_refused`); requires `--cache`; **≠ LRU ≠ trim**; default quiet ≡ 1.7 |
| `cat` / `verify --progress` | Opt-in stderr `progress: op=cat|verify done=N/TOTAL` per listing chunk; default **off** ≡ 1.7; **orthogonal** to `--format json` / `--jobs` / `--cache` / `--fallback` / `--cache-stats` |
| `store create` | Create empty local CAS (`Store::create`); `--compression none|zstd` (omit ≡ **none** ≡ 1.8); existing → non-zero; **≠** recompress / trim / default zstd / pack |
| `pull --compression` | Create-time only for new `--store` (same as make/archive/`store create`); omit ≡ none ≡ 1.8; existing by meta / conflict → non-zero; dry-run never creates |
| `make --jobs` | Opt-in post-chunk store put concurrency (default **1** ≡ 1.8); FastCDC stays serial; **not** parallel FastCDC |
| `diff --progress` | Opt-in stderr `progress: op=diff done=N/TOTAL` (filtered **File or Symlink** path union); default **off** ≡ 1.8; **orthogonal** to `--format json` |

There is **no** remote-scrub first-class command and **no** bidirectional sync.


## Ops JSON field matrix

Stable `--format json` fields for ops commands live in
**[ops-json.md](ops-json.md)** (one row per command: `archive` / `diff` /
`verify` / `doctor` / `extract` / `push` / `pull` / `gc` / `store scrub` /
`store stats` / **`store create`** / **`make`** / **`cat`** / **`filter`**
(Phase24)).
Default remains **text**. **Field rename → breaking** (same policy as above).
Path filter on extract/pull/push does **not** rename fields (`unique_chunks` =
filtered set). Prior command field names stay stable; **store stats** (1.4)
and **make** / **cat** (1.5) are additive only.

Compat gates: [`scripts/check_compat_1_0.sh`](../scripts/check_compat_1_0.sh)
(1.0 defaults), [`scripts/check_compat_1_1.sh`](../scripts/check_compat_1_1.sh)
(1.1/1.2 additive flags; calls 1_0),
[`scripts/check_compat_1_2.sh`](../scripts/check_compat_1_2.sh)
(1.3 path/archive flags; calls 1_1),
[`scripts/check_compat_1_3.sh`](../scripts/check_compat_1_3.sh)
(1.4 push path / store stats / exclude-from; calls 1_2), and
[`scripts/check_compat_1_4.sh`](../scripts/check_compat_1_4.sh)
(1.5 cache-max / make·cat format; calls 1_3), and
[`scripts/check_compat_1_5.sh`](../scripts/check_compat_1_5.sh)
(1.6 `--fallback` / suffixes / `bytes_plaintext`/`--decode`; calls 1_4),
[`scripts/check_compat_1_6.sh`](../scripts/check_compat_1_6.sh)
(1.7 `--compression` / archive·extract·make `--progress`; calls 1_5).
[`scripts/check_compat_1_7.sh`](../scripts/check_compat_1_7.sh)
(1.8 `pull --verify` / `--cache-stats` / cat·verify `--progress`; calls 1_6).
[`scripts/check_compat_1_8.sh`](../scripts/check_compat_1_8.sh)
(1.9 `store create` / `pull --compression` / `diff --progress`; calls 1_7).
[`scripts/check_compat_1_9.sh`](../scripts/check_compat_1_9.sh)
(1.10 `--path-from` / doctor·verify `--path`; no gc `--path`; calls 1_8).
[`scripts/check_compat_1_10.sh`](../scripts/check_compat_1_10.sh)
(1.11 mount path quartet; no gc `--path`; no mount `--progress`; calls 1_9).
[`scripts/check_compat_1_11.sh`](../scripts/check_compat_1_11.sh)
(1.12 archive `--symlinks` / demo_symlink; calls 1_10).
[`scripts/check_compat_1_12.sh`](../scripts/check_compat_1_12.sh)
(1.13 `diff --symlinks` / demo_diff_tree_symlink; calls 1_11).
**`check_compat_1_13.sh`** (Phase24; gates `filter` + 1.14 flags; calls
1_12).
No absolute perf SLA.
