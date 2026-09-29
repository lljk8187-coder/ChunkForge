#!/usr/bin/env bash
# Phase 6 seed smoke: tree → archive → verify → change one file →
# archive --seed → show seed_reused_files / rechunked_files → verify →
# extract+diff; optional pull via put_stub (local only).
# Usage: ./scripts/demo_seed.sh
# Requires: cargo, python3. No real internet — only 127.0.0.1 /tmp.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_SEED_DEMO_DIR:-/tmp/cf-seed-demo}"
BIN="${CHUNKFORGE_BIN:-}"
FIXTURE="${CHUNKFORGE_SEED_FIXTURE:-$ROOT/fixtures/hello.txt}"
PORT="${CHUNKFORGE_SEED_PORT:-8767}"
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
V1="$DEMO_DIR/v1.cfdir"
V2="$DEMO_DIR/v2.cfdir"
STORE2="$DEMO_DIR/store2"
mkdir -p "$SRC/sub" "$STORE" "$MIRROR"

echo 'hello-seed-v1' > "$SRC/a.txt"
cp "$FIXTURE" "$SRC/sub/b.txt"
cp "$SRC/a.txt" "$SRC/a-copy.txt"

STUB_PID=""
cleanup() {
  if [[ -n "$STUB_PID" ]] && kill -0 "$STUB_PID" 2>/dev/null; then
    kill "$STUB_PID" 2>/dev/null || true
    wait "$STUB_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo
echo "==> first archive $SRC → $V1"
ARCH1="$("$BIN" archive --store "$STORE" -o "$V1" "$SRC" 2>&1)"
echo "$ARCH1"
if echo "$ARCH1" | grep -Eq 'seed_reused_files='; then
  echo "error: first archive (no --seed) must omit seed counters" >&2
  exit 1
fi

echo
echo "==> verify v1"
"$BIN" verify --store "$STORE" "$V1"

echo
echo "==> change one file (a.txt)"
echo 'hello-seed-v2' > "$SRC/a.txt"

echo
echo "==> archive --seed $V1 → $V2 (expect mostly reuse, one rechunk)"
ARCH2="$("$BIN" archive --store "$STORE" -o "$V2" --seed "$V1" "$SRC" 2>&1)"
echo "$ARCH2"
# Expect 2 reused (a-copy.txt + sub/b.txt) and 1 rechunk (a.txt)
if ! echo "$ARCH2" | grep -Eq 'seed_reused_files=2'; then
  echo "error: expected seed_reused_files=2 (unchanged files); stderr above" >&2
  exit 1
fi
if ! echo "$ARCH2" | grep -Eq 'rechunked_files=1'; then
  echo "error: expected rechunked_files=1 (a.txt only); stderr above" >&2
  exit 1
fi

echo
echo "==> verify v2"
"$BIN" verify --store "$STORE" "$V2"

echo
echo "==> extract → $OUT + diff -qr"
"$BIN" extract --store "$STORE" "$V2" -o "$OUT"
diff -qr "$SRC" "$OUT"
echo "diff: OK"

echo
echo "==> dry-run --seed v2 on same tree (expect would_rechunk=0)"
DRY="$("$BIN" archive --store "$STORE" -o "$DEMO_DIR/unused.cfdir" \
  --seed "$V2" --dry-run "$SRC" 2>&1)"
echo "$DRY"
if ! echo "$DRY" | grep -Eq 'would_rechunk=0'; then
  echo "error: expected would_rechunk=0 on identical tree; stderr above" >&2
  exit 1
fi
if [[ -f "$DEMO_DIR/unused.cfdir" ]]; then
  echo "error: dry-run must not write .cfdir" >&2
  exit 1
fi

# Optional pull smoke via put_stub (nice-to-have)
echo
echo "==> start put_stub on 127.0.0.1:${PORT} (root=$MIRROR)"
"$PYTHON" "$ROOT/scripts/put_stub.py" --root "$MIRROR" --port "$PORT" \
  >"$DEMO_DIR/stub.log" 2>&1 &
STUB_PID=$!

for _ in $(seq 1 50); do
  if ! kill -0 "$STUB_PID" 2>/dev/null; then
    wait "$STUB_PID" || true
    echo "warning: put_stub exited early — skipping pull smoke" >&2
    cat "$DEMO_DIR/stub.log" >&2 || true
    STUB_PID=""
    break
  fi
  if "$PYTHON" -c "import socket; s=socket.create_connection(('127.0.0.1',$PORT),1); s.close()" 2>/dev/null; then
    break
  fi
  sleep 0.1
done

if [[ -n "$STUB_PID" ]] \
  && "$PYTHON" -c "import socket; s=socket.create_connection(('127.0.0.1',$PORT),1); s.close()" 2>/dev/null; then
  DEST="http://127.0.0.1:${PORT}"
  echo
  echo "==> push v2 → $DEST"
  "$BIN" push --store "$STORE" --dest "$DEST" --url-template '{base}/{path}' "$V2"

  echo
  echo "==> pull into empty store2 + verify"
  mkdir -p "$STORE2"
  "$BIN" pull --store "$STORE2" --source "$DEST" "$V2"
  "$BIN" verify --store "$STORE2" "$V2"
  echo "pull smoke: OK"
else
  echo "==> skipping pull smoke (put_stub not ready)"
fi

echo
echo "seed demo OK"
echo "  store=$STORE"
echo "  v1=$V1"
echo "  v2=$V2"
echo "  compared=$SRC ↔ $OUT"
