#!/usr/bin/env bash
# Phase 16 M5 smoke: dual-store --fallback (primary miss → fallback hit),
# --cache-max-bytes 1M suffix parse, store stats bytes_plaintext (none),
# optional --decode no-op on none store. Local only — no internet.
# Usage: ./scripts/demo_fallback_bytes_suffix.sh
# Requires: cargo, python3.
# Env (optional):
#   CHUNKFORGE_BIN, CHUNKFORGE_PYTHON
#   CHUNKFORGE_FALLBACK_DEMO_DIR (default /tmp/cf-fallback-bytes-demo)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_FALLBACK_DEMO_DIR:-/tmp/cf-fallback-bytes-demo}"
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
FALLBACK="$DEMO_DIR/fallback"
CACHE="$DEMO_DIR/cache"
mkdir -p "$PRIMARY" "$FALLBACK" "$CACHE"

# Fixed-size FastCDC → predictable small chunk set
CHUNK_SIZE="4096:4096:4096"
dd if=/dev/urandom of="$DEMO_DIR/payload.bin" bs=16384 count=1 status=none

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
echo "==> A0. make → fallback store (full chunks); primary = same meta, chunks removed"
"$BIN" make --store "$FALLBACK" -o "$DEMO_DIR/blob.cfidx" \
  --chunk-size "$CHUNK_SIZE" --format json "$DEMO_DIR/payload.bin" \
  | tee "$DEMO_DIR/make.json" \
  | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('bytes') == 16384, obj
assert isinstance(obj.get('chunks'), int) and obj['chunks'] >= 1, obj
"
# Valid empty-of-chunks primary: copy store layout then drop .cnk objects
cp -a "$FALLBACK/." "$PRIMARY/"
find "$PRIMARY" -type f -name '*.cnk' -delete
PRIMARY_CNK="$(find "$PRIMARY" -type f -name '*.cnk' 2>/dev/null | wc -l | tr -d ' ')"
FALLBACK_CNK="$(find "$FALLBACK" -type f -name '*.cnk' 2>/dev/null | wc -l | tr -d ' ')"
if [[ "$PRIMARY_CNK" -ne 0 ]]; then
  echo "error: primary still has $PRIMARY_CNK .cnk files" >&2
  exit 1
fi
if [[ "$FALLBACK_CNK" -lt 1 ]]; then
  echo "error: fallback has no .cnk files" >&2
  exit 1
fi
echo "primary chunks=$PRIMARY_CNK fallback chunks=$FALLBACK_CNK: OK"

echo
echo "==> A1. cat --source primary (no fallback) → must fail (Missing)"
if "$BIN" cat --source "$PRIMARY" -o "$DEMO_DIR/should-fail.bin" \
  "$DEMO_DIR/blob.cfidx" 2>"$DEMO_DIR/primary-only.err"; then
  echo "error: expected non-zero exit when primary has no chunks" >&2
  exit 1
fi
echo "primary-only cat fails as expected: OK"

echo
echo "==> A2. cat --source primary --fallback fallback → success"
"$BIN" cat --source "$PRIMARY" --fallback "$FALLBACK" \
  -o "$DEMO_DIR/out-cat.bin" --format json "$DEMO_DIR/blob.cfidx" \
  | tee "$DEMO_DIR/cat.json" \
  | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('bytes') == 16384, obj
"
cmp "$DEMO_DIR/payload.bin" "$DEMO_DIR/out-cat.bin"
echo "cat + --fallback: OK (payload matched)"

echo
echo "==> A3. verify --source primary --fallback fallback → success"
"$BIN" verify --source "$PRIMARY" --fallback "$FALLBACK" \
  --format json "$DEMO_DIR/blob.cfidx" \
  | tee "$DEMO_DIR/verify.json" \
  | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('kind') == 'cfidx', obj
assert obj.get('bytes') == 16384, obj
"
echo "verify + --fallback: OK"

echo
echo "==> B1. --cache-max-bytes 1M parse smoke (with --cache; ≡ 1048576)"
"$BIN" cat --source "$FALLBACK" --cache "$CACHE" \
  --cache-max-bytes 1M -o "$DEMO_DIR/out-cache.bin" \
  --format json "$DEMO_DIR/blob.cfidx" \
  | tee "$DEMO_DIR/cat-cache.json" \
  | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('bytes') == 16384, obj
"
cmp "$DEMO_DIR/payload.bin" "$DEMO_DIR/out-cache.bin"
CACHE_STATS="$("$BIN" store stats --store "$CACHE" --format json)"
echo "$CACHE_STATS"
echo "$CACHE_STATS" | parse_json_ok "
assert obj.get('ok') is True, obj
assert isinstance(obj.get('chunks'), int) and obj['chunks'] >= 1, obj
assert isinstance(obj.get('bytes_on_disk'), int) and obj['bytes_on_disk'] > 0, obj
assert obj['bytes_on_disk'] <= 1048576, obj
"
echo "--cache-max-bytes 1M (+ --cache): OK"

echo
echo "==> B2. illegal suffix smoke (1.5M → non-zero clear error)"
if "$BIN" cat --source "$FALLBACK" --cache "$CACHE" \
  --cache-max-bytes 1.5M -o "$DEMO_DIR/should-fail2.bin" \
  "$DEMO_DIR/blob.cfidx" 2>"$DEMO_DIR/bad-suffix.err"; then
  echo "error: expected non-zero for decimal suffix 1.5M" >&2
  exit 1
fi
if ! grep -qiE 'cache-max-bytes|byte|suffix|invalid|parse' "$DEMO_DIR/bad-suffix.err"; then
  echo "error: expected clear parse error; got:" >&2
  cat "$DEMO_DIR/bad-suffix.err" >&2
  exit 1
fi
echo "illegal 1.5M suffix rejected: OK"

echo
echo "==> C1. store stats --format json (none store) → bytes_plaintext ≡ bytes_on_disk"
STATS_JSON="$("$BIN" store stats --store "$FALLBACK" --format json)"
echo "$STATS_JSON"
echo "$STATS_JSON" | parse_json_ok "
assert obj.get('ok') is True, obj
assert isinstance(obj.get('chunks'), int) and obj['chunks'] >= 1, obj
assert isinstance(obj.get('bytes_on_disk'), int) and obj['bytes_on_disk'] > 0, obj
assert 'bytes_plaintext' in obj, obj
assert obj.get('bytes_plaintext') == obj.get('bytes_on_disk'), (
    f\"expected bytes_plaintext == bytes_on_disk, got plaintext={obj.get('bytes_plaintext')} on_disk={obj.get('bytes_on_disk')}\")
assert obj.get('compression') in (None, 'none'), obj
"
BYTES_ON="$("$PYTHON" -c "import json,sys; print(json.load(sys.stdin)['bytes_on_disk'])" <<<"$STATS_JSON")"
BYTES_PT="$("$PYTHON" -c "import json,sys; print(json.load(sys.stdin)['bytes_plaintext'])" <<<"$STATS_JSON")"
echo "bytes_plaintext=$BYTES_PT ≡ bytes_on_disk=$BYTES_ON: OK"

echo
echo "==> D1. store stats --decode (none store → no-op; plaintext still present)"
STATS_DEC="$("$BIN" store stats --store "$FALLBACK" --decode --format json)"
echo "$STATS_DEC"
echo "$STATS_DEC" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('bytes_plaintext') == obj.get('bytes_on_disk'), obj
assert obj.get('bytes_plaintext') == $BYTES_PT, obj
"
echo "--decode on none store (no-op): OK"

echo
echo "fallback + bytes-suffix demo OK"
echo "  primary=$PRIMARY (chunks stripped; Missing)"
echo "  fallback=$FALLBACK (has chunks)"
echo "  cache=$CACHE (--cache-max-bytes 1M)"
echo "  cat/verify via --fallback: OK"
echo "  store stats bytes_plaintext=$BYTES_PT ≡ on_disk (none); --decode no-op: OK"
echo "  fallback ≠ cache ≠ sync; refuse-fill ≠ LRU"
