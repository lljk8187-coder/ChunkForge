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
  [--cache <local-cache-store>] \
  [--cache-max-bytes N] \
  [--name <filename>] \
  [--no-prefetch] \
  [--prefetch-chunks N] \
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
- **`--cache-max-bytes N`** (Phase 15): soft fill budget in **bytes** (pure
  integer; no KiB suffix). Requires `--cache` (without it → clear non-zero).
  Over budget **skips fill** but still serves primary data; **never** evicts /
  LRU / trim / GC / sync. Omit ≡ **1.4** unbounded fill. Orthogonal to
  prefetch / jobs / retries / SigV4 / `--format`. Smoke:
  [`scripts/demo_cache_budget_ops_json.sh`](../scripts/demo_cache_budget_ops_json.sh).
- `--jobs` does **not** apply to mount.

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
./scripts/demo_mount.sh      # .cfidx single-file smoke
./scripts/demo_archive.sh    # includes optional .cfdir mount (skips if fuse unavailable)
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

## Out of scope

- Writable mounts / COW write-back
- Prefetch depth beyond the hard cap (`N≤2` / ≤512 KiB)
- macOS (macFUSE / Fuse-T) and native Windows as supported platforms
