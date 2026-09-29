# Read-only FUSE mount

Phase 2 presents a single `.cfidx` blob as **one** regular file under a mount point.
Writes are rejected (`EROFS` / `EACCES`); there is no directory-tree archive and no write-back.

## Requirements

| Item | Notes |
|---|---|
| OS | **Linux** is first-class. macOS / Windows are not Phase 2 acceptance targets. |
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
  [--name <filename>] \
  <index.cfidx> <mountpoint>
```

`--store <path>` is a Phase 1 synonym for `--source <path>` (same as `cat` / `verify`).

- Mount point must be an **existing directory**.
- Under it appears **one** file: default name = index basename with `.cfidx` stripped
  (override with `--name`).
- Process stays in the foreground until unmount (`Ctrl-C`, or `fusermount3 -u <mountpoint>`).
- Options always include kernel **RO**; optional `--cache` fills a local store on miss
  (never writes the primary source).

### Local store example

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

### Smoke script

```bash
./scripts/demo_mount.sh
```

See also [remote-layout.md](remote-layout.md) for HTTP / `file://` chunk URLs.

## Unmount

Prefer:

```bash
fusermount3 -u /path/to/mnt
```

or interrupt the `chunkforge mount` process (`Ctrl-C`). With `AutoUnmount`, leaving the
session also tears down the mount when possible.

## Out of scope (Phase 2)

- Writable mounts / COW write-back
- Full directory-tree presentation
- macOS (macFUSE / Fuse-T) and native Windows as supported platforms
