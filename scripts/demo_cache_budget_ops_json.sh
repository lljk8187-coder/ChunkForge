#!/usr/bin/env bash
# Phase 15 M5 smoke: make/cat --format json parse + --cache + small
# --cache-max-bytes refuse-fill (first batch fills; second miss does not grow
# disk; read still succeeds). Local only — file primary (no internet).
# Usage: ./scripts/demo_cache_budget_ops_json.sh
# Requires: cargo, python3.
# Env (optional):
#   CHUNKFORGE_BIN, CHUNKFORGE_PYTHON
#   CHUNKFORGE_CACHE_BUDGET_DEMO_DIR (default /tmp/cf-cache-budget-demo)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_CACHE_BUDGET_DEMO_DIR:-/tmp/cf-cache-budget-demo}"
BIN="${CHUNKFORGE_BIN:-}"
PYTHON="${CHUNKFORGE_PYTHON:-python3}"

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
PRIMARY="$DEMO_DIR/primary"
CACHE="$DEMO_DIR/cache"
mkdir -p "$PRIMARY" "$CACHE"

# Fixed-size FastCDC → predictable chunk counts (2 × 4KiB each blob)
CHUNK_SIZE="4096:4096:4096"
# After first blob fills cache (~8192 on-disk), one more 4KiB chunk would
# exceed this soft budget → refuse-fill on second blob misses.
CACHE_MAX=10000

dd if=/dev/urandom of="$DEMO_DIR/a.bin" bs=8192 count=1 status=none
dd if=/dev/urandom of="$DEMO_DIR/b.bin" bs=8192 count=1 status=none

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

echo
echo "==> A1. make --format json (blob A) → primary"
MAKE_A="$("$BIN" make --store "$PRIMARY" -o "$DEMO_DIR/a.cfidx" \
  --chunk-size "$CHUNK_SIZE" --format json "$DEMO_DIR/a.bin")"
echo "$MAKE_A"
echo "$MAKE_A" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('bytes') == 8192, obj
assert obj.get('chunks') == 2, obj
assert isinstance(obj.get('new'), int) and obj['new'] >= 1, obj
assert isinstance(obj.get('reused'), int), obj
assert set(obj.keys()) >= {'ok', 'bytes', 'chunks', 'new', 'reused'}, obj
"
echo "make json (A): OK"

echo
echo "==> A2. make --format json (blob B; distinct chunks) → primary"
MAKE_B="$("$BIN" make --store "$PRIMARY" -o "$DEMO_DIR/b.cfidx" \
  --chunk-size "$CHUNK_SIZE" --format json "$DEMO_DIR/b.bin")"
echo "$MAKE_B"
echo "$MAKE_B" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('bytes') == 8192, obj
assert obj.get('chunks') == 2, obj
assert obj.get('new') == 2, obj
assert obj.get('reused') == 0, obj
"
echo "make json (B): OK"

echo
echo "==> B1. cat --format json (blob A) with --cache + --cache-max-bytes=$CACHE_MAX (first miss fills)"
CAT_A="$("$BIN" cat --source "$PRIMARY" --cache "$CACHE" \
  --cache-max-bytes "$CACHE_MAX" -o "$DEMO_DIR/out-a.bin" \
  --format json "$DEMO_DIR/a.cfidx")"
echo "$CAT_A"
echo "$CAT_A" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('bytes') == 8192, obj
assert set(obj.keys()) >= {'ok', 'bytes'}, obj
"
cmp "$DEMO_DIR/a.bin" "$DEMO_DIR/out-a.bin"
STATS1="$("$BIN" store stats --store "$CACHE" --format json)"
echo "$STATS1"
echo "$STATS1" | parse_json_ok "
assert obj.get('ok') is True, obj
assert isinstance(obj.get('chunks'), int) and obj['chunks'] >= 1, obj
assert isinstance(obj.get('bytes_on_disk'), int) and obj['bytes_on_disk'] > 0, obj
assert obj['bytes_on_disk'] <= $CACHE_MAX, obj
"
CHUNKS1="$("$PYTHON" -c "import json,sys; print(json.load(sys.stdin)['chunks'])" <<<"$STATS1")"
BYTES1="$("$PYTHON" -c "import json,sys; print(json.load(sys.stdin)['bytes_on_disk'])" <<<"$STATS1")"
echo "first fill: chunks=$CHUNKS1 bytes_on_disk=$BYTES1 (≤ $CACHE_MAX): OK"

echo
echo "==> B2. cat --format json (blob B) same cache+max — miss refuse-fill; disk must not grow; read OK"
CAT_B="$("$BIN" cat --source "$PRIMARY" --cache "$CACHE" \
  --cache-max-bytes "$CACHE_MAX" -o "$DEMO_DIR/out-b.bin" \
  --format json "$DEMO_DIR/b.cfidx")"
echo "$CAT_B"
echo "$CAT_B" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('bytes') == 8192, obj
"
cmp "$DEMO_DIR/b.bin" "$DEMO_DIR/out-b.bin"
STATS2="$("$BIN" store stats --store "$CACHE" --format json)"
echo "$STATS2"
echo "$STATS2" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('chunks') == $CHUNKS1, (
    f\"expected chunks unchanged ($CHUNKS1), got {obj.get('chunks')}\")
assert obj.get('bytes_on_disk') == $BYTES1, (
    f\"expected bytes_on_disk unchanged ($BYTES1), got {obj.get('bytes_on_disk')}\")
assert obj['bytes_on_disk'] <= $CACHE_MAX, obj
"
echo "second miss: no disk growth (chunks=$CHUNKS1 bytes_on_disk=$BYTES1); read OK"

echo
echo "==> C1. --cache-max-bytes without --cache → non-zero (clear error)"
if "$BIN" cat --source "$PRIMARY" --cache-max-bytes "$CACHE_MAX" \
  -o "$DEMO_DIR/should-fail.bin" "$DEMO_DIR/a.cfidx" 2>"$DEMO_DIR/no-cache.err"; then
  echo "error: expected non-zero exit when --cache-max-bytes without --cache" >&2
  exit 1
fi
if ! grep -qiE 'cache-max-bytes|cache' "$DEMO_DIR/no-cache.err"; then
  echo "error: expected clear error mentioning cache; got:" >&2
  cat "$DEMO_DIR/no-cache.err" >&2
  exit 1
fi
echo "no --cache + --cache-max-bytes → error: OK"

echo
echo "cache-budget + ops-json demo OK"
echo "  primary=$PRIMARY"
echo "  cache=$CACHE (max=$CACHE_MAX; refuse-fill ≠ LRU ≠ trim ≠ GC ≠ sync)"
echo "  make/cat --format json fields: make={ok,bytes,chunks,new,reused} cat={ok,bytes}"
echo "  after A: chunks=$CHUNKS1 bytes_on_disk=$BYTES1"
echo "  after B: unchanged (refuse-fill); both reads matched inputs"
