#!/usr/bin/env bash
# Phase 4 push smoke: local PUT stub → chunkforge push → verify --source → cmp.
# Usage: ./scripts/demo_push.sh
# Requires: cargo, python3. No real internet — only 127.0.0.1.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_PUSH_DEMO_DIR:-/tmp/cf-push-demo}"
BIN="${CHUNKFORGE_BIN:-}"
FIXTURE="${CHUNKFORGE_PUSH_FIXTURE:-$ROOT/fixtures/hello.txt}"
PORT="${CHUNKFORGE_PUSH_PORT:-8766}"
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
STORE="$DEMO_DIR/store"
MIRROR="$DEMO_DIR/mirror"
IDX="$DEMO_DIR/hello.cfidx"
OUT="$DEMO_DIR/hello.out"
mkdir -p "$STORE" "$MIRROR"

STUB_PID=""
cleanup() {
  if [[ -n "$STUB_PID" ]] && kill -0 "$STUB_PID" 2>/dev/null; then
    kill "$STUB_PID" 2>/dev/null || true
    wait "$STUB_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo
echo "==> make index from $(basename "$FIXTURE")"
"$BIN" make --store "$STORE" -o "$IDX" "$FIXTURE"

echo
echo "==> start put_stub on 127.0.0.1:${PORT} (root=$MIRROR)"
"$PYTHON" "$ROOT/scripts/put_stub.py" --root "$MIRROR" --port "$PORT" \
  >"$DEMO_DIR/stub.log" 2>&1 &
STUB_PID=$!

# Wait until the stub accepts connections
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
echo "==> push (first) → $DEST"
PUSH1="$("$BIN" push --store "$STORE" --dest "$DEST" --url-template '{base}/{path}' "$IDX" 2>&1)"
echo "$PUSH1"
if ! echo "$PUSH1" | grep -Eq 'uploaded=[1-9][0-9]*'; then
  echo "error: expected uploaded≥1 on first push" >&2
  exit 1
fi
if ! echo "$PUSH1" | grep -Eq 'failed=0'; then
  echo "error: expected failed=0 on first push" >&2
  exit 1
fi

echo
echo "==> verify --source $DEST"
"$BIN" verify --source "$DEST" "$IDX"

echo
echo "==> cat --source + cmp"
"$BIN" cat --source "$DEST" "$IDX" -o "$OUT"
cmp "$FIXTURE" "$OUT"
echo "cmp: OK"

echo
echo "==> push (second, expect uploaded=0 / skipped≥1)"
PUSH2="$("$BIN" push --store "$STORE" --dest "$DEST" "$IDX" 2>&1)"
echo "$PUSH2"
if ! echo "$PUSH2" | grep -Eq 'uploaded=0'; then
  echo "error: expected uploaded=0 on idempotent push" >&2
  exit 1
fi
if ! echo "$PUSH2" | grep -Eq 'skipped=[1-9][0-9]*'; then
  echo "error: expected skipped≥1 on idempotent push" >&2
  exit 1
fi

# Confirm no .cfidx landed under the mirror (push uploads chunks only)
if find "$MIRROR" -type f -name '*.cfidx' 2>/dev/null | grep -q .; then
  echo "error: unexpected .cfidx under mirror (push must not upload indexes)" >&2
  find "$MIRROR" -type f -name '*.cfidx' >&2
  exit 1
fi

echo
echo "push demo OK"
echo "  store=$STORE"
echo "  mirror=$MIRROR"
echo "  dest=$DEST"
echo "  index=$IDX (kept local; not uploaded)"
echo "  compared=$FIXTURE ↔ $OUT"
