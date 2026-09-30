#!/usr/bin/env bash
# Phase 26 M4 smoke: archive --empty-dirs leaf Dir stays visible to
# ls --path / filter --path via shared filter_dir_archive, plus store get
# of one chunk id to -o.
# Narrative:
#   filter_dir_archive leaf-Dir ≠ prune ≠ gc-path
#   store get ≠ scrub ≠ cat ≠ extract ≠ recompress ≠ remove
#   ≠ pack ≠ write mount
# Local only — no internet.
# Usage: ./scripts/demo_empty_dir_path_store_get.sh
# Requires: cargo, python3, cmp.
# Env (optional):
#   CHUNKFORGE_BIN, CHUNKFORGE_PYTHON
#   CHUNKFORGE_P26_DEMO_DIR (default /tmp/cf-p26-empty-dir-store-get-demo)
#
# Version gate expects chunkforge 1.15.0 (still, until Phase26-M7).
#
# Gate (Phase26-M5): require check_compat_1_15.sh present + executable
# (same pattern as Phase25-M5 demo_ls_cat_path ↔ compat_1_14).
# check_compat_1_14.sh must also exist and be executable (Phase25).
# Do not force-run the long gate here.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_P26_DEMO_DIR:-/tmp/cf-p26-empty-dir-store-get-demo}"
BIN="${CHUNKFORGE_BIN:-}"
PYTHON="${CHUNKFORGE_PYTHON:-python3}"

if ! command -v "$PYTHON" >/dev/null 2>&1; then
  echo "error: $PYTHON not on PATH" >&2
  exit 1
fi

echo "==> building chunkforge-cli"
cargo build -p chunkforge-cli --quiet
if [[ -z "$BIN" ]]; then
  BIN="$ROOT/target/debug/chunkforge"
fi

rm -rf "$DEMO_DIR"
SRC="$DEMO_DIR/src"
STORE="$DEMO_DIR/store"
mkdir -p "$SRC/empty_leaf" "$SRC/other_empty" "$STORE"
# Small payload: one FastCDC chunk ≡ whole file (store get bytes ≡ source).
printf 'phase26-store-get-payload\n' > "$SRC/payload.txt"

echo
echo "==> A. archive --empty-dirs (empty leaf Dir + one file for a chunk id)"
"$BIN" store create --store "$STORE"
"$BIN" archive --store "$STORE" --empty-dirs -o "$DEMO_DIR/full.cfdir" \
  --format json "$SRC" \
  >"$DEMO_DIR/archive.json" \
  2>"$DEMO_DIR/archive.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/archive.json'))
assert obj.get('ok') is True, obj
assert obj.get('dirs', 0) >= 2, obj
assert obj.get('files', 0) >= 1, obj
print('A: files=', obj['files'], 'dirs=', obj['dirs'], 'chunks=', obj.get('chunks'))
"
"$BIN" ls "$DEMO_DIR/full.cfdir" >"$DEMO_DIR/ls-full.txt" 2>"$DEMO_DIR/ls-full.err"
if ! grep -F $'dir\tempty_leaf' "$DEMO_DIR/ls-full.txt" >/dev/null; then
  echo "error: full listing missing empty leaf Dir" >&2
  cat "$DEMO_DIR/ls-full.txt" >&2
  exit 1
fi
if ! grep -F $'dir\tother_empty' "$DEMO_DIR/ls-full.txt" >/dev/null; then
  echo "error: full listing missing other_empty Dir" >&2
  cat "$DEMO_DIR/ls-full.txt" >&2
  exit 1
fi
echo "A: listing has empty leaf Dir: OK"

echo
echo "==> B. ls --path empty_leaf sees dir<TAB>empty_leaf"
"$BIN" ls --path empty_leaf "$DEMO_DIR/full.cfdir" \
  >"$DEMO_DIR/ls-path.txt" \
  2>"$DEMO_DIR/ls-path.err"
if ! grep -F $'dir\tempty_leaf' "$DEMO_DIR/ls-path.txt" >/dev/null; then
  echo "error: ls --path empty_leaf missing dir row" >&2
  cat "$DEMO_DIR/ls-path.txt" >&2
  exit 1
fi
if grep -E 'other_empty|payload\.txt' "$DEMO_DIR/ls-path.txt" >/dev/null; then
  echo "error: ls --path empty_leaf must not list unrelated paths" >&2
  cat "$DEMO_DIR/ls-path.txt" >&2
  exit 1
fi
# Non-empty inventory (leaf-Dir kept; not dropped).
if [[ ! -s "$DEMO_DIR/ls-path.txt" ]]; then
  echo "error: ls --path empty_leaf produced an empty listing" >&2
  exit 1
fi
echo "B: ls --path empty_leaf sees dir: OK"

echo
echo "==> C. filter --path empty_leaf keeps Dir; ls of subset is non-empty"
"$BIN" filter --path empty_leaf -o "$DEMO_DIR/empty.cfdir" \
  "$DEMO_DIR/full.cfdir" --format json \
  >"$DEMO_DIR/filter-empty.json" \
  2>"$DEMO_DIR/filter-empty.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/filter-empty.json'))
assert obj.get('ok') is True, obj
assert obj.get('dirs', 0) >= 1, obj
assert obj.get('files', 0) == 0, obj
print('C: filter dirs=', obj['dirs'], 'files=', obj['files'], 'excluded=', obj.get('excluded'))
"
"$BIN" ls "$DEMO_DIR/empty.cfdir" >"$DEMO_DIR/ls-filtered.txt" 2>"$DEMO_DIR/ls-filtered.err"
if [[ ! -s "$DEMO_DIR/ls-filtered.txt" ]]; then
  echo "error: filtered listing ls is empty (leaf Dir was dropped)" >&2
  exit 1
fi
if ! grep -F $'dir\tempty_leaf' "$DEMO_DIR/ls-filtered.txt" >/dev/null; then
  echo "error: filtered ls missing kept Dir" >&2
  cat "$DEMO_DIR/ls-filtered.txt" >&2
  exit 1
fi
if grep -E 'other_empty|payload\.txt' "$DEMO_DIR/ls-filtered.txt" >/dev/null; then
  echo "error: filtered listing kept unrelated paths" >&2
  cat "$DEMO_DIR/ls-filtered.txt" >&2
  exit 1
fi
# leaf-Dir ≠ prune: an unrelated file beside the filtered listing stays.
printf 'keep-me\n' > "$DEMO_DIR/unrelated.txt"
if [[ ! -f "$DEMO_DIR/unrelated.txt" ]]; then
  echo "error: filter must not prune unrelated files" >&2
  exit 1
fi
echo "C: filter keeps leaf Dir; ≠ prune: OK"

echo
echo "==> D. store get one chunk hex; -o bytes match source / chunk-id"
"$BIN" chunk-id --format json "$SRC/payload.txt" \
  >"$DEMO_DIR/chunk-id.json" \
  2>"$DEMO_DIR/chunk-id.err"
"$BIN" ls --chunks --format json "$DEMO_DIR/full.cfdir" \
  >"$DEMO_DIR/ls-chunks.json" \
  2>"$DEMO_DIR/ls-chunks.err"
"$PYTHON" -c "
import json
cid = json.load(open('$DEMO_DIR/chunk-id.json'))
assert cid.get('ok') is True, cid
chunks = cid.get('chunks') or []
assert len(chunks) == 1, chunks
ch = chunks[0]
assert ch.get('offset') == 0, ch
src_len = len(open('$SRC/payload.txt','rb').read())
assert ch.get('length') == src_len, (ch, src_len)
listing = json.load(open('$DEMO_DIR/ls-chunks.json'))
files = [e for e in listing['entries'] if e.get('path') == 'payload.txt']
assert len(files) == 1, listing['entries']
ids = files[0].get('chunks') or []
assert ids == [ch['id']], (ids, ch)
open('$DEMO_DIR/chunk.id','w').write(ch['id'] + '\n')
print('D: chunk id', ch['id'], 'bytes', ch['length'])
"
ID="$(tr -d '[:space:]' < "$DEMO_DIR/chunk.id")"
"$BIN" store get --store "$STORE" "$ID" -o "$DEMO_DIR/got.bin" --format json \
  >"$DEMO_DIR/store-get.json" \
  2>"$DEMO_DIR/store-get.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/store-get.json'))
assert obj.get('ok') is True, obj
assert obj.get('id') == '$ID', obj
src_len = len(open('$SRC/payload.txt','rb').read())
assert obj.get('bytes') == src_len, obj
print('D: store get json bytes=', obj['bytes'])
"
cmp "$SRC/payload.txt" "$DEMO_DIR/got.bin"
"$BIN" store get --store "$STORE" --verify "$ID" -o "$DEMO_DIR/got-verify.bin" \
  >"$DEMO_DIR/store-get-verify.out" \
  2>"$DEMO_DIR/store-get-verify.err"
cmp "$SRC/payload.txt" "$DEMO_DIR/got-verify.bin"
# store get ≠ remove: chunk still present
"$BIN" store has --store "$STORE" "$ID" >/dev/null
echo "D: store get -o ≡ source; --verify ≡ source; ≠ remove: OK"

echo
echo "==> E. help / narrative nails (≠ prune / ≠ scrub / ≠ cat / ≠ pack / ≠ write mount / ≠ gc-path)"
GET_HELP="$("$BIN" store get --help)"
for needle in scrub cat extract recompress remove; do
  if ! grep -F "$needle" <<<"$GET_HELP" >/dev/null; then
    echo "error: store get --help must nail ≠ $needle" >&2
    echo "$GET_HELP" >&2
    exit 1
  fi
done
for flag in --store --verify --format; do
  if ! grep -F -- "$flag" <<<"$GET_HELP" >/dev/null; then
    echo "error: store get --help missing $flag" >&2
    exit 1
  fi
done
if ! grep -E -- '(^|[[:space:]])-o,' <<<"$GET_HELP" >/dev/null && \
   ! grep -F -- '--output' <<<"$GET_HELP" >/dev/null; then
  echo "error: store get --help missing -o/--output" >&2
  echo "$GET_HELP" >&2
  exit 1
fi
echo "store get --help ≠ scrub ≠ cat ≠ extract ≠ recompress ≠ remove: OK"

GC_HELP="$("$BIN" gc --help)"
if grep -E -- '--path\b' <<<"$GC_HELP" >/dev/null; then
  echo "error: gc must NOT expose --path (leaf-Dir ≠ gc-path)" >&2
  echo "$GC_HELP" >&2
  exit 1
fi
echo "gc --help has no --path: OK"

EXT_HELP="$("$BIN" extract --help)"
if grep -E -- '--delete\b' <<<"$EXT_HELP" >/dev/null; then
  echo "error: extract must NOT expose --delete (leaf-Dir ≠ prune)" >&2
  exit 1
fi
echo "extract --help has no --delete (≠ prune): OK"

MOUNT_HELP="$("$BIN" mount --help)"
if ! grep -F 'read-only' <<<"$MOUNT_HELP" >/dev/null; then
  echo "error: mount --help must stay read-only (≠ write mount)" >&2
  exit 1
fi
if ! grep -F 'write-mount' <<<"$MOUNT_HELP" >/dev/null; then
  echo "error: mount --help must nail not write-mount" >&2
  exit 1
fi
echo "mount --help read-only / ≠ write mount: OK"

TOP_HELP="$("$BIN" --help)"
if grep -Eiw 'packfile' <<<"$TOP_HELP" >/dev/null; then
  echo "error: top-level help must not advertise pack" >&2
  exit 1
fi
if grep -E '^  pack\b' <<<"$TOP_HELP" >/dev/null; then
  echo "error: pack must not be a subcommand" >&2
  exit 1
fi
echo "no pack subcommand (≠ pack): OK"
echo "E: help / narrative nails: OK"

echo
echo "==> F. version 1.15.0 + compat_1_14 + compat_1_15 (Phase26-M5)"
VER="$("$BIN" --version)"
echo "version: $VER"
if ! grep -F '1.15.0' <<<"$VER" >/dev/null; then
  echo "error: expected chunkforge 1.15.0 (until M7); got $VER" >&2
  exit 1
fi
if ! grep -E '^version = "1\.15\.0"' "$ROOT/Cargo.toml" >/dev/null; then
  echo "error: workspace Cargo.toml version must stay 1.15.0" >&2
  grep -E '^version' "$ROOT/Cargo.toml" >&2 || true
  exit 1
fi
COMPAT114="$ROOT/scripts/check_compat_1_14.sh"
if [[ ! -f "$COMPAT114" ]]; then
  echo "error: check_compat_1_14.sh must exist" >&2
  exit 1
fi
if [[ ! -x "$COMPAT114" ]]; then
  echo "error: check_compat_1_14.sh must be executable" >&2
  exit 1
fi
echo "compat_1_14 present+executable: OK"

# Phase26-M5: compat_1_15 must exist + executable (same pattern as
# demo_ls_cat_path ↔ compat_1_14 after Phase25-M5). Do not force-run
# the long gate here.
COMPAT115="$ROOT/scripts/check_compat_1_15.sh"
if [[ ! -f "$COMPAT115" ]]; then
  echo "error: check_compat_1_15.sh must exist" >&2
  exit 1
fi
if [[ ! -x "$COMPAT115" ]]; then
  echo "error: check_compat_1_15.sh must be executable" >&2
  exit 1
fi
echo "compat_1_15 present+executable: OK"
echo "F: version 1.15.0 / compat_1_14 + compat_1_15 present+executable: OK"

echo
echo "demo_empty_dir_path_store_get: ALL OK / PASS"
echo "proved: archive --empty-dirs → ls --path empty_leaf sees dir;"
echo "        filter --path empty_leaf keeps Dir; filtered ls non-empty;"
echo "        store get hex -o bytes ≡ source / chunk-id; optional --verify;"
echo "        filter_dir_archive leaf-Dir ≠ prune ≠ gc-path;"
echo "        store get ≠ scrub ≠ cat ≠ extract ≠ recompress ≠ remove;"
echo "        ≠ pack ≠ write mount; version 1.15.0; compat_1_15 hard-required"
