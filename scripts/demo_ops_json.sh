#!/usr/bin/env bash
# Phase 11 ops smoke: extract --skip-trust-mtime + extract/push/pull --format json.
# Local only — no real internet (put_stub on 127.0.0.1).
# Usage: ./scripts/demo_ops_json.sh
# Requires: cargo, python3.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_OPS_JSON_DEMO_DIR:-/tmp/cf-ops-json-demo}"
BIN="${CHUNKFORGE_BIN:-}"
FIXTURE="${CHUNKFORGE_OPS_JSON_FIXTURE:-$ROOT/fixtures/hello.txt}"
PORT="${CHUNKFORGE_OPS_JSON_PORT:-8771}"
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
PULL_STORE="$DEMO_DIR/pull-store"
V1="$DEMO_DIR/v1.cfdir"
mkdir -p "$SRC" "$STORE" "$MIRROR" "$PULL_STORE"

echo 'hello-ops-json-v1' > "$SRC/a.txt"
cp "$FIXTURE" "$SRC/b.txt"
N_FILES=2

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

parse_json_ok() {
  # stdin → must be one JSON object; assert keys via optional python snippet arg
  local check="${1:-}"
  "$PYTHON" -c "
import json, sys
obj = json.load(sys.stdin)
assert isinstance(obj, dict), obj
$check
print('json parse: OK', obj)
"
}

echo
echo "==> A1. archive $SRC → $V1"
"$BIN" archive --store "$STORE" -o "$V1" "$SRC"

echo
echo "==> A2. first extract → $OUT (expect wrote ${N_FILES})"
FIRST="$("$BIN" extract --store "$STORE" -o "$OUT" "$V1" 2>&1)"
echo "$FIRST"
if ! echo "$FIRST" | grep -Eq "wrote ${N_FILES} files|wrote=.*${N_FILES}|\( *${N_FILES} files"; then
  echo "error: expected first extract to write ${N_FILES} files; got:" >&2
  echo "$FIRST" >&2
  exit 1
fi
cmp "$SRC/a.txt" "$OUT/a.txt"
cmp "$SRC/b.txt" "$OUT/b.txt"
echo "first extract: OK"

# Align dest mtimes with source (= listing mtime_secs) so trust-mtime can
# take the size+mtime fast path (extract restores mode, not mtime).
touch -r "$SRC/a.txt" "$OUT/a.txt"
touch -r "$SRC/b.txt" "$OUT/b.txt"

echo
echo "==> A3. extract --skip-unchanged --skip-trust-mtime --force (expect skipped=${N_FILES})"
TRUST="$("$BIN" extract --store "$STORE" -o "$OUT" \
  --skip-unchanged --skip-trust-mtime --force "$V1" 2>&1)"
echo "$TRUST"
if ! echo "$TRUST" | grep -Eq "skipped=${N_FILES}"; then
  echo "error: expected skipped=${N_FILES} with --skip-trust-mtime; stderr above" >&2
  exit 1
fi
if ! echo "$TRUST" | grep -Eq 'wrote=0'; then
  echo "error: expected wrote=0 with --skip-trust-mtime on matching tree" >&2
  exit 1
fi
echo "skip-trust-mtime (skipped=${N_FILES}): OK"

echo
echo "==> A4. contrast: --skip-unchanged --force (no trust-mtime; content path; expect skipped=${N_FILES})"
CONTENT="$("$BIN" extract --store "$STORE" -o "$OUT" \
  --skip-unchanged --force "$V1" 2>&1)"
echo "$CONTENT"
if ! echo "$CONTENT" | grep -Eq "skipped=${N_FILES}"; then
  echo "error: expected skipped=${N_FILES} on content-path skip; stderr above" >&2
  exit 1
fi
echo "content-path skip (no trust-mtime): OK"

echo
echo "==> B1. extract --format json (write path; --skip-unchanged --force)"
EXT_JSON="$("$BIN" extract --store "$STORE" -o "$OUT" \
  --skip-unchanged --force --format json "$V1")"
echo "$EXT_JSON"
echo "$EXT_JSON" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('dry_run') is False, obj
assert obj.get('skipped') == ${N_FILES}, obj
assert obj.get('wrote') == 0, obj
assert 'dirs' in obj, obj
"
echo "extract --format json (write): OK"

echo
echo "==> B2. extract --format json --dry-run --skip-unchanged"
DRY_JSON="$("$BIN" extract --store "$STORE" -o "$OUT" \
  --skip-unchanged --dry-run --format json "$V1")"
echo "$DRY_JSON"
echo "$DRY_JSON" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('dry_run') is True, obj
assert obj.get('would_skip') == ${N_FILES}, obj
assert obj.get('would_write') == 0, obj
assert 'would_dirs' in obj and 'would_fail' in obj, obj
"
echo "extract --format json (dry-run): OK"

echo
echo "==> B3. start put_stub on 127.0.0.1:${PORT} (root=$MIRROR)"
: >"$DEMO_DIR/stub.log"
"$PYTHON" "$ROOT/scripts/put_stub.py" --root "$MIRROR" --port "$PORT" \
  >"$DEMO_DIR/stub.log" 2>&1 &
STUB_PID=$!
wait_for_port "$PORT"
DEST="http://127.0.0.1:${PORT}"

echo
echo "==> B4. push --format json → $DEST"
PUSH_JSON="$("$BIN" push --store "$STORE" --dest "$DEST" --format json "$V1")"
echo "$PUSH_JSON"
echo "$PUSH_JSON" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('dry_run') is False, obj
assert obj.get('failed') == 0, obj
assert obj.get('uploaded', 0) >= 1, obj
assert 'skipped' in obj and 'failed_transient' in obj and 'failed_permanent' in obj, obj
assert 'retries' in obj and 'unique_chunks' in obj and 'listings' in obj, obj
"
echo "push --format json: OK"

echo
echo "==> B5. pull --format json (source=$DEST → store=$PULL_STORE)"
PULL_JSON="$("$BIN" pull --store "$PULL_STORE" --source "$DEST" --format json "$V1")"
echo "$PULL_JSON"
echo "$PULL_JSON" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('dry_run') is False, obj
assert obj.get('failed') == 0, obj
assert obj.get('fetched', 0) >= 1, obj
assert 'skipped' in obj and 'failed_transient' in obj and 'failed_permanent' in obj, obj
assert 'retries' in obj and 'unique_chunks' in obj and 'listings' in obj, obj
"
echo "pull --format json: OK"

echo
echo "==> B6. push --format json again (idempotent; expect uploaded=0 / skipped≥1)"
PUSH2_JSON="$("$BIN" push --store "$STORE" --dest "$DEST" --format json "$V1")"
echo "$PUSH2_JSON"
echo "$PUSH2_JSON" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('uploaded') == 0, obj
assert obj.get('skipped', 0) >= 1, obj
assert obj.get('failed') == 0, obj
"
echo "push --format json (idempotent): OK"

echo
echo "ops-json demo OK"
echo "  store=$STORE"
echo "  listing=$V1"
echo "  out=$OUT"
echo "  dest=$DEST"
echo "  pull-store=$PULL_STORE"
