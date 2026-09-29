#!/usr/bin/env bash
# Phase 8 HTTP-retry smoke: put_stub --fail-transient →
#   --http-retries 0 fails on 503;
#   --http-retries 3 succeeds after 503→200; summary contains retries=.
# Local only — no real internet.
# Usage: ./scripts/demo_http_retry.sh
# Requires: cargo, python3.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_RETRY_DEMO_DIR:-/tmp/cf-http-retry-demo}"
BIN="${CHUNKFORGE_BIN:-}"
FIXTURE="${CHUNKFORGE_RETRY_FIXTURE:-$ROOT/fixtures/hello.txt}"
PORT="${CHUNKFORGE_RETRY_PORT:-8768}"
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
MIRROR0="$DEMO_DIR/mirror0"
MIRROR3="$DEMO_DIR/mirror3"
SRC="$DEMO_DIR/src"
CFDIR="$DEMO_DIR/v1.cfdir"
mkdir -p "$STORE" "$MIRROR0" "$MIRROR3" "$SRC"

echo 'hello-retry-v1' > "$SRC/a.txt"
cp "$FIXTURE" "$SRC/b.txt"

STUB_PID=""
cleanup() {
  if [[ -n "${STUB_PID:-}" ]] && kill -0 "$STUB_PID" 2>/dev/null; then
    kill "$STUB_PID" 2>/dev/null || true
    wait "$STUB_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

wait_for_port() {
  local port="$1"
  for _ in $(seq 1 50); do
    if ! kill -0 "$STUB_PID" 2>/dev/null; then
      wait "$STUB_PID" || true
      echo "error: put_stub exited early; log:" >&2
      cat "$DEMO_DIR/stub.log" >&2 || true
      exit 1
    fi
    if "$PYTHON" -c "import socket; s=socket.create_connection(('127.0.0.1',$port),1); s.close()" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  echo "error: timed out waiting for put_stub on port $port" >&2
  cat "$DEMO_DIR/stub.log" >&2 || true
  exit 1
}

start_stub() {
  local root="$1"
  local fail_n="$2"
  cleanup
  STUB_PID=""
  : >"$DEMO_DIR/stub.log"
  echo "==> start put_stub on 127.0.0.1:${PORT} (root=$root fail-transient=$fail_n)"
  "$PYTHON" "$ROOT/scripts/put_stub.py" --root "$root" --port "$PORT" \
    --fail-transient "$fail_n" \
    >"$DEMO_DIR/stub.log" 2>&1 &
  STUB_PID=$!
  wait_for_port "$PORT"
}

echo
echo "==> archive $SRC → $CFDIR"
"$BIN" archive --store "$STORE" -o "$CFDIR" "$SRC"

DEST="http://127.0.0.1:${PORT}"

echo
echo "==> push --http-retries 0 (expect fail on injected 503)"
start_stub "$MIRROR0" 1
set +e
PUSH0="$("$BIN" push --store "$STORE" --dest "$DEST" \
  --http-retries 0 --http-retry-backoff-ms 0 "$CFDIR" 2>&1)"
PUSH0_RC=$?
set -e
echo "$PUSH0"
if [[ "$PUSH0_RC" -eq 0 ]]; then
  echo "error: expected non-zero exit with --http-retries 0 against 503" >&2
  exit 1
fi
if ! echo "$PUSH0" | grep -Eq 'retries='; then
  echo "error: expected retries= in push summary" >&2
  exit 1
fi
if ! echo "$PUSH0" | grep -Eq 'failed_transient=[1-9]|failed=[1-9]'; then
  echo "error: expected failed_transient≥1 (or failed≥1) on 503 with retries=0" >&2
  exit 1
fi
echo "retries=0 → fail on 503: OK (exit=$PUSH0_RC)"

echo
echo "==> push --http-retries 3 (expect success after 2×503 → 200)"
start_stub "$MIRROR3" 2
set +e
PUSH3="$("$BIN" push --store "$STORE" --dest "$DEST" \
  --http-retries 3 --http-retry-backoff-ms 0 "$CFDIR" 2>&1)"
PUSH3_RC=$?
set -e
echo "$PUSH3"
if [[ "$PUSH3_RC" -ne 0 ]]; then
  echo "error: expected exit 0 with --http-retries 3 after transient 503s" >&2
  cat "$DEMO_DIR/stub.log" >&2 || true
  exit 1
fi
if ! echo "$PUSH3" | grep -Eq 'retries='; then
  echo "error: expected retries= in successful push summary" >&2
  exit 1
fi
if ! echo "$PUSH3" | grep -Eq 'failed=0'; then
  echo "error: expected failed=0 on successful push" >&2
  exit 1
fi
if ! echo "$PUSH3" | grep -Eq 'uploaded=[1-9]'; then
  echo "error: expected uploaded≥1 on successful push" >&2
  exit 1
fi
echo "retries=3 → success after 503→200: OK"

echo
echo "==> verify --source $DEST (post-push; --http-retries 0)"
"$BIN" verify --source "$DEST" --http-retries 0 "$CFDIR"

echo
echo "==> diff --format json (self-compare; expect exit 0)"
DIFF_JSON="$("$BIN" diff --format json "$CFDIR" "$CFDIR")"
echo "$DIFF_JSON" | "$PYTHON" -c "
import json, sys
obj = json.load(sys.stdin)
assert obj.get('added') == [] and obj.get('removed') == [] and obj.get('changed') == [], obj
print('diff --format json: OK (identical)')
"

echo
echo "http-retry demo OK"
echo "  store=$STORE"
echo "  listing=$CFDIR"
echo "  dest=$DEST"
echo "  mirror0=$MIRROR0 (retries=0 fail path)"
echo "  mirror3=$MIRROR3 (retries=3 success path)"
