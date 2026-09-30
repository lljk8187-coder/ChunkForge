# Read-only FUSE mount

Phase 2 presents a single `.cfidx` blob as **one** regular file under a mount
point. Phase 5 extends the same `mount` command to present a `.cfdir` as a
**directory tree**. Phase 10 adds **sequential chunk prefetch** (process-local;
distinct from Store `--cache`). Writes are rejected (`EROFS` / `EACCES`); there
is no write-back.

## Requirements

| Item | Notes |
|---|---|
| OS | **Linux** is first-class. macOS / Windows are not acceptance targets. |
| Build | CLI cargo feature `fuse` (on by default). Disable with `--no-default-features`. |
| Runtime | **fuse3** userspace helpers + a usable **`/dev/fuse`**. |

Debian / Ubuntu:

```bash
sudo apt install fuse3
ls -l /dev/fuse          # typically crw-rw-rw- or group `fuse`
which fusermount3
```

If mount fails with a permission or missing-device error, the CLI prints short hints
(install fuse3, check `/dev/fuse`, group membership).

## Usage

```text
chunkforge mount \
  --source <local-store|file:///path|http(s)://host/base> \
  [--fallback <PATH|URL>]... \
  [--cache <local-cache-store>] \
  [--cache-max-bytes N|1M|64Mi|…] \
  [--name <filename>] \
  [--no-prefetch] \
  [--prefetch-chunks N] \
  [--path <prefix>]... \
  [--path-from <file>]... \
  [--exclude <pat>]... \
  [--exclude-from <file>]... \
  <listing.cfidx|listing.cfdir> <mountpoint>
```

`--store <path>` is a Phase 1 synonym for `--source <path>` (same as `cat` / `verify`).

- Mount point must be an **existing directory**.
- Magic-dispatch on the listing header:
  - **`.cfidx`** → one virtual file (default name = stem without `.cfidx`;
    override with `--name`).
  - **`.cfdir`** → directory tree of archived relative paths (`--name` ignored).
- Process stays in the foreground until unmount (`Ctrl-C`, or
  `fusermount3 -u <mountpoint>`).
- Options always include kernel **RO**; optional `--cache` fills a local store
  on miss (never writes the primary source).
- **`--cache-max-bytes N`** (Phase 15 + Phase 16 suffixes): soft fill budget.
  Accepts a plain decimal integer **or** `<num>[K|M|G|Ki|Mi|Gi]` (1024-base,
  case-insensitive; no decimals; `KB`/`MB`/`GB` rejected). Requires `--cache`
  (without it → clear non-zero). Over budget **skips fill** but still serves
  primary data; **never** evicts / LRU / trim / GC / sync. Omit ≡ **1.4**
  unbounded fill. Orthogonal to prefetch / jobs / retries / SigV4 / `--format`
  / `--fallback`. Smoke:
  [`scripts/demo_cache_budget_ops_json.sh`](../scripts/demo_cache_budget_ops_json.sh),
  [`scripts/demo_fallback_bytes_suffix.sh`](../scripts/demo_fallback_bytes_suffix.sh).
- **`--fallback <PATH|URL>`** (Phase 16 / 1.6.0): repeatable; ordered
  Missing-only failover behind `--source`/`--store`. Zero times ≡ **1.5**
  single origin. With `--cache`, outer Cache wraps the **whole** Fallback
  chain. **≠ cache ≠ sync ≠ prune ≠ write-back** (see below).
- `--jobs` does **not** apply to mount.

### Path scope (Phase 21 / 1.11 opt-in; Cargo still 1.10.0 until M7)

Repeatable **`--path` / `--path-from` / `--exclude` / `--exclude-from`**
restrict which **`.cfdir` File** paths appear under the mount point (kept
Files + ancestor Dirs). Semantics match archive/extract/push/pull/diff/doctor/
verify (`PathFilter` / `load_path_file` / `load_exclude_file`). Library path:
`filter_dir_archive` → `DirFs::new` (empty filter ≡ identity ≡ **1.10** full
tree).

| Rule | Detail |
|---|---|
| Default | **No** path/exclude flags ⇒ **full tree** ≡ **1.10.0** |
| `.cfidx` | Any path/exclude/`--path-from`/`--exclude-from` flag → **clear non-zero** |
| Orthogonality | Works with `--fallback` / `--cache*` / prefetch / SigV4; still **RO** |
| Session | Mount stays session-typed — **no** `--format json`, **no** `--progress` |

**`mount path` ≠ write mount ≠ prune ≠ `gc --path` ≠ sync ≠ pack** — filtering
only **shows fewer** paths; it never writes back, never deletes extras under a
target tree, never shrinks the GC keep-set, and never packs chunks.

```bash
# Subset mount (packages/foo only)
chunkforge mount --store ./store --path packages/foo tree.cfdir /mnt/cf
# path-from + exclude-from combined
chunkforge mount --store ./store \
  --path-from include.txt --exclude-from exclude.txt \
  tree.cfdir /mnt/cf
```

Smoke: [`scripts/demo_mount_path.sh`](../scripts/demo_mount_path.sh)
(library DirFs / `filter_dir_archive` is the primary CI-friendly path; real
FUSE is optional when `/dev/fuse` + fuse3 are present).


### Sequential prefetch (Phase 10 + Phase 11 P1 O1)

Default: **prefetch on** (conservative). After a forward sequential `read` that
consumes into/past a chunk, the mount **best-effort** fetches up to **N**
subsequent index entries into a process-local cache (depth **N** **and** total
cached plaintext **≤ 512 KiB**, whichever stricter). Hits skip a synchronous
`ChunkSource::get` on the following sequential read.

| Behaviour | Detail |
|---|---|
| Default | Prefetch **on**, depth **1** ≡ 1.0.0 — result bytes identical; may do **fewer** sync gets on sequential streams |
| `--prefetch-chunks N` | Prefetch depth (default **1**; hard cap **≤2**; clap rejects `N` outside 1..=2). Mount only |
| `--no-prefetch` | Prefetch **off** ≡ 0.9.0 on-demand get; **takes priority** over `--prefetch-chunks` |
| Invalidation | Seek backward, non-contiguous offset, or cross-file (`DirFs` inode change) → **cold-start** the window (drop cached chunks) |
| Errors | Prefetch `get` failure **never** fails a read whose current range is already satisfied |
| vs `--cache` | Prefetch is **process-local / mount-lifetime** and does **not** persist; `--cache` is a disk `CacheSource` layer under `get` (optional `--cache-max-bytes` soft-refuses fill; not LRU) |

Library: `BlobFs`/`DirFs::with_prefetch(bool)` and `with_prefetch_chunks(n)`
(`PrefetchCache::enabled_with_max_chunks`; values `>2` clamp to 2).

True FUSE mount smoke tests may stay `#[ignore]` (need fuse3 + `/dev/fuse`);
prefetch algebra is covered by in-process unit tests that count `get` calls.

### Single-file (`.cfidx`) example

```bash
cargo build -p chunkforge-cli
mkdir -p /tmp/cf-mnt-demo/{store,mnt}
./target/debug/chunkforge make --store /tmp/cf-mnt-demo/store \
  -o /tmp/cf-mnt-demo/hello.cfidx ./fixtures/hello.txt
./target/debug/chunkforge mount --store /tmp/cf-mnt-demo/store \
  /tmp/cf-mnt-demo/hello.cfidx /tmp/cf-mnt-demo/mnt
# other terminal:
cmp ./fixtures/hello.txt /tmp/cf-mnt-demo/mnt/hello
fusermount3 -u /tmp/cf-mnt-demo/mnt
```

Disable prefetch (0.9.0-like on-demand gets):

```bash
./target/debug/chunkforge mount --store /tmp/cf-mnt-demo/store \
  --no-prefetch /tmp/cf-mnt-demo/hello.cfidx /tmp/cf-mnt-demo/mnt
```

Deeper prefetch (Phase 11; still ≤2 subsequent chunks / ≤512 KiB):

```bash
./target/debug/chunkforge mount --store /tmp/cf-mnt-demo/store \
  --prefetch-chunks 2 /tmp/cf-mnt-demo/hello.cfidx /tmp/cf-mnt-demo/mnt
```

### Directory-tree (`.cfdir`) mount

```bash
./target/debug/chunkforge archive --store /tmp/cf-mnt-demo/store \
  -o /tmp/cf-mnt-demo/release.cfdir /tmp/cf-mnt-demo/src
mkdir -p /tmp/cf-mnt-demo/mnt-tree
./target/debug/chunkforge mount --store /tmp/cf-mnt-demo/store \
  /tmp/cf-mnt-demo/release.cfdir /tmp/cf-mnt-demo/mnt-tree
# other terminal: tree /tmp/cf-mnt-demo/mnt-tree ; cmp files as needed
fusermount3 -u /tmp/cf-mnt-demo/mnt-tree
```

Under the mount point, relative paths from the `.cfdir` appear as directories and
regular files. File content is assembled from `ChunkSource::get`, with sequential
prefetch of subsequent chunk(s) when enabled (default depth 1).

### Smoke scripts

```bash
./scripts/demo_mount.sh           # .cfidx single-file smoke
./scripts/demo_archive.sh         # includes optional .cfdir mount (skips if fuse unavailable)
./scripts/demo_mount_path.sh      # Phase21 path quartet (DirFs lib + optional FUSE)
```

See also [remote-layout.md](remote-layout.md) for HTTP / `file://` chunk URLs and
[archive.md](archive.md) for the directory workflow.

## Unmount

Prefer:

```bash
fusermount3 -u /path/to/mnt
```

or interrupt the `chunkforge mount` process (`Ctrl-C`). With `AutoUnmount`, leaving the
session also tears down the mount when possible.


## `--fallback` ≠ `--cache` ≠ sync

| Mechanism | Writes disk? | Role |
|---|---|---|
| **`--fallback`** | **No** (read-only multi-origin) | On Missing, try next origin in CLI order; Transient/Corrupt fail fast |
| **`--cache`** (+ optional `--cache-max-bytes`) | **Yes** (cache store only) | Miss → `get` from chain → `put` into cache; over budget **refuse-fill**, still serves |
| Sync / prune / write mount | n/a | **Not implemented** — FUSE stays RO; fallback is not write-back, not bidirectional sync, not prune |
| **`--path` / `--path-from` / `--exclude` / `--exclude-from`** | **No** (filter listing before DirFs) | Subset **visibility** only; **≠** write mount **≠** prune **≠** `gc --path` **≠** sync **≠** pack |

Composition (recommended): `CacheSource(Fallback([primary, …fallbacks]), cache)`.

## Out of scope

- Writable mounts / COW write-back (**mount path is not write-back**)
- Prefetch depth beyond the hard cap (`N≤2` / ≤512 KiB)
- Treating `--fallback` as sync / prune / write-back / LRU
- `gc --path` / extract prune / `--delete` / bidirectional sync / packfile
- `mount --progress` done/TOTAL (session-typed; no natural TOTAL)
- macOS (macFUSE / Fuse-T) and native Windows as supported platforms
