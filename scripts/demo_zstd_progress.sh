#!/usr/bin/env bash
# Phase 17 M5 smoke: make --compression zstd → store stats compression=zstd
# + --decode plaintext; default make still none; archive/extract/make
# --progress stderr smoke. Local only — no internet.
# Usage: ./scripts/demo_zstd_progress.sh
# Requires: cargo, python3.
# Env (optional):
#   CHUNKFORGE_BIN, CHUNKFORGE_PYTHON
#   CHUNKFORGE_ZSTD_PROGRESS_DEMO_DIR (default /tmp/cf-zstd-progress-demo)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_ZSTD_PROGRESS_DEMO_DIR:-/tmp/cf-zstd-progress-demo}"
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
STORE_Z="$DEMO_DIR/store-z"
STORE_NONE="$DEMO_DIR/store-none"
TREE="$DEMO_DIR/tree"
OUT_TREE="$DEMO_DIR/out-tree"
mkdir -p "$STORE_Z" "$STORE_NONE" "$TREE"

# Fixed-size FastCDC → predictable small chunk set; compressible payload for zstd
CHUNK_SIZE="4096:4096:4096"
dd if=/dev/zero of="$DEMO_DIR/payload.bin" bs=16384 count=1 status=none
# Distinct second file for archive multi-file progress
dd if=/dev/zero of="$TREE/a.bin" bs=8192 count=1 status=none
dd if=/dev/urandom of="$TREE/b.bin" bs=4096 count=1 status=none
printf 'hello-tree\n' > "$TREE/readme.txt"

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
echo "==> A1. make --compression zstd → store stats compression=zstd"
"$BIN" make --store "$STORE_Z" --compression zstd \
  -o "$DEMO_DIR/blob-z.cfidx" \
  --chunk-size "$CHUNK_SIZE" --format json "$DEMO_DIR/payload.bin" \
  | tee "$DEMO_DIR/make-z.json" \
  | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('bytes') == 16384, obj
assert isinstance(obj.get('chunks'), int) and obj['chunks'] >= 1, obj
"

STATS_Z="$("$BIN" store stats --store "$STORE_Z" --format json)"
echo "$STATS_Z"
echo "$STATS_Z" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('compression') == 'zstd', obj
assert isinstance(obj.get('chunks'), int) and obj['chunks'] >= 1, obj
assert isinstance(obj.get('bytes_on_disk'), int) and obj['bytes_on_disk'] > 0, obj
assert obj.get('bytes_plaintext') is None, (
    f'zstd without --decode should null bytes_plaintext, got {obj.get(\"bytes_plaintext\")}')
"
echo "store stats compression=zstd, bytes_plaintext=null: OK"

echo
echo "==> A2. store stats --decode → bytes_plaintext present; cat → plaintext match"
STATS_DEC="$("$BIN" store stats --store "$STORE_Z" --decode --format json)"
echo "$STATS_DEC"
echo "$STATS_DEC" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('compression') == 'zstd', obj
assert isinstance(obj.get('bytes_plaintext'), int) and obj['bytes_plaintext'] > 0, obj
assert obj.get('bytes_on_disk') <= obj.get('bytes_plaintext'), obj
"
"$BIN" cat --store "$STORE_Z" -o "$DEMO_DIR/out-z.bin" \
  --format json "$DEMO_DIR/blob-z.cfidx" \
  | tee "$DEMO_DIR/cat-z.json" \
  | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('bytes') == 16384, obj
"
cmp "$DEMO_DIR/payload.bin" "$DEMO_DIR/out-z.bin"
echo "zstd --decode + cat plaintext: OK"

echo
echo "==> B1. default make (no --compression) → compression=none"
"$BIN" make --store "$STORE_NONE" \
  -o "$DEMO_DIR/blob-none.cfidx" \
  --chunk-size "$CHUNK_SIZE" --format json "$DEMO_DIR/payload.bin" \
  | tee "$DEMO_DIR/make-none.json" \
  | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('bytes') == 16384, obj
"
STATS_NONE="$("$BIN" store stats --store "$STORE_NONE" --format json)"
echo "$STATS_NONE"
echo "$STATS_NONE" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('compression') in (None, 'none'), obj
assert obj.get('bytes_plaintext') == obj.get('bytes_on_disk'), obj
"
echo "default create compression=none: OK"

echo
echo "==> C1. archive --progress → stderr has progress: op=archive"
# Reuse none store (already exists; omit compression flag)
"$BIN" archive --store "$STORE_NONE" --progress \
  -o "$DEMO_DIR/tree.cfdir" --format json "$TREE" \
  >"$DEMO_DIR/archive.json" 2>"$DEMO_DIR/archive.progress"
cat "$DEMO_DIR/archive.json" | parse_json_ok "
assert obj.get('ok') is True, obj
assert isinstance(obj.get('files'), int) and obj['files'] >= 1, obj
"
if ! grep -q 'progress: op=archive' "$DEMO_DIR/archive.progress"; then
  echo "error: expected 'progress: op=archive' on stderr; got:" >&2
  cat "$DEMO_DIR/archive.progress" >&2
  exit 1
fi
echo "archive --progress: OK"
grep 'progress: op=archive' "$DEMO_DIR/archive.progress" | head -3

echo
echo "==> C2. extract --progress → stderr has progress: op=extract"
"$BIN" extract --store "$STORE_NONE" --progress \
  -o "$OUT_TREE" --format json "$DEMO_DIR/tree.cfdir" \
  >"$DEMO_DIR/extract.json" 2>"$DEMO_DIR/extract.progress"
cat "$DEMO_DIR/extract.json" | parse_json_ok "
assert obj.get('ok') is True, obj
assert isinstance(obj.get('wrote'), int) and obj['wrote'] >= 1, obj
"
if ! grep -q 'progress: op=extract' "$DEMO_DIR/extract.progress"; then
  echo "error: expected 'progress: op=extract' on stderr; got:" >&2
  cat "$DEMO_DIR/extract.progress" >&2
  exit 1
fi
cmp "$TREE/a.bin" "$OUT_TREE/a.bin"
cmp "$TREE/b.bin" "$OUT_TREE/b.bin"
cmp "$TREE/readme.txt" "$OUT_TREE/readme.txt"
echo "extract --progress: OK"
grep 'progress: op=extract' "$DEMO_DIR/extract.progress" | head -3

echo
echo "==> D1. make --progress → stderr has progress: op=make"
STORE_MAKE_P="$DEMO_DIR/store-make-progress"
mkdir -p "$STORE_MAKE_P"
"$BIN" make --store "$STORE_MAKE_P" --progress \
  -o "$DEMO_DIR/blob-prog.cfidx" \
  --chunk-size "$CHUNK_SIZE" --format json "$DEMO_DIR/payload.bin" \
  >"$DEMO_DIR/make-prog.json" 2>"$DEMO_DIR/make.progress"
cat "$DEMO_DIR/make-prog.json" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('bytes') == 16384, obj
"
if ! grep -q 'progress: op=make' "$DEMO_DIR/make.progress"; then
  echo "error: expected 'progress: op=make' on stderr; got:" >&2
  cat "$DEMO_DIR/make.progress" >&2
  exit 1
fi
echo "make --progress: OK"
grep 'progress: op=make' "$DEMO_DIR/make.progress" | head -3

echo
echo "zstd + progress demo OK"
echo "  store-z=$STORE_Z (compression=zstd; decode+cat plaintext)"
echo "  store-none=$STORE_NONE (default create none)"
echo "  archive/extract/make --progress stderr: OK"
echo "  disk zstd ≠ wire compression ≠ pack; progress ≠ otel"
