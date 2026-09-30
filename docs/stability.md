# ChunkForge 1.0 stability surface

Phase 10 freezes the **1.0 commitments** below: what callers and scripts may
rely on across minor releases, what counts as a breaking change, and what this
project does **not** promise. **ChunkForge 1.0.0** is released (annotated tag
`v1.0.0`). **1.1.0** adds opt-in flags only (`extract --skip-trust-mtime`,
`extract`/`push`/`pull --format json`, `mount --prefetch-chunks N`); defaults
and the frozen surface stay ≡ **1.0.0**. The workspace reports **1.1.0**.

Cross-links: [index-format.md](index-format.md), [dir-format.md](dir-format.md),
[mount.md](mount.md), [perf.md](perf.md), [sigv4.md](sigv4.md),
[remote-layout.md](remote-layout.md).

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
`--no-prefetch`, `--skip-trust-mtime`, `--prefetch-chunks N`) may ship in
**minor** releases when defaults stay compatible. **1.1.0** is such a minor:
all new flags default off / text / depth 1 ≡ 1.0.0.

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
   `push` / `pull` / `diff` / `doctor` / `gc` / `store scrub` / `mount` with
   documented defaults

## Non-promises / not guaranteed

| Not promised | Notes |
|---|---|
| Absolute throughput SLA | [perf.md](perf.md) is a measurement recipe, not a CI gate |
| True mount in fuse-less CI | Real mount tests may stay `#[ignore]`; prefetch algebra is unit-tested |
| Cross-OS first-class support | Linux + fuse3 is the acceptance platform |
| casync `.catar` / `.caibx` bit-compat | Semantic alignment only; native formats |
| Remote scrub / remote GC | Use `verify --source` for referenced remote integrity; `doctor` for presence; local `store scrub` / `gc` only |
| Packfile / multi-chunk objects | Not implemented; promotion checklist stays in [perf.md](perf.md) |
| Write mount / COW / bidirectional sync | FUSE stays RO; `diff` / extract skip ≠ sync |
| Full AWS SDK, multipart, IMDS/SSO, byte-range resume, push listing upload | Explicit non-goals |

## Command responsibilities (no remote scrub, no sync)

| Command | Role |
|---|---|
| `verify` | Listing structure + referenced chunk integrity (incl. HTTP `verify_hash`) |
| `doctor` | Presence check (optional `--deep` = `get`) |
| `gc` | Local unreferenced loose chunks (dry-run / `--apply`) |
| `store scrub` | Local loose-chunk full BLAKE3 rehash |
| `diff` | Listing↔listing (+ `--tree`); not sync |
| `extract --skip-unchanged` / `--dry-run` / `--skip-trust-mtime` | Incremental / plan-only materialize; mtime trust is opt-in; **no** prune |
| `extract` / `push` / `pull --format json` | Ops JSON (default **text** ≡ 1.0.0); field rename is breaking |
| `mount` (+ prefetch / `--no-prefetch` / `--prefetch-chunks N`) | Read-only FUSE; sequential prefetch is RO UX only (default depth 1 ≡ 1.0.0) |

There is **no** remote-scrub first-class command and **no** bidirectional sync.
