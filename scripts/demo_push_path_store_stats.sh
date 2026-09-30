#!/usr/bin/env bash
# Phase 14 M5 smoke: full-tree .cfdir → store stats json → push --path subset
# (PUT count < full) → optional --exclude-from on push.
# Local only — no real internet (put_stub on 127.0.0.1).
# Usage: ./scripts/demo_push_path_store_stats.sh
# Requires: cargo, python3.
# Env (optional):
#   CHUNKFORGE_BIN, CHUNKFORGE_PYTHON
#   CHUNKFORGE_PUSH_PATH_STATS_DEMO_DIR (default /tmp/cf-push-path-stats-demo)
#   CHUNKFORGE_PUSH_PATH_STATS_PORT (default 8774)
#   CHUNKFORGE_PUSH_PATH_STATS_FIXTURE (default fixtures/hello.txt)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_PUSH_PATH_STATS_DEMO_DIR:-/tmp/cf-push-path-stats-demo}"
BIN="${CHUNKFORGE_BIN:-}"
FIXTURE="${CHUNKFORGE_PUSH_PATH_STATS_FIXTURE:-$ROOT/fixtures/hello.txt}"
PORT="${CHUNKFORGE_PUSH_PATH_STATS_PORT:-8774}"
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
MIRROR_FULL="$DEMO_DIR/mirror-full"
MIRROR_PATH="$DEMO_DIR/mirror-path"
MIRROR_EXCL="$DEMO_DIR/mirror-excl"
APP="$DEMO_DIR/full.cfdir"
EXCLUDES="$DEMO_DIR/excludes.txt"
mkdir -p \
  "$SRC/packages/foo" \
  "$SRC/packages/bar" \
  "$SRC/packages/baz" \
  "$STORE" \
  "$MIRROR_FULL" \
  "$MIRROR_PATH" \
  "$MIRROR_EXCL"

# Distinct payloads so FastCDC yields different chunk ids per package
echo 'foo-payload-phase14-m5-aaaa' > "$SRC/packages/foo/a.txt"
cp "$FIXTURE" "$SRC/packages/foo/b.txt"
echo 'bar-payload-phase14-m5-bbbb' > "$SRC/packages/bar/b.txt"
echo 'baz-payload-phase14-m5-cccc' > "$SRC/packages/baz/c.txt"

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
  if [[ -n "${STUB_PID:-}" ]] && kill -0 "$STUB_PID" 2>/dev/null; then
    kill "$STUB_PID" 2>/dev/null || true
    wait "$STUB_PID" 2>/dev/null || true
    STUB_PID=""
  fi
  : >"$DEMO_DIR/stub.log"
  "$PYTHON" "$ROOT/scripts/put_stub.py" --root "$root" --port "$PORT" \
    >"$DEMO_DIR/stub.log" 2>&1 &
  STUB_PID=$!
  wait_for_port "$PORT"
}

parse_json_ok() {
  local check="${1:-}"
  "$PYTHON" -c "
import json, sys
obj = json.load(sys.stdin)
assert isinstance(obj, dict), obj
$check
print('json parse: OK', obj)
"
}

count_cnk() {
  local root="$1"
  find "$root" -type f -name '*.cnk' 2>/dev/null | wc -l | tr -d ' '
}

echo
echo "==> A1. archive full tree → $APP"
"$BIN" archive --store "$STORE" -o "$APP" --format json "$SRC" | tee "$DEMO_DIR/archive.json" \
  | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('dry_run') is False, obj
assert isinstance(obj.get('files'), int) and obj['files'] >= 3, obj
assert isinstance(obj.get('chunks'), int) and obj['chunks'] >= 1, obj
"
echo "archive full-tree .cfdir: OK"

echo
echo "==> A2. store stats --format json (store has chunks)"
STATS_JSON="$("$BIN" store stats --store "$STORE" --format json)"
echo "$STATS_JSON"
echo "$STATS_JSON" | parse_json_ok "
assert obj.get('ok') is True, obj
assert isinstance(obj.get('chunks'), int) and obj['chunks'] >= 1, obj
assert isinstance(obj.get('bytes_on_disk'), int) and obj['bytes_on_disk'] > 0, obj
assert obj.get('compression') in ('none', 'zstd'), obj
"
# Alias du should work too (smoke)
DU_TEXT="$("$BIN" store du --store "$STORE")"
echo "$DU_TEXT"
if ! echo "$DU_TEXT" | grep -Eq '^store stats: chunks=[1-9]'; then
  echo "error: store du text summary unexpected: $DU_TEXT" >&2
  exit 1
fi
echo "store stats / du: OK"

DEST="http://127.0.0.1:${PORT}"

echo
echo "==> B1. put_stub → push full reference set (baseline PUT count)"
start_stub "$MIRROR_FULL"
PUSH_FULL="$("$BIN" push --store "$STORE" --dest "$DEST" --format json "$APP")"
echo "$PUSH_FULL"
echo "$PUSH_FULL" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('failed') == 0, obj
assert obj.get('uploaded', 0) >= 1, obj
assert isinstance(obj.get('unique_chunks'), int) and obj['unique_chunks'] >= 1, obj
"
FULL_UNIQUE="$("$PYTHON" -c "import json,sys; print(json.load(sys.stdin)['unique_chunks'])" <<<"$PUSH_FULL")"
FULL_PUTS="$(count_cnk "$MIRROR_FULL")"
if [[ "$FULL_PUTS" -lt 1 ]]; then
  echo "error: expected ≥1 .cnk under full mirror, got $FULL_PUTS" >&2
  exit 1
fi
# Listing must not land under mirror (chunks only)
if find "$MIRROR_FULL" -type f \( -name '*.cfdir' -o -name '*.cfidx' \) 2>/dev/null | grep -q .; then
  echo "error: unexpected listing under mirror-full (push must not upload listings)" >&2
  find "$MIRROR_FULL" -type f \( -name '*.cfdir' -o -name '*.cfidx' \) >&2
  exit 1
fi
echo "push full: unique_chunks=$FULL_UNIQUE puts=$FULL_PUTS OK"

echo
echo "==> B2. put_stub → push --path packages/foo (subset; PUT < full)"
start_stub "$MIRROR_PATH"
PUSH_PATH="$("$BIN" push --store "$STORE" --dest "$DEST" \
  --path packages/foo --format json "$APP")"
echo "$PUSH_PATH"
echo "$PUSH_PATH" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('failed') == 0, obj
assert obj.get('uploaded', 0) >= 1, obj
assert isinstance(obj.get('unique_chunks'), int) and obj['unique_chunks'] >= 1, obj
assert obj['unique_chunks'] < $FULL_UNIQUE, (
    f\"expected unique_chunks < full ($FULL_UNIQUE), got {obj['unique_chunks']}\")
"
PATH_UNIQUE="$("$PYTHON" -c "import json,sys; print(json.load(sys.stdin)['unique_chunks'])" <<<"$PUSH_PATH")"
PATH_PUTS="$(count_cnk "$MIRROR_PATH")"
if [[ "$PATH_PUTS" -ge "$FULL_PUTS" ]]; then
  echo "error: path push PUT count ($PATH_PUTS) not < full ($FULL_PUTS)" >&2
  exit 1
fi
if [[ "$PATH_PUTS" -ne "$PATH_UNIQUE" ]]; then
  echo "error: path mirror .cnk count ($PATH_PUTS) != unique_chunks ($PATH_UNIQUE)" >&2
  exit 1
fi
if find "$MIRROR_PATH" -type f \( -name '*.cfdir' -o -name '*.cfidx' \) 2>/dev/null | grep -q .; then
  echo "error: unexpected listing under mirror-path" >&2
  exit 1
fi
echo "push --path: unique_chunks=$PATH_UNIQUE puts=$PATH_PUTS (< $FULL_PUTS): OK"

echo
echo "==> C1. --exclude-from on push (exclude packages/bar/ + packages/baz/)"
printf '%s\n' '# exclude bar + baz subtrees' 'packages/bar/' 'packages/baz/' > "$EXCLUDES"
start_stub "$MIRROR_EXCL"
PUSH_EXCL="$("$BIN" push --store "$STORE" --dest "$DEST" \
  --exclude-from "$EXCLUDES" --format json "$APP")"
echo "$PUSH_EXCL"
echo "$PUSH_EXCL" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('failed') == 0, obj
assert isinstance(obj.get('unique_chunks'), int) and obj['unique_chunks'] >= 1, obj
assert obj['unique_chunks'] < $FULL_UNIQUE, (
    f\"expected unique_chunks < full ($FULL_UNIQUE), got {obj['unique_chunks']}\")
"
EXCL_UNIQUE="$("$PYTHON" -c "import json,sys; print(json.load(sys.stdin)['unique_chunks'])" <<<"$PUSH_EXCL")"
EXCL_PUTS="$(count_cnk "$MIRROR_EXCL")"
if [[ "$EXCL_PUTS" -ge "$FULL_PUTS" ]]; then
  echo "error: exclude-from PUT count ($EXCL_PUTS) not < full ($FULL_PUTS)" >&2
  exit 1
fi
echo "push --exclude-from: unique_chunks=$EXCL_UNIQUE puts=$EXCL_PUTS (< $FULL_PUTS): OK"

echo
echo "push-path + store-stats demo OK"
echo "  store=$STORE"
echo "  listing=$APP (full tree; not uploaded)"
echo "  full unique_chunks=$FULL_UNIQUE puts=$FULL_PUTS"
echo "  --path packages/foo unique_chunks=$PATH_UNIQUE puts=$PATH_PUTS"
echo "  --exclude-from unique_chunks=$EXCL_UNIQUE puts=$EXCL_PUTS"
echo "  dest=$DEST"
