#!/usr/bin/env bash
# Phase 5 archive smoke: tree → archive → verify → extract → diff;
# optional FUSE mount; put_stub push → verify --source; .cfidx regression.
# Usage: ./scripts/demo_archive.sh
# Requires: cargo, python3. FUSE is optional (skipped gracefully).
# No real internet — only 127.0.0.1.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_ARCHIVE_DEMO_DIR:-/tmp/cf-archive-demo}"
BIN="${CHUNKFORGE_BIN:-}"
FIXTURE="${CHUNKFORGE_ARCHIVE_FIXTURE:-$ROOT/fixtures/hello.txt}"
PORT="${CHUNKFORGE_ARCHIVE_PORT:-8766}"
PYTHON="${CHUNKFORGE_PYTHON:-python3}"

if [[ ! -f "$FIXTURE" ]]; then
  echo "error: fixture not found: $FIXTURE" >&2
  exit 1
fi

if ! command -v "$PYTHON" >/dev/null 2>&1; then
  echo "error: $PYTHON not on PATH" >&2
  exit 1
fi

echo "==> building chunkforge"
cargo build -p chunkforge-cli --quiet
if [[ -z "$BIN" ]]; then
  BIN="$ROOT/target/debug/chunkforge"
fi

rm -rf "$DEMO_DIR"
SRC="$DEMO_DIR/src"
STORE="$DEMO_DIR/store"
OUT="$DEMO_DIR/out"
MIRROR="$DEMO_DIR/mirror"
CFDIR="$DEMO_DIR/release.cfdir"
CFIDX="$DEMO_DIR/hello.cfidx"
MNT="$DEMO_DIR/mnt"
mkdir -p "$SRC/sub" "$STORE" "$MIRROR" "$MNT"

echo 'hello-tree' > "$SRC/a.txt"
cp "$FIXTURE" "$SRC/sub/b.txt"
cp "$SRC/a.txt" "$SRC/a-copy.txt"

STUB_PID=""
MOUNT_PID=""
cleanup() {
  if [[ -n "$MOUNT_PID" ]] && kill -0 "$MOUNT_PID" 2>/dev/null; then
    fusermount3 -u "$MNT" 2>/dev/null || fusermount -u "$MNT" 2>/dev/null || true
    wait "$MOUNT_PID" 2>/dev/null || true
  fi
  if mountpoint -q "$MNT" 2>/dev/null; then
    fusermount3 -u "$MNT" 2>/dev/null || fusermount -u "$MNT" 2>/dev/null || true
  fi
  if [[ -n "$STUB_PID" ]] && kill -0 "$STUB_PID" 2>/dev/null; then
    kill "$STUB_PID" 2>/dev/null || true
    wait "$STUB_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo
echo "==> archive $SRC → $CFDIR"
"$BIN" archive --store "$STORE" -o "$CFDIR" "$SRC"

echo
echo "==> verify .cfdir (local store)"
"$BIN" verify --store "$STORE" "$CFDIR"

echo
echo "==> extract → $OUT + diff -qr"
"$BIN" extract --store "$STORE" "$CFDIR" -o "$OUT"
diff -qr "$SRC" "$OUT"
echo "diff: OK"

# Optional FUSE directory mount
echo
if [[ "$(uname -s)" == "Linux" ]] \
  && [[ -e /dev/fuse ]] \
  && { command -v fusermount3 >/dev/null 2>&1 || command -v fusermount >/dev/null 2>&1; }; then
  echo "==> mount .cfdir (optional FUSE)"
  "$BIN" mount --store "$STORE" "$CFDIR" "$MNT" &
  MOUNT_PID=$!
  for _ in $(seq 1 50); do
    if [[ -f "$MNT/a.txt" ]]; then
      break
    fi
    if ! kill -0 "$MOUNT_PID" 2>/dev/null; then
      wait "$MOUNT_PID" || true
      echo "warning: mount exited early — skipping FUSE checks" >&2
      MOUNT_PID=""
      break
    fi
    sleep 0.1
  done
  if [[ -n "$MOUNT_PID" ]] && [[ -f "$MNT/a.txt" ]]; then
    cmp "$SRC/a.txt" "$MNT/a.txt"
    cmp "$SRC/sub/b.txt" "$MNT/sub/b.txt"
    echo "mount cmp: OK"
    fusermount3 -u "$MNT" 2>/dev/null || fusermount -u "$MNT"
    wait "$MOUNT_PID" 2>/dev/null || true
    MOUNT_PID=""
  else
    echo "warning: timed out waiting for mount — skipping FUSE checks" >&2
    if [[ -n "$MOUNT_PID" ]]; then
      kill "$MOUNT_PID" 2>/dev/null || true
      wait "$MOUNT_PID" 2>/dev/null || true
      MOUNT_PID=""
    fi
  fi
else
  echo "==> skipping FUSE mount (Linux+/dev/fuse/fusermount required)"
fi

echo
echo "==> start put_stub on 127.0.0.1:${PORT} (root=$MIRROR)"
"$PYTHON" "$ROOT/scripts/put_stub.py" --root "$MIRROR" --port "$PORT" \
  >"$DEMO_DIR/stub.log" 2>&1 &
STUB_PID=$!

for _ in $(seq 1 50); do
  if ! kill -0 "$STUB_PID" 2>/dev/null; then
    wait "$STUB_PID" || true
    echo "error: put_stub exited early; log:" >&2
    cat "$DEMO_DIR/stub.log" >&2 || true
    exit 1
  fi
  if "$PYTHON" -c "import socket; s=socket.create_connection(('127.0.0.1',$PORT),1); s.close()" 2>/dev/null; then
    break
  fi
  sleep 0.1
done
if ! "$PYTHON" -c "import socket; s=socket.create_connection(('127.0.0.1',$PORT),1); s.close()" 2>/dev/null; then
  echo "error: timed out waiting for put_stub on port $PORT" >&2
  cat "$DEMO_DIR/stub.log" >&2 || true
  exit 1
fi

DEST="http://127.0.0.1:${PORT}"

echo
echo "==> push .cfdir → $DEST"
PUSH1="$("$BIN" push --store "$STORE" --dest "$DEST" --url-template '{base}/{path}' "$CFDIR" 2>&1)"
echo "$PUSH1"
if ! echo "$PUSH1" | grep -Eq 'uploaded=[1-9][0-9]*'; then
  echo "error: expected uploaded≥1 on first .cfdir push" >&2
  exit 1
fi
if ! echo "$PUSH1" | grep -Eq 'failed=0'; then
  echo "error: expected failed=0 on first .cfdir push" >&2
  exit 1
fi

echo
echo "==> verify --source $DEST (.cfdir)"
"$BIN" verify --source "$DEST" "$CFDIR"

# Confirm no listing file landed under the mirror
if find "$MIRROR" -type f \( -name '*.cfdir' -o -name '*.cfidx' \) 2>/dev/null | grep -q .; then
  echo "error: unexpected listing under mirror (push must upload chunks only)" >&2
  find "$MIRROR" -type f \( -name '*.cfdir' -o -name '*.cfidx' \) >&2
  exit 1
fi

echo
echo "==> .cfidx make/verify still works (compat)"
"$BIN" make --store "$STORE" -o "$CFIDX" "$FIXTURE"
"$BIN" verify --store "$STORE" "$CFIDX"

echo
echo "==> doctor / gc accept .cfdir"
"$BIN" doctor --store "$STORE" "$CFDIR" "$CFIDX"
"$BIN" gc --store "$STORE" "$CFDIR" "$CFIDX" >/dev/null

echo
echo "archive demo OK"
echo "  store=$STORE"
echo "  cfdir=$CFDIR (kept local; not uploaded)"
echo "  cfidx=$CFIDX"
echo "  dest=$DEST"
echo "  mirror=$MIRROR"
echo "  compared=$SRC ↔ $OUT"
