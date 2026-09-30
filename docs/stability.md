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

Cross-links: [index-format.md](index-format.md), [dir-format.md](dir-format.md),
[mount.md](mount.md), [perf.md](perf.md), [sigv4.md](sigv4.md),
[remote-layout.md](remote-layout.md), [ops-json.md](ops-json.md).

## Frozen surface (1.0 commitments)

| Surface | Commitment |
|---|---|
| **`.cfidx` v1** | Byte layout frozen (`CFIDX\0\0\x01`, `format_version=1`). Incompatible changes → **major** (2.x). See [index-format.md](index-format.md). |
| **`.cfdir` v1** | Byte layout frozen (`CFDIR\0\0\x01`, `format_version=1`). Incompatible changes → **major** (2.x). See [dir-format.md](dir-format.md). |
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
`bytes_plaintext` / `--decode`, `--compression`, `archive`/`extract`/`make --progress`) may ship in
**minor** releases when defaults stay compatible. **1.1.0**, **1.2.0**,
**1.3.0**, **1.4.0**, **1.5.0**, **1.6.0**, and **1.7.0** are such minors: all new
flags default off / text / jobs=1 / depth 1 / no path filter / no cache-max /
no `--fallback` / create compression **none** / progress **off** ≡ prior release. Soft budget is **refuse-fill only** (≠ LRU ≠
trim ≠ GC ≠ sync).

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
| Packfile / multi-chunk objects | Not implemented; promotion checklist stays in [perf.md](perf.md); **1.5.0** / **1.6.0** / **1.7.0** still do not implement pack |
| Write mount / COW / bidirectional sync | FUSE stays RO; `diff` / extract skip ≠ sync; cache-max ≠ sync |
| Cache LRU / auto trim / `store trim` | Soft budget is **refuse-fill only**; never evicts `.cnk` |
| Full AWS SDK, multipart, IMDS/SSO, byte-range resume, push listing upload | Explicit non-goals |
| Default store zstd / HTTP Content-Encoding / `store recompress` | Create default stays **none**; HTTP body stays plaintext; no recompress |

## Command responsibilities (no remote scrub, no sync)

| Command | Role |
|---|---|
| `verify` | Listing structure + referenced chunk integrity (incl. HTTP `verify_hash`) |
| `doctor` | Presence check (optional `--deep` = `get`) |
| `gc` | Local unreferenced loose chunks (dry-run / `--apply`) |
| `store scrub` | Local loose-chunk full BLAKE3 rehash |
| `store stats` / `du` | Local chunk count + on-disk bytes (observation; not trim) |
| `diff` | Listing↔listing (+ `--tree`); optional `--path`/`--exclude`/`--exclude-from` (narrow before compare; default ≡ full); not sync |
| `extract --skip-unchanged` / `--dry-run` / `--skip-trust-mtime` | Incremental / plan-only materialize; mtime trust is opt-in; **no** prune |
| `diff` / `verify` / `doctor` / `extract` / `push` / `pull` / `gc` / `store scrub` / `make` / `cat --format json` | Ops JSON (default **text**); field rename is breaking — see [Ops JSON field matrix](ops-json.md) |
| `mount` (+ prefetch / `--no-prefetch` / `--prefetch-chunks N` / `--cache-max-bytes`) | Read-only FUSE; sequential prefetch is RO UX only (default depth 1 ≡ 1.0.0); `--cache-max-bytes` = refuse-fill (≠ LRU) |
| `cat` / `verify` / `extract` / `mount --cache-max-bytes` | Soft fill budget with `--cache`; human suffixes (`1M` …) accepted (Phase 16); omit ≡ 1.4 unbounded; **≠ LRU ≠ trim ≠ GC ≠ sync** |
| `cat` / `verify` / `extract` / `mount` / `pull` / `doctor --fallback` | Ordered Missing-only failover behind primary; **≠ cache fill ≠ sync ≠ prune ≠ write-back**; zero times ≡ 1.5 single origin |
| `store stats` `bytes_plaintext` / `--decode` | Observation: none ⇒ plaintext ≡ on_disk; zstd needs `--decode`; **≠ trim ≠ LRU** |
| `make` / `archive --compression` | Create-time store meta only (`none`\|`zstd`; omit ≡ **none** ≡ 1.6); existing store opens by meta; **≠ wire compression ≠ pack ≠ LRU** |
| `archive` / `extract` / `make --progress` | Opt-in stderr `progress: op=…`; default **off** ≡ 1.6; **orthogonal** to `--format json` |

There is **no** remote-scrub first-class command and **no** bidirectional sync.


## Ops JSON field matrix

Stable `--format json` fields for ops commands live in
**[ops-json.md](ops-json.md)** (one row per command: `archive` / `diff` /
`verify` / `doctor` / `extract` / `push` / `pull` / `gc` / `store scrub` /
`store stats` / **`make`** / **`cat`**).
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
(1.6 `--fallback` / suffixes / `bytes_plaintext`/`--decode`; calls 1_4).
No absolute perf SLA.
