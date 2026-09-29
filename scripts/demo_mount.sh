#!/usr/bin/env bash
# Local-store read-only FUSE smoke: mount → cmp → write-fail → unmount.
# Usage: ./scripts/demo_mount.sh
# Requires: cargo, fuse3 (/dev/fuse + fusermount3). Linux only.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_MOUNT_DEMO_DIR:-/tmp/cf-mount-demo}"
BIN="${CHUNKFORGE_BIN:-}"
FIXTURE="${CHUNKFORGE_MOUNT_FIXTURE:-$ROOT/fixtures/hello.txt}"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "error: demo_mount.sh is Linux-only (Phase 2 FUSE acceptance)" >&2
  exit 1
fi

if [[ ! -e /dev/fuse ]]; then
  echo "error: /dev/fuse missing — install fuse3 (Debian/Ubuntu: sudo apt install fuse3)" >&2
  exit 1
fi
if ! command -v fusermount3 >/dev/null 2>&1 && ! command -v fusermount >/dev/null 2>&1; then
  echo "error: fusermount3 not on PATH — install fuse3 (Debian/Ubuntu: sudo apt install fuse3)" >&2
  exit 1
fi

echo "==> building chunkforge (fuse feature)"
cargo build -p chunkforge-cli --features fuse --quiet
if [[ -z "$BIN" ]]; then
  BIN="$ROOT/target/debug/chunkforge"
fi

if [[ ! -f "$FIXTURE" ]]; then
  echo "error: fixture not found: $FIXTURE" >&2
  exit 1
fi

rm -rf "$DEMO_DIR"
STORE="$DEMO_DIR/store"
MNT="$DEMO_DIR/mnt"
IDX="$DEMO_DIR/hello.cfidx"
mkdir -p "$STORE" "$MNT"

echo
echo "==> make index from $(basename "$FIXTURE")"
"$BIN" make --store "$STORE" -o "$IDX" "$FIXTURE"

BLOB_NAME="$(basename "$IDX" .cfidx)"
VIRTUAL="$MNT/$BLOB_NAME"

cleanup() {
  if mountpoint -q "$MNT" 2>/dev/null || [[ -e "$VIRTUAL" ]]; then
    fusermount3 -u "$MNT" 2>/dev/null || fusermount -u "$MNT" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo
echo "==> mount --store $STORE $IDX $MNT"
"$BIN" mount --store "$STORE" "$IDX" "$MNT" &
MOUNT_PID=$!

# Wait for the virtual file to appear
for _ in $(seq 1 50); do
  if [[ -f "$VIRTUAL" ]]; then
    break
  fi
  if ! kill -0 "$MOUNT_PID" 2>/dev/null; then
    wait "$MOUNT_PID" || true
    echo "error: mount process exited before file appeared" >&2
    exit 1
  fi
  sleep 0.1
done

if [[ ! -f "$VIRTUAL" ]]; then
  echo "error: timed out waiting for $VIRTUAL" >&2
  kill "$MOUNT_PID" 2>/dev/null || true
  exit 1
fi

echo
echo "==> cmp virtual file vs original"
cmp "$FIXTURE" "$VIRTUAL"
echo "cmp: OK"

echo
echo "==> write attempt should fail (read-only)"
set +e
# Subshell so the open(RO) failure is captured into write.err (not the outer shell).
bash -c 'echo x >"$1"' _ "$VIRTUAL" >"$DEMO_DIR/write.out" 2>"$DEMO_DIR/write.err"
WRITE_RC=$?
set -e
if [[ "$WRITE_RC" -eq 0 ]]; then
  echo "error: write unexpectedly succeeded" >&2
  exit 1
fi
ERR_MSG="$(tr '\n' ' ' <"$DEMO_DIR/write.err" | sed 's/[[:space:]]*$//')"
echo "write failed as expected (exit $WRITE_RC): ${ERR_MSG:-Read-only file system}"

echo
echo "==> unmount"
fusermount3 -u "$MNT" 2>/dev/null || fusermount -u "$MNT"
# Mount process should exit after unmount
wait "$MOUNT_PID" 2>/dev/null || true
trap - EXIT

if [[ -e "$VIRTUAL" ]]; then
  # After unmount the path should not still look like our FUSE file.
  if mountpoint -q "$MNT" 2>/dev/null; then
    echo "error: mountpoint still mounted" >&2
    exit 1
  fi
fi

echo
echo "mount demo OK"
echo "  store=$STORE"
echo "  index=$IDX"
echo "  compared=$FIXTURE ↔ $MNT/$BLOB_NAME (while mounted)"
