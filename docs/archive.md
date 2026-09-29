# Directory archive (`archive` / `extract` / tree `verify` / push)

Phase 5 multi-file workflow built on [`.cfdir` v1](dir-format.md). Single-blob
[`.cfidx`](index-format.md) commands (`make` / `cat`) are unchanged.

## Commands

| Command | Role |
|---|---|
| `chunkforge archive --store <cas> -o out.cfdir <src-dir>` | Recursively chunk regular files into the local CAS; write a `.cfdir` listing |
| `chunkforge extract --store\|--source … archive.cfdir -o <out-dir>` | Materialize the tree (parents created; existing paths → non-zero) |
| `chunkforge verify --store\|--source … archive.cfdir` | Magic-dispatch: tree structure + per-file `blob_blake3` |
| `chunkforge mount --store\|--source … archive.cfdir <mnt>` | Read-only FUSE directory tree (see [mount.md](mount.md)) |
| `chunkforge push --store <cas> --dest http(s)://… archive.cfdir` | Upload **chunks only** referenced by the `.cfdir` |

`--jobs N` on extract / verify / push / doctor defaults to **1** (serial).

## Typical flow

```bash
cargo build -p chunkforge-cli

mkdir -p /tmp/cf-arch/src/sub
echo 'hello-tree' > /tmp/cf-arch/src/a.txt
cp fixtures/hello.txt /tmp/cf-arch/src/sub/b.txt
cp /tmp/cf-arch/src/a.txt /tmp/cf-arch/src/a-copy.txt   # cross-file dedup

./target/debug/chunkforge archive \
  --store /tmp/cf-arch/store -o /tmp/cf-arch/release.cfdir /tmp/cf-arch/src

./target/debug/chunkforge verify --store /tmp/cf-arch/store /tmp/cf-arch/release.cfdir
./target/debug/chunkforge extract --store /tmp/cf-arch/store \
  /tmp/cf-arch/release.cfdir -o /tmp/cf-arch/out
diff -qr /tmp/cf-arch/src /tmp/cf-arch/out
```

### Push chunks (listing stays local)

`push` never uploads the `.cfdir` / `.cfidx` file itself — only plaintext chunk
objects under the same layout as `verify --source` (see [push.md](push.md)).

```bash
# terminal 1
python3 scripts/put_stub.py --root /tmp/cf-arch/mirror --port 8766

# terminal 2
./target/debug/chunkforge push \
  --store /tmp/cf-arch/store \
  --dest http://127.0.0.1:8766 \
  /tmp/cf-arch/release.cfdir
./target/debug/chunkforge verify --source http://127.0.0.1:8766 \
  /tmp/cf-arch/release.cfdir
```

### Smoke script

```bash
./scripts/demo_archive.sh
```

Covers archive → verify → extract → diff, optional FUSE mount (skipped if fuse
unavailable), push via `put_stub.py` + `verify --source`, and a `.cfidx`
make/verify regression path.

## Archive policy (P0)

- **Regular files** only are recorded (optional empty `Dir` entries omitted).
- **Symlinks**: skipped with a stderr warning (not followed, not recorded).
- **fifo / socket / device**: skipped with a stderr warning.
- Chunk params default to FastCDC 16KiB / 64KiB / 256KiB; override with
  `--chunk-size min:avg:max`.

## Extract conflicts

If any destination path already exists, `extract` exits non-zero. There is no
`--force` yet — remove or choose a fresh `-o` directory.

## Related

- Format bytes: [dir-format.md](dir-format.md)
- FUSE directory mount: [mount.md](mount.md)
- Push / doctor / gc accept `.cfdir`: [push.md](push.md), [doctor-gc.md](doctor-gc.md)
