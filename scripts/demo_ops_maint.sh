#!/usr/bin/env bash
# Phase 12 M4 ops maint smoke: gc / store scrub --format json (+ optional --jobs).
# Local only — no real internet.
# Usage: ./scripts/demo_ops_maint.sh
# Requires: cargo, python3.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_OPS_MAINT_DEMO_DIR:-/tmp/cf-ops-maint-demo}"
BIN="${CHUNKFORGE_BIN:-}"
FIXTURE="${CHUNKFORGE_OPS_MAINT_FIXTURE:-$ROOT/fixtures/hello.txt}"
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
V1="$DEMO_DIR/v1.cfdir"
V2="$DEMO_DIR/v2.cfdir"
mkdir -p "$SRC" "$STORE"

echo 'hello-ops-maint-v1' > "$SRC/a.txt"
cp "$FIXTURE" "$SRC/b.txt"

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
echo "==> A2. change a.txt and archive --seed → $V2 (leaves orphan chunks vs v2-only)"
echo 'hello-ops-maint-v2-changed' > "$SRC/a.txt"
"$BIN" archive --store "$STORE" -o "$V2" --seed "$V1" "$SRC"

echo
echo "==> B1. gc --format json (dry-run; listing=$V2 only → expect unreferenced>0)"
GC_JSON="$("$BIN" gc --store "$STORE" --format json "$V2")"
echo "$GC_JSON"
echo "$GC_JSON" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('dry_run') is True, obj
assert obj.get('applied') is False, obj
assert obj.get('listings') == 1, obj
assert isinstance(obj.get('referenced'), int) and obj['referenced'] >= 1, obj
assert isinstance(obj.get('unreferenced'), int) and obj['unreferenced'] > 0, obj
assert obj.get('deleted') == 0, obj
"
echo "gc --format json (dry-run): OK"

echo
echo "==> B2. store scrub --format json (healthy store; all .cnk hashes valid)"
SCRUB_JSON="$("$BIN" store scrub --store "$STORE" --format json)"
echo "$SCRUB_JSON"
echo "$SCRUB_JSON" | parse_json_ok "
assert obj.get('ok') is True, obj
assert isinstance(obj.get('checked'), int) and obj['checked'] >= 1, obj
assert isinstance(obj.get('ok_count'), int) and obj['ok_count'] == obj['checked'], obj
assert obj.get('corrupt') == 0, obj
assert obj.get('unreadable') == 0, obj
assert isinstance(obj.get('corrupt_ids'), list) and obj['corrupt_ids'] == [], obj
assert isinstance(obj.get('unreadable_ids'), list) and obj['unreadable_ids'] == [], obj
"
echo "store scrub --format json: OK"

echo
echo "==> B3. optional --jobs 4 smoke (gc dry-run + scrub; result set ≡ serial)"
GC_J4="$("$BIN" gc --store "$STORE" --format json --jobs 4 "$V2")"
echo "$GC_J4"
echo "$GC_J4" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('dry_run') is True, obj
assert obj.get('unreferenced') > 0, obj
assert obj.get('deleted') == 0, obj
"
SCRUB_J4="$("$BIN" store scrub --store "$STORE" --format json --jobs 4)"
echo "$SCRUB_J4"
echo "$SCRUB_J4" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('checked') >= 1, obj
assert obj.get('corrupt') == 0 and obj.get('unreadable') == 0, obj
"
echo "--jobs 4 smoke: OK"

echo
echo "ops-maint demo OK"
echo "  store=$STORE"
echo "  v1=$V1"
echo "  v2=$V2"
