#!/usr/bin/env bash
# Phase 19 M4 smoke: store create --compression zstd → pull into that store;
# pull omit ≡ create none (≡ 1.8); pull --compression zstd create-on-pull;
# diff --progress stderr; default quiet; repeat create ≠ recompress.
# Local only — no internet.
# Usage: ./scripts/demo_store_create_pull_compression.sh
# Requires: cargo, python3.
# Env (optional):
#   CHUNKFORGE_BIN, CHUNKFORGE_PYTHON
#   CHUNKFORGE_P19_DEMO_DIR (default /tmp/cf-p19-store-create-demo)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_P19_DEMO_DIR:-/tmp/cf-p19-store-create-demo}"
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
TREE="$DEMO_DIR/tree"
mkdir -p "$STORE_SRC" "$TREE"

# Fixed-size FastCDC → predictable multi-chunk set
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

assert_no_progress() {
  local label="$1"
  local errfile="$2"
  if grep -E '^progress:' "$errfile" >/dev/null 2>&1; then
    echo "error: $label stderr must stay quiet (no progress:); got:" >&2
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
# Second tree listing for diff (mutate one file)
cp -a "$TREE" "$DEMO_DIR/tree2"
printf 'readme-changed\n' > "$DEMO_DIR/tree2/readme.txt"
"$BIN" archive --store "$STORE_SRC" -o "$DEMO_DIR/tree2.cfdir" \
  --chunk-size "$CHUNK_SIZE" --format json "$DEMO_DIR/tree2" \
  | tee "$DEMO_DIR/archive2.json" \
  | parse_json_ok "
assert obj.get('ok') is True, obj
"
echo "source listings ready: OK"

echo
echo "==> A. store create --compression zstd → pull (omit compression) → stats zstd"
STORE_A="$DEMO_DIR/store-a-zstd"
"$BIN" store create --store "$STORE_A" --compression zstd --format json \
  | tee "$DEMO_DIR/create-a.json" \
  | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('compression') == 'zstd', obj
assert isinstance(obj.get('store'), str) and obj['store'], obj
"
"$BIN" pull --store "$STORE_A" --source "$STORE_SRC" --format json \
  "$DEMO_DIR/blob.cfidx" \
  >"$DEMO_DIR/pull-a.json" \
  2>"$DEMO_DIR/pull-a.err"
cat "$DEMO_DIR/pull-a.err" || true
parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('failed') == 0, obj
assert isinstance(obj.get('unique_chunks'), int) and obj['unique_chunks'] >= 1, obj
" < "$DEMO_DIR/pull-a.json"
STATS_A="$("$BIN" store stats --store "$STORE_A" --format json)"
echo "$STATS_A"
echo "$STATS_A" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('compression') == 'zstd', obj
assert isinstance(obj.get('chunks'), int) and obj['chunks'] >= 1, obj
"
echo "A: create zstd + pull omit + stats zstd: OK"

echo
echo "==> B. pull omit flag → brand-new path → compression none (≡ 1.8)"
STORE_B="$DEMO_DIR/store-b-none"
"$BIN" pull --store "$STORE_B" --source "$STORE_SRC" --format json \
  "$DEMO_DIR/blob.cfidx" \
  >"$DEMO_DIR/pull-b.json" \
  2>"$DEMO_DIR/pull-b.err"
parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('failed') == 0, obj
" < "$DEMO_DIR/pull-b.json"
STATS_B="$("$BIN" store stats --store "$STORE_B" --format json)"
echo "$STATS_B"
echo "$STATS_B" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('compression') in (None, 'none'), obj
"
echo "B: omit pull create ≡ none: OK"

echo
echo "==> C. pull --compression zstd → another empty path → zstd + fetch"
STORE_C="$DEMO_DIR/store-c-zstd"
"$BIN" pull --store "$STORE_C" --source "$STORE_SRC" \
  --compression zstd --format json \
  "$DEMO_DIR/blob.cfidx" \
  >"$DEMO_DIR/pull-c.json" \
  2>"$DEMO_DIR/pull-c.err"
parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('failed') == 0, obj
assert isinstance(obj.get('fetched'), int) and obj['fetched'] >= 1, obj
" < "$DEMO_DIR/pull-c.json"
STATS_C="$("$BIN" store stats --store "$STORE_C" --format json)"
echo "$STATS_C"
echo "$STATS_C" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('compression') == 'zstd', obj
"
echo "C: pull --compression zstd create-on-pull: OK"

echo
echo "==> D. diff --progress → stderr progress: op=diff"
"$BIN" diff --progress --format json \
  "$DEMO_DIR/tree.cfdir" "$DEMO_DIR/tree2.cfdir" \
  >"$DEMO_DIR/diff-prog.json" \
  2>"$DEMO_DIR/diff-prog.err" || true
cat "$DEMO_DIR/diff-prog.err"
parse_json_ok "
assert isinstance(obj.get('changed'), list), obj
" < "$DEMO_DIR/diff-prog.json"
if ! grep -E '^progress: op=diff done=' "$DEMO_DIR/diff-prog.err" >/dev/null; then
  echo "error: expected progress: op=diff done=N/TOTAL on stderr" >&2
  exit 1
fi
if grep -F 'progress:' "$DEMO_DIR/diff-prog.json" >/dev/null; then
  echo "error: progress must not pollute json stdout" >&2
  exit 1
fi
echo "D: diff --progress (stderr only; json orthogonal): OK"

echo
echo "==> E. default quiet paths (no progress noise)"
"$BIN" diff --format json \
  "$DEMO_DIR/tree.cfdir" "$DEMO_DIR/tree2.cfdir" \
  >"$DEMO_DIR/diff-quiet.json" \
  2>"$DEMO_DIR/diff-quiet.err" || true
assert_no_progress "default diff" "$DEMO_DIR/diff-quiet.err"

"$BIN" pull --store "$DEMO_DIR/store-quiet" --source "$STORE_SRC" \
  --format json "$DEMO_DIR/blob.cfidx" \
  >"$DEMO_DIR/pull-quiet.json" \
  2>"$DEMO_DIR/pull-quiet.err"
assert_no_progress "default pull" "$DEMO_DIR/pull-quiet.err"
echo "E: default quiet paths: OK"

echo
echo "==> F. repeat store create same path → non-zero (create ≠ recompress)"
set +e
"$BIN" store create --store "$STORE_A" --compression zstd \
  >"$DEMO_DIR/create-repeat.out" \
  2>"$DEMO_DIR/create-repeat.err"
RC=$?
set -e
if [[ "$RC" -eq 0 ]]; then
  echo "error: repeat store create must be non-zero" >&2
  cat "$DEMO_DIR/create-repeat.out" "$DEMO_DIR/create-repeat.err" >&2
  exit 1
fi
echo "repeat create exit=$RC (non-zero): OK"
echo "F: create ≠ recompress: OK"

echo
echo "==> help / version surfaces"
if ! "$BIN" store --help | grep -E '\bcreate\b' >/dev/null; then
  echo "error: store --help missing create" >&2
  exit 1
fi
if ! "$BIN" pull --help | grep -F -- '--compression' >/dev/null; then
  echo "error: pull --help missing --compression" >&2
  exit 1
fi
if ! "$BIN" diff --help | grep -F -- '--progress' >/dev/null; then
  echo "error: diff --help missing --progress" >&2
  exit 1
fi
VER="$("$BIN" --version)"
echo "version: $VER"
if ! echo "$VER" | grep -F '1.14.0' >/dev/null; then
  echo "error: expected chunkforge 1.14.0; got $VER" >&2
  exit 1
fi
COMPAT="$ROOT/scripts/check_compat_1_8.sh"
if [[ ! -f "$COMPAT" ]]; then
  echo "error: check_compat_1_8.sh must exist (1.9.0 / M5+)" >&2
  exit 1
fi
if [[ ! -x "$COMPAT" ]]; then
  echo "error: check_compat_1_8.sh must be executable" >&2
  exit 1
fi

echo
echo "demo_store_create_pull_compression: ALL OK"
