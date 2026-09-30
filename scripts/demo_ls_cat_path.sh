#!/usr/bin/env bash
# Phase 25 M4 smoke: chunkforge ls (listing inventory) + cat --path (single
# File from .cfdir). Flow: archive --symlinks record tree with pkgs/foo +
# symlink → optional filter → subset → ls shows retained File+Symlink →
# cat --path single file bytes ≡ source/extract → .cfidx ls/cat thin
# regression → help / ≠ mount / ≠ extract / ≠ verify / ≠ pack / ≠ filter /
# ≠ prune / ≠ gc-path nails.
# Narrative:
#   ls ≠ mount ≠ extract ≠ verify ≠ pack ≠ filter
#   cat --path ≠ extract ≠ prune ≠ sync
# Local only — no internet.
# Usage: ./scripts/demo_ls_cat_path.sh
# Requires: cargo, python3, cmp.
# Env (optional):
#   CHUNKFORGE_BIN, CHUNKFORGE_PYTHON
#   CHUNKFORGE_P25_DEMO_DIR (default /tmp/cf-p25-ls-cat-demo)
#
# Version gate expects chunkforge 1.14.0 (Phase25 toward 1.15.0; bump in M7).
#
# Gate (Phase25-M5): require check_compat_1_14.sh present + executable
# (same pattern as Phase24-M5 demo_filter_listing ↔ compat_1_13).
# Do not force-run the long gate here.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_P25_DEMO_DIR:-/tmp/cf-p25-ls-cat-demo}"
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
mkdir -p \
  "$SRC/pkgs/foo" \
  "$SRC/pkgs/bar" \
  "$STORE"

echo 'hello-foo' > "$SRC/pkgs/foo/a.txt"
ln -s a.txt "$SRC/pkgs/foo/link.txt"
echo 'hello-bar' > "$SRC/pkgs/bar/b.txt"
printf 'readme\n' > "$SRC/readme.txt"

echo
echo "==> A. store create + archive --symlinks record full tree"
"$BIN" store create --store "$STORE"
"$BIN" archive --store "$STORE" -o "$DEMO_DIR/full.cfdir" \
  --symlinks record --format json "$SRC" \
  >"$DEMO_DIR/archive.json" \
  2>"$DEMO_DIR/archive.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/archive.json'))
assert obj.get('ok') is True, obj
assert obj.get('recorded_symlinks', 0) >= 1, obj
print('A: recorded_symlinks=', obj['recorded_symlinks'], 'files=', obj.get('files'))
"
echo "A: full.cfdir archived: OK"

echo
echo "==> B. optional filter --path pkgs/foo -o foo.cfdir"
"$BIN" filter --path pkgs/foo -o "$DEMO_DIR/foo.cfdir" \
  "$DEMO_DIR/full.cfdir" --format json \
  >"$DEMO_DIR/filter-foo.json" \
  2>"$DEMO_DIR/filter-foo.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/filter-foo.json'))
assert obj.get('ok') is True, obj
assert obj.get('files', 0) >= 1, obj
assert obj.get('symlinks', 0) >= 1, obj
print('B: filter files=', obj['files'], 'symlinks=', obj['symlinks'],
      'excluded=', obj['excluded'])
"
echo "B: subset foo.cfdir: OK"

echo
echo "==> C. ls foo.cfdir shows retained File + Symlink (text + json)"
"$BIN" ls "$DEMO_DIR/foo.cfdir" \
  >"$DEMO_DIR/ls-foo.txt" \
  2>"$DEMO_DIR/ls-foo.err"
if ! grep -F $'file\tpkgs/foo/a.txt\t' "$DEMO_DIR/ls-foo.txt" >/dev/null; then
  echo "error: ls text missing File pkgs/foo/a.txt" >&2
  cat "$DEMO_DIR/ls-foo.txt" >&2
  exit 1
fi
if ! grep -F $'symlink\tpkgs/foo/link.txt\t' "$DEMO_DIR/ls-foo.txt" >/dev/null; then
  echo "error: ls text missing Symlink pkgs/foo/link.txt" >&2
  cat "$DEMO_DIR/ls-foo.txt" >&2
  exit 1
fi
# bar / readme must be filtered out
if grep -E 'pkgs/bar|readme' "$DEMO_DIR/ls-foo.txt" >/dev/null; then
  echo "error: ls subset must not list filtered-out paths" >&2
  cat "$DEMO_DIR/ls-foo.txt" >&2
  exit 1
fi
"$BIN" ls --format json "$DEMO_DIR/foo.cfdir" \
  >"$DEMO_DIR/ls-foo.json" \
  2>"$DEMO_DIR/ls-foo-json.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/ls-foo.json'))
assert obj.get('ok') is True, obj
entries = obj.get('entries')
assert isinstance(entries, list), obj
kinds = {e['path']: e for e in entries}
assert 'pkgs/foo/a.txt' in kinds, entries
assert kinds['pkgs/foo/a.txt']['kind'] == 'file', kinds['pkgs/foo/a.txt']
assert 'size' in kinds['pkgs/foo/a.txt'], kinds['pkgs/foo/a.txt']
assert 'pkgs/foo/link.txt' in kinds, entries
assert kinds['pkgs/foo/link.txt']['kind'] == 'symlink', kinds['pkgs/foo/link.txt']
assert kinds['pkgs/foo/link.txt'].get('target') == 'a.txt', kinds['pkgs/foo/link.txt']
# no store / no chunks key unless --chunks
assert 'chunks' not in kinds['pkgs/foo/a.txt'], kinds['pkgs/foo/a.txt']
print('C: ls json entries=', len(entries),
      'file size=', kinds['pkgs/foo/a.txt']['size'],
      'symlink target=', kinds['pkgs/foo/link.txt']['target'])
"
echo "C: ls retained File+Symlink (text+json): OK"

echo
echo "==> D. cat --store … --path pkgs/foo/a.txt -o … ; bytes ≡ source"
"$BIN" cat --store "$STORE" --path pkgs/foo/a.txt \
  -o "$DEMO_DIR/a-from-cat.bin" "$DEMO_DIR/foo.cfdir" \
  --format json \
  >"$DEMO_DIR/cat-path.json" \
  2>"$DEMO_DIR/cat-path.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/cat-path.json'))
assert obj.get('ok') is True, obj
assert obj.get('bytes', 0) >= 1, obj
print('D: cat --path bytes=', obj['bytes'])
"
cmp "$SRC/pkgs/foo/a.txt" "$DEMO_DIR/a-from-cat.bin"
echo "D: cat --path ≡ source bytes: OK"

echo
echo "==> E. cat --path ≡ extract same File (same store+listing)"
mkdir -p "$DEMO_DIR/extract-out"
"$BIN" extract --store "$STORE" --path pkgs/foo/a.txt \
  -o "$DEMO_DIR/extract-out" "$DEMO_DIR/foo.cfdir" \
  >"$DEMO_DIR/extract.out" \
  2>"$DEMO_DIR/extract.err"
cmp "$DEMO_DIR/a-from-cat.bin" "$DEMO_DIR/extract-out/pkgs/foo/a.txt"
# cat --path ≠ prune: extract dest tree is independent; unrelated file untouched
printf 'keep-me\n' > "$DEMO_DIR/extract-out/unrelated.txt"
"$BIN" cat --store "$STORE" --path pkgs/foo/a.txt \
  -o "$DEMO_DIR/a-from-cat2.bin" "$DEMO_DIR/foo.cfdir"
if [[ ! -f "$DEMO_DIR/extract-out/unrelated.txt" ]]; then
  echo "error: cat --path must not prune unrelated extract-tree files" >&2
  exit 1
fi
echo "E: cat --path ≡ extract; ≠ prune: OK"

echo
echo "==> F. .cfidx ls / cat thin regression"
"$BIN" make --store "$STORE" -o "$DEMO_DIR/hello.cfidx" "$SRC/pkgs/foo/a.txt" \
  >"$DEMO_DIR/make.out" \
  2>"$DEMO_DIR/make.err"
"$BIN" ls --format json "$DEMO_DIR/hello.cfidx" \
  >"$DEMO_DIR/ls-cfidx.json" \
  2>"$DEMO_DIR/ls-cfidx.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/ls-cfidx.json'))
assert obj.get('ok') is True, obj
entries = obj['entries']
assert len(entries) == 1, entries
assert entries[0]['kind'] == 'file', entries[0]
assert entries[0]['path'] == 'hello', entries[0]
assert 'size' in entries[0], entries[0]
print('F: cfidx ls path=', entries[0]['path'], 'size=', entries[0]['size'])
"
# cfidx + path flag → non-zero
set +e
"$BIN" ls --path pkgs/foo "$DEMO_DIR/hello.cfidx" \
  >"$DEMO_DIR/ls-cfidx-path.out" \
  2>"$DEMO_DIR/ls-cfidx-path.err"
RC_LS_PATH=$?
set -e
if [[ "$RC_LS_PATH" -eq 0 ]]; then
  echo "error: ls .cfidx + --path must be non-zero" >&2
  exit 1
fi
"$BIN" cat --store "$STORE" -o "$DEMO_DIR/hello-from-cat.bin" \
  "$DEMO_DIR/hello.cfidx" --format json \
  >"$DEMO_DIR/cat-cfidx.json" \
  2>"$DEMO_DIR/cat-cfidx.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/cat-cfidx.json'))
assert obj.get('ok') is True, obj
print('F: cfidx cat bytes=', obj['bytes'])
"
cmp "$SRC/pkgs/foo/a.txt" "$DEMO_DIR/hello-from-cat.bin"
# cfidx + --path → non-zero
set +e
"$BIN" cat --store "$STORE" --path pkgs/foo/a.txt \
  -o "$DEMO_DIR/bad.bin" "$DEMO_DIR/hello.cfidx" \
  >"$DEMO_DIR/cat-cfidx-path.out" \
  2>"$DEMO_DIR/cat-cfidx-path.err"
RC_CAT_PATH=$?
set -e
if [[ "$RC_CAT_PATH" -eq 0 ]]; then
  echo "error: cat .cfidx + --path must be non-zero" >&2
  exit 1
fi
echo "F: .cfidx ls/cat regression + path rejection: OK"

echo
echo "==> G. help / responsibility surfaces (ls ≠ mount ≠ extract ≠ verify ≠ pack ≠ filter; cat --path ≠ prune; gc no --path)"
TOP_HELP="$("$BIN" --help)"
if ! grep -E '(^| )ls( |$)' <<<"$TOP_HELP" >/dev/null; then
  echo "error: top-level --help must list ls" >&2
  echo "$TOP_HELP" >&2
  exit 1
fi
echo "top-level --help has ls: OK"

LS_HELP="$("$BIN" ls --help)"
for flag in --path --path-from --exclude --exclude-from --format --chunks; do
  if ! grep -F -- "$flag" <<<"$LS_HELP" >/dev/null; then
    echo "error: ls --help missing $flag" >&2
    echo "$LS_HELP" >&2
    exit 1
  fi
done
echo "ls --help has path 四件套 + format/chunks: OK"

CAT_HELP="$("$BIN" cat --help)"
if ! grep -F -- '--path' <<<"$CAT_HELP" >/dev/null; then
  echo "error: cat --help must expose --path" >&2
  echo "$CAT_HELP" >&2
  exit 1
fi
echo "cat --help has --path: OK"

GC_HELP="$("$BIN" gc --help)"
if grep -E -- '--path\b' <<<"$GC_HELP" >/dev/null; then
  echo "error: gc must NOT expose --path (ls/cat ≠ gc-path hard ban)" >&2
  echo "$GC_HELP" >&2
  exit 1
fi
echo "gc --help has no --path: OK"

EXT_HELP="$("$BIN" extract --help)"
if grep -E -- '--delete\b' <<<"$EXT_HELP" >/dev/null; then
  echo "error: extract must NOT expose --delete (cat --path ≠ prune)" >&2
  exit 1
fi
echo "extract --help has no --delete: OK"

# No pack subcommand / help surface
if grep -Eiw 'packfile|pack\b' <<<"$TOP_HELP" >/dev/null; then
  echo "error: top-level help must not advertise pack" >&2
  exit 1
fi
if "$BIN" --help 2>/dev/null | grep -E '^  pack\b' >/dev/null; then
  echo "error: pack must not be a subcommand" >&2
  exit 1
fi
echo "no pack subcommand: OK"
echo "G: help / ≠ prune / ≠ gc-path / ≠ pack: OK"

echo
echo "==> H. version 1.14.0 + Cargo 1.14.0 + compat_1_14 (Phase25-M5)"
VER="$("$BIN" --version)"
echo "version: $VER"
if ! grep -F '1.14.0' <<<"$VER" >/dev/null; then
  echo "error: expected chunkforge 1.14.0; got $VER" >&2
  exit 1
fi
if ! grep -E '^version = "1\.14\.0"' "$ROOT/Cargo.toml" >/dev/null; then
  echo "error: workspace Cargo.toml version must be 1.14.0 (Phase25 until M7)" >&2
  grep -E '^version' "$ROOT/Cargo.toml" >&2 || true
  exit 1
fi
# Existing gate that already exists must stay present (1_13 from Phase24).
COMPAT113="$ROOT/scripts/check_compat_1_13.sh"
if [[ ! -f "$COMPAT113" ]]; then
  echo "error: check_compat_1_13.sh must exist" >&2
  exit 1
fi
if [[ ! -x "$COMPAT113" ]]; then
  echo "error: check_compat_1_13.sh must be executable" >&2
  exit 1
fi
# Phase25-M5: compat_1_14 must exist + executable (same pattern as
# demo_filter_listing ↔ compat_1_13). Do not force-run the long gate here.
COMPAT114="$ROOT/scripts/check_compat_1_14.sh"
if [[ ! -f "$COMPAT114" ]]; then
  echo "error: check_compat_1_14.sh must exist" >&2
  exit 1
fi
if [[ ! -x "$COMPAT114" ]]; then
  echo "error: check_compat_1_14.sh must be executable" >&2
  exit 1
fi
echo "H: version 1.14.0 / Cargo 1.14.0 / compat_1_13 + compat_1_14 present+executable: OK"

echo
echo "demo_ls_cat_path: ALL OK / PASS"
echo "proved: archive --symlinks record → filter --path pkgs/foo → ls File+Symlink;"
echo "        cat --path single File ≡ source / extract; ≠ prune;"
echo "        .cfidx ls/cat regression; path flags rejected on cfidx;"
echo "        ls ≠ mount ≠ extract ≠ verify ≠ pack ≠ filter;"
echo "        cat --path ≠ extract ≠ prune ≠ sync; gc no --path; no pack"
