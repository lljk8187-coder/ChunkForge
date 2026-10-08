#!/usr/bin/env bash
# Phase 18 M5 smoke: pull --verify (local store→store), --cache + --cache-stats,
# cat/verify --progress, and default quiet path. Local only — no internet.
# Usage: ./scripts/demo_pull_verify_cache_stats.sh
# Requires: cargo, python3.
# Env (optional):
#   CHUNKFORGE_BIN, CHUNKFORGE_PYTHON
#   CHUNKFORGE_P18_DEMO_DIR (default /tmp/cf-p18-pull-verify-demo)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_P18_DEMO_DIR:-/tmp/cf-p18-pull-verify-demo}"
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
STORE_SRC="$DEMO_DIR/store-src"
STORE_PULL="$DEMO_DIR/store-pull"
CACHE="$DEMO_DIR/cache"
TREE="$DEMO_DIR/tree"
mkdir -p "$STORE_SRC" "$STORE_PULL" "$CACHE" "$TREE"

# Fixed-size FastCDC → predictable multi-chunk set for progress TOTAL
CHUNK_SIZE="4096:4096:4096"
dd if=/dev/urandom of="$DEMO_DIR/payload.bin" bs=16384 count=1 status=none
dd if=/dev/urandom of="$TREE/a.bin" bs=8192 count=1 status=none
dd if=/dev/urandom of="$TREE/b.bin" bs=4096 count=1 status=none
printf 'readme\n' > "$TREE/readme.txt"

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

assert_no_noise() {
  local label="$1"
  local errfile="$2"
  if grep -E '^(progress:|cache:)' "$errfile" >/dev/null 2>&1; then
    echo "error: $label stderr must stay quiet (no progress:/cache:); got:" >&2
    cat "$errfile" >&2
    exit 1
  fi
  if grep -F 'pull: verifying' "$errfile" >/dev/null 2>&1; then
    echo "error: $label stderr must not contain pull verify chatter; got:" >&2
    cat "$errfile" >&2
    exit 1
  fi
}

echo
echo "==> A0. make + archive → source store (cfidx + cfdir)"
"$BIN" make --store "$STORE_SRC" -o "$DEMO_DIR/blob.cfidx" \
  --chunk-size "$CHUNK_SIZE" --format json "$DEMO_DIR/payload.bin" \
  | tee "$DEMO_DIR/make.json" \
  | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('bytes') == 16384, obj
assert isinstance(obj.get('chunks'), int) and obj['chunks'] >= 1, obj
"
"$BIN" archive --store "$STORE_SRC" -o "$DEMO_DIR/tree.cfdir" \
  --chunk-size "$CHUNK_SIZE" --format json "$TREE" \
  | tee "$DEMO_DIR/archive.json" \
  | parse_json_ok "
assert obj.get('ok') is True, obj
assert isinstance(obj.get('files'), int) and obj['files'] >= 1, obj
"
echo "source listings ready: OK"

echo
echo "==> A1. pull --verify (local source → empty store) → success + verify ok"
"$BIN" pull --store "$STORE_PULL" --source "$STORE_SRC" \
  --verify --format json \
  "$DEMO_DIR/blob.cfidx" \
  >"$DEMO_DIR/pull-verify.json" \
  2>"$DEMO_DIR/pull-verify.err"
cat "$DEMO_DIR/pull-verify.err"
parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('failed') == 0, obj
assert obj.get('dry_run') is False, obj
assert isinstance(obj.get('unique_chunks'), int) and obj['unique_chunks'] >= 1, obj
" < "$DEMO_DIR/pull-verify.json"
if ! grep -F 'pull: verifying' "$DEMO_DIR/pull-verify.err" >/dev/null; then
  echo "error: expected 'pull: verifying' on stderr" >&2
  exit 1
fi
if ! grep -E 'pull: verify ok' "$DEMO_DIR/pull-verify.err" >/dev/null; then
  echo "error: expected 'pull: verify ok' on stderr" >&2
  exit 1
fi
if ! grep -E '^verify: ok' "$DEMO_DIR/pull-verify.err" >/dev/null; then
  echo "error: expected verify: ok from post-pull verify" >&2
  exit 1
fi
echo "pull --verify success path: OK"

echo
echo "==> A2. pull --verify --dry-run → skip verify chatter"
"$BIN" pull --store "$DEMO_DIR/store-dry" --source "$STORE_SRC" \
  --verify --dry-run --format json \
  "$DEMO_DIR/blob.cfidx" \
  >"$DEMO_DIR/pull-dry.json" \
  2>"$DEMO_DIR/pull-dry.err"
cat "$DEMO_DIR/pull-dry.err"
parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('dry_run') is True, obj
" < "$DEMO_DIR/pull-dry.json"
if ! grep -F 'pull: --verify skipped (dry-run' "$DEMO_DIR/pull-dry.err" >/dev/null; then
  echo "error: expected dry-run verify skip note" >&2
  exit 1
fi
echo "pull --verify dry-run skip: OK"

echo
echo "==> B1. cat --cache --cache-stats → stderr cache: line"
"$BIN" cat --store "$STORE_SRC" --cache "$CACHE" --cache-stats \
  -o "$DEMO_DIR/out1.bin" --format json "$DEMO_DIR/blob.cfidx" \
  >"$DEMO_DIR/cat-cache1.json" \
  2>"$DEMO_DIR/cat-cache1.err"
cat "$DEMO_DIR/cat-cache1.err"
parse_json_ok "
assert obj.get('ok') is True, obj
assert 'cache_hits' in obj and 'cache_miss_fills' in obj and 'cache_miss_refused' in obj, obj
" < "$DEMO_DIR/cat-cache1.json"
if ! grep -E '^cache: hits=' "$DEMO_DIR/cat-cache1.err" >/dev/null; then
  echo "error: expected cache: hits=… on stderr" >&2
  exit 1
fi
echo "cache-stats first miss-fill: OK"

echo
echo "==> B2. second cat --cache --cache-stats → hits increase (optional)"
"$BIN" cat --store "$STORE_SRC" --cache "$CACHE" --cache-stats \
  -o "$DEMO_DIR/out2.bin" --format json "$DEMO_DIR/blob.cfidx" \
  >"$DEMO_DIR/cat-cache2.json" \
  2>"$DEMO_DIR/cat-cache2.err"
cat "$DEMO_DIR/cat-cache2.err"
HITS1="$("$PYTHON" -c "import re,sys; m=re.search(r'hits=(\d+)', open(sys.argv[1]).read()); print(m.group(1) if m else -1)" "$DEMO_DIR/cat-cache1.err")"
HITS2="$("$PYTHON" -c "import re,sys; m=re.search(r'hits=(\d+)', open(sys.argv[1]).read()); print(m.group(1) if m else -1)" "$DEMO_DIR/cat-cache2.err")"
if [[ "$HITS2" -lt "$HITS1" ]]; then
  echo "error: second-run hits ($HITS2) should be >= first ($HITS1)" >&2
  exit 1
fi
# Prefer observing a hit on the warm cache; soft-assert if payload was a single miss path.
if [[ "$HITS2" -gt 0 ]]; then
  echo "second-run cache hits=$HITS2 (>= first $HITS1): OK"
else
  echo "note: second-run hits still 0 (acceptable if jobs/order odd); cache: line present"
  if ! grep -E '^cache: hits=' "$DEMO_DIR/cat-cache2.err" >/dev/null; then
    echo "error: missing cache: line on second run" >&2
    exit 1
  fi
fi
cmp "$DEMO_DIR/payload.bin" "$DEMO_DIR/out1.bin"
cmp "$DEMO_DIR/payload.bin" "$DEMO_DIR/out2.bin"

echo
echo "==> C1. cat --progress → stderr progress: op=cat"
"$BIN" cat --store "$STORE_SRC" --progress \
  -o "$DEMO_DIR/out-prog.bin" "$DEMO_DIR/blob.cfidx" \
  2>"$DEMO_DIR/cat-progress.err"
cat "$DEMO_DIR/cat-progress.err"
if ! grep -E '^progress: op=cat done=' "$DEMO_DIR/cat-progress.err" >/dev/null; then
  echo "error: expected progress: op=cat done=N/TOTAL" >&2
  exit 1
fi
if ! grep -E 'done=[0-9]+/[0-9]+' "$DEMO_DIR/cat-progress.err" >/dev/null; then
  echo "error: expected done=N/TOTAL form" >&2
  exit 1
fi
cmp "$DEMO_DIR/payload.bin" "$DEMO_DIR/out-prog.bin"
echo "cat --progress: OK"

echo
echo "==> C2. verify --progress (cfidx + cfdir) → stderr progress: op=verify"
"$BIN" verify --store "$STORE_SRC" --progress "$DEMO_DIR/blob.cfidx" \
  2>"$DEMO_DIR/verify-progress.err"
cat "$DEMO_DIR/verify-progress.err"
if ! grep -E '^progress: op=verify done=' "$DEMO_DIR/verify-progress.err" >/dev/null; then
  echo "error: expected progress: op=verify done=N/TOTAL on cfidx" >&2
  exit 1
fi
"$BIN" verify --store "$STORE_SRC" --progress --format json "$DEMO_DIR/tree.cfdir" \
  >"$DEMO_DIR/verify-cfdir.json" \
  2>"$DEMO_DIR/verify-cfdir.err"
cat "$DEMO_DIR/verify-cfdir.err"
parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('kind') == 'cfdir', obj
" < "$DEMO_DIR/verify-cfdir.json"
if ! grep -E '^progress: op=verify done=' "$DEMO_DIR/verify-cfdir.err" >/dev/null; then
  echo "error: expected progress: op=verify on cfdir" >&2
  exit 1
fi
if grep -F 'progress:' "$DEMO_DIR/verify-cfdir.json" >/dev/null; then
  echo "error: progress must not pollute json stdout" >&2
  exit 1
fi
echo "verify --progress (cfidx + cfdir; json orthogonal): OK"

echo
echo "==> D1. default quiet paths (no progress / cache / verify noise)"
"$BIN" pull --store "$DEMO_DIR/store-quiet" --source "$STORE_SRC" \
  --format json "$DEMO_DIR/blob.cfidx" \
  >"$DEMO_DIR/pull-quiet.json" \
  2>"$DEMO_DIR/pull-quiet.err"
assert_no_noise "default pull" "$DEMO_DIR/pull-quiet.err"

"$BIN" cat --store "$STORE_SRC" -o "$DEMO_DIR/out-quiet.bin" \
  "$DEMO_DIR/blob.cfidx" \
  2>"$DEMO_DIR/cat-quiet.err"
assert_no_noise "default cat" "$DEMO_DIR/cat-quiet.err"

"$BIN" verify --store "$STORE_SRC" "$DEMO_DIR/blob.cfidx" \
  2>"$DEMO_DIR/verify-quiet.err"
assert_no_noise "default verify" "$DEMO_DIR/verify-quiet.err"
# verify success still prints verify: ok on text format — that is fine; just no progress:/cache:
if ! grep -E '^verify: ok' "$DEMO_DIR/verify-quiet.err" >/dev/null; then
  echo "error: default verify should still print verify: ok" >&2
  exit 1
fi
echo "default quiet paths: OK"

echo
echo "==> help surfaces"
if ! "$BIN" verify --help | grep -F -- '--progress' >/dev/null; then
  echo "error: verify --help missing --progress" >&2
  exit 1
fi
if ! "$BIN" cat --help | grep -F -- '--progress' >/dev/null; then
  echo "error: cat --help missing --progress" >&2
  exit 1
fi
if ! "$BIN" pull --help | grep -F -- '--verify' >/dev/null; then
  echo "error: pull --help missing --verify" >&2
  exit 1
fi
VER="$("$BIN" --version)"
echo "version: $VER"
if ! echo "$VER" | grep -F '1.16.0' >/dev/null; then
  echo "error: expected chunkforge 1.16.0; got $VER" >&2
  exit 1
fi

echo
echo "demo_pull_verify_cache_stats: ALL OK"
