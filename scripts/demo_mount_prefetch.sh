#!/usr/bin/env bash
# Phase 10 mount-prefetch smoke (G5): unit tests count ChunkSource::get for
# sequential reads — prefetch on skips re-get of the next chunk; --no-prefetch
# ≡ on-demand get counts. Optional real FUSE mount when /dev/fuse + fusermount3
# are available; otherwise SKIP real mount (exit 0 if unit tests passed).
# Local only — no real internet.
# Usage: ./scripts/demo_mount_prefetch.sh
# Requires: cargo. Optional: fuse3 (/dev/fuse + fusermount3) for real mount.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

BIN="${CHUNKFORGE_BIN:-}"
DEMO_DIR="${CHUNKFORGE_PREFETCH_DEMO_DIR:-/tmp/cf-mount-prefetch-demo}"
FIXTURE="${CHUNKFORGE_PREFETCH_FIXTURE:-$ROOT/fixtures/hello.txt}"

echo "==> Phase 10 demo_mount_prefetch"
echo "    Unit tests count ChunkSource::get via CountingSource:"
echo "    - prefetch on  → fewer gets (next chunk prefetched, not re-gotten)"
echo "    - --no-prefetch / with_prefetch(false) ≡ on-demand get path"

echo
echo "==> cargo test -p chunkforge-fuse --lib (prefetch get-count algebra)"
cargo test -p chunkforge-fuse --lib \
  sequential_read_prefetch_skips_reget_of_next_chunk -- --nocapture
cargo test -p chunkforge-fuse --lib \
  prefetch_disabled_get_count_equals_on_demand_path -- --nocapture

echo
echo "unit tests: OK (prefetch on skips reget; --no-prefetch ≡ on-demand)"

# Optional real mount path — skip (exit 0) when fuse is unavailable.
can_real_mount=1
if [[ "$(uname -s)" != "Linux" ]]; then
  echo
  echo "SKIP real mount: not Linux (uname=$(uname -s))"
  can_real_mount=0
elif [[ ! -e /dev/fuse ]]; then
  echo
  echo "SKIP real mount: /dev/fuse missing"
  can_real_mount=0
elif ! command -v fusermount3 >/dev/null 2>&1 && ! command -v fusermount >/dev/null 2>&1; then
  echo
  echo "SKIP real mount: fusermount3/fusermount not on PATH"
  can_real_mount=0
fi

if [[ "$can_real_mount" -eq 1 ]]; then
  echo
  echo "==> optional real mount smoke (prefetch default + --no-prefetch)"
  echo "==> building chunkforge-cli (fuse feature)"
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
  MNT_ON="$DEMO_DIR/mnt-on"
  MNT_OFF="$DEMO_DIR/mnt-off"
  IDX="$DEMO_DIR/hello.cfidx"
  mkdir -p "$STORE" "$MNT_ON" "$MNT_OFF"

  echo "==> make index from $(basename "$FIXTURE")"
  "$BIN" make --store "$STORE" -o "$IDX" "$FIXTURE"
  BLOB_NAME="$(basename "$IDX" .cfidx)"

  FUSERMOUNT=fusermount3
  command -v fusermount3 >/dev/null 2>&1 || FUSERMOUNT=fusermount

  cleanup() {
    if mountpoint -q "$MNT_ON" 2>/dev/null; then
      "$FUSERMOUNT" -u "$MNT_ON" 2>/dev/null || true
    fi
    if mountpoint -q "$MNT_OFF" 2>/dev/null; then
      "$FUSERMOUNT" -u "$MNT_OFF" 2>/dev/null || true
    fi
  }
  trap cleanup EXIT

  echo "==> mount (prefetch on, default) → cat → unmount"
  "$BIN" mount --store "$STORE" "$IDX" "$MNT_ON" &
  MOUNT_PID=$!
  for _ in $(seq 1 50); do
    if [[ -e "$MNT_ON/$BLOB_NAME" ]]; then
      break
    fi
    if ! kill -0 "$MOUNT_PID" 2>/dev/null; then
      wait "$MOUNT_PID" || true
      echo "error: mount (prefetch on) exited early" >&2
      exit 1
    fi
    sleep 0.1
  done
  if [[ ! -e "$MNT_ON/$BLOB_NAME" ]]; then
    echo "error: mount path never appeared: $MNT_ON/$BLOB_NAME" >&2
    exit 1
  fi
  cmp -s "$FIXTURE" "$MNT_ON/$BLOB_NAME"
  echo "  prefetch-on cat: OK (bytes match fixture)"
  "$FUSERMOUNT" -u "$MNT_ON"
  wait "$MOUNT_PID" 2>/dev/null || true

  echo "==> mount --no-prefetch → cat → unmount"
  "$BIN" mount --store "$STORE" --no-prefetch "$IDX" "$MNT_OFF" &
  MOUNT_PID=$!
  for _ in $(seq 1 50); do
    if [[ -e "$MNT_OFF/$BLOB_NAME" ]]; then
      break
    fi
    if ! kill -0 "$MOUNT_PID" 2>/dev/null; then
      wait "$MOUNT_PID" || true
      echo "error: mount --no-prefetch exited early" >&2
      exit 1
    fi
    sleep 0.1
  done
  if [[ ! -e "$MNT_OFF/$BLOB_NAME" ]]; then
    echo "error: mount path never appeared: $MNT_OFF/$BLOB_NAME" >&2
    exit 1
  fi
  cmp -s "$FIXTURE" "$MNT_OFF/$BLOB_NAME"
  echo "  --no-prefetch cat: OK (bytes match fixture; ≡ on-demand gets)"
  "$FUSERMOUNT" -u "$MNT_OFF"
  wait "$MOUNT_PID" 2>/dev/null || true
  trap - EXIT
  echo "real mount: OK (result bytes identical; get-count algebra covered by unit tests)"
else
  echo "(get-count comparison lives in unit tests above; real mount not required for this demo)"
fi

echo
echo "OK: demo_mount_prefetch"
