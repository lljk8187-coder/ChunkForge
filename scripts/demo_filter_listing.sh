#!/usr/bin/env bash
# Phase 24 M4 smoke: chunkforge filter persists a path-scoped subset of an
# existing .cfdir (library filter_dir_archive → encode → -o).
# Flow: archive --symlinks record full tree → filter --path pkgs/foo →
# verify green; empty-filter identity; optional drop-all-symlinks → v1;
# help / ≠ prune / ≠ gc-path / ≠ pack nails.
# Narrative:
#   filter ≠ prune ≠ gc-path ≠ sync ≠ write mount ≠ pack ≠ archive --path
# Local only — no internet.
# Usage: ./scripts/demo_filter_listing.sh
# Requires: cargo, python3.
# Env (optional):
#   CHUNKFORGE_BIN, CHUNKFORGE_PYTHON
#   CHUNKFORGE_P24_DEMO_DIR (default /tmp/cf-p24-filter-demo)
#
# Version gate expects chunkforge 1.13.0 (Phase24 mid-stream; M7 bumps 1.14.0).
#
# Gate (Phase24-M5): require check_compat_1_13.sh present + executable
# (same pattern as Phase23-M5 demo_diff_tree_symlink ↔ compat_1_12).
# Do not force-run the long gate here.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_P24_DEMO_DIR:-/tmp/cf-p24-filter-demo}"
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

# Decode .cfdir header + count KIND tags (1=File, 2=Dir, 3=Symlink).
# Prints: format_version file_count dir_count symlink_count
cfdir_summary() {
  local path="$1"
  "$PYTHON" - "$path" <<'PY'
import struct, sys
path = sys.argv[1]
data = open(path, "rb").read()
assert data[:8] == b"CFDIR\0\0\1", data[:8]
fmt_ver = struct.unpack_from("<H", data, 8)[0]
entry_count = struct.unpack_from("<Q", data, 16)[0]
off = 24
files = dirs = symlinks = 0
for _ in range(entry_count):
    path_len = struct.unpack_from("<H", data, off)[0]; off += 2
    off += path_len
    kind = data[off]; off += 1
    if kind == 1:  # File
        files += 1
        off += 4 + 8 + 8 + 32  # mode,size,mtime,blob
        chunk_count = struct.unpack_from("<Q", data, off)[0]; off += 8
        off += chunk_count * 40
    elif kind == 2:  # Dir
        dirs += 1
        off += 4  # mode
    elif kind == 3:  # Symlink
        symlinks += 1
        off += 4  # mode
        tlen = struct.unpack_from("<H", data, off)[0]; off += 2
        off += tlen
    else:
        raise SystemExit(f"unknown kind {kind} at offset")
print(f"{fmt_ver} {files} {dirs} {symlinks}")
PY
}

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
read -r FULL_VER FULL_FILES FULL_DIRS FULL_SYMS < <(cfdir_summary "$DEMO_DIR/full.cfdir")
if [[ "$FULL_VER" != "2" ]]; then
  echo "error: full record listing must be format_version=2; got $FULL_VER" >&2
  exit 1
fi
if [[ "$FULL_SYMS" -lt 1 ]]; then
  echo "error: full listing must retain ≥1 Symlink; got $FULL_SYMS" >&2
  exit 1
fi
echo "A: format_version=$FULL_VER files=$FULL_FILES dirs=$FULL_DIRS symlinks=$FULL_SYMS: OK"

echo
echo "==> B. filter --path pkgs/foo -o foo.cfdir (--format json)"
"$BIN" filter --path pkgs/foo -o "$DEMO_DIR/foo.cfdir" \
  "$DEMO_DIR/full.cfdir" --format json \
  >"$DEMO_DIR/filter-foo.json" \
  2>"$DEMO_DIR/filter-foo.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/filter-foo.json'))
assert obj.get('ok') is True, obj
assert obj.get('dry_run') is False, obj
assert 'input' in obj and 'output' in obj, obj
assert obj.get('files', 0) >= 1, obj
assert obj.get('symlinks', 0) >= 1, obj
assert obj.get('excluded', 0) >= 1, obj  # bar + readme leaves out
print('B: files=', obj['files'], 'dirs=', obj['dirs'],
      'symlinks=', obj['symlinks'], 'excluded=', obj['excluded'])
"
read -r FOO_VER FOO_FILES FOO_DIRS FOO_SYMS < <(cfdir_summary "$DEMO_DIR/foo.cfdir")
if [[ "$FOO_VER" != "2" ]]; then
  echo "error: symlink-keeping subset must write format_version=2; got $FOO_VER" >&2
  exit 1
fi
if [[ "$FOO_SYMS" -lt 1 ]]; then
  echo "error: --path pkgs/foo must keep link.txt Symlink; got $FOO_SYMS" >&2
  exit 1
fi
echo "B: format_version=$FOO_VER files=$FOO_FILES dirs=$FOO_DIRS symlinks=$FOO_SYMS: OK (Symlink keep → v2)"

echo
echo "==> C. verify --store … foo.cfdir green"
"$BIN" verify --store "$STORE" "$DEMO_DIR/foo.cfdir" --format json \
  >"$DEMO_DIR/verify-foo.json" \
  2>"$DEMO_DIR/verify-foo.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/verify-foo.json'))
assert obj.get('ok') is True, obj
assert obj.get('kind') == 'cfdir', obj
print('C: verify ok; files=', obj.get('files'), 'symlinks=', obj.get('symlinks'))
"
echo "C: verify subset green: OK"

echo
echo "==> D. empty-filter identity (diff identical)"
"$BIN" filter -o "$DEMO_DIR/copy.cfdir" "$DEMO_DIR/full.cfdir" --format json \
  >"$DEMO_DIR/filter-id.json" \
  2>"$DEMO_DIR/filter-id.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/filter-id.json'))
assert obj.get('ok') is True, obj
assert obj.get('excluded', -1) == 0, obj
print('D: empty filter excluded=', obj['excluded'],
      'files=', obj['files'], 'symlinks=', obj['symlinks'])
"
set +e
"$BIN" diff "$DEMO_DIR/full.cfdir" "$DEMO_DIR/copy.cfdir" --format json \
  >"$DEMO_DIR/diff-id.json" \
  2>"$DEMO_DIR/diff-id.err"
RC_ID=$?
set -e
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/diff-id.json'))
for k in ('added', 'removed', 'changed', 'meta_changed'):
    assert obj.get(k) == [], (k, obj)
print('D: diff categories empty (logical identity)')
"
if [[ "$RC_ID" -ne 0 ]]; then
  echo "error: empty-filter copy must diff identical (exit 0); got $RC_ID" >&2
  cat "$DEMO_DIR/diff-id.err" >&2 || true
  exit 1
fi
echo "D: empty filter ≡ identity (diff exit 0): OK"

echo
echo "==> E. optional: filter out all Symlinks → encode v1"
"$BIN" filter --exclude pkgs/foo/link.txt -o "$DEMO_DIR/nosym.cfdir" \
  "$DEMO_DIR/full.cfdir" --format json \
  >"$DEMO_DIR/filter-nosym.json" \
  2>"$DEMO_DIR/filter-nosym.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/filter-nosym.json'))
assert obj.get('ok') is True, obj
assert obj.get('symlinks', -1) == 0, obj
print('E: symlinks=', obj['symlinks'], 'files=', obj['files'], 'excluded=', obj['excluded'])
"
read -r NS_VER NS_FILES NS_DIRS NS_SYMS < <(cfdir_summary "$DEMO_DIR/nosym.cfdir")
if [[ "$NS_VER" != "1" ]]; then
  echo "error: dropping all Symlinks must encode format_version=1; got $NS_VER" >&2
  exit 1
fi
if [[ "$NS_SYMS" != "0" ]]; then
  echo "error: nosym listing must have 0 Symlink kinds; got $NS_SYMS" >&2
  exit 1
fi
echo "E: format_version=$NS_VER files=$NS_FILES dirs=$NS_DIRS symlinks=$NS_SYMS: OK (all Symlinks filtered → v1)"

echo
echo "==> F. dry-run does not write; existing -o without --force → non-zero"
"$BIN" filter --path pkgs/foo --dry-run -o "$DEMO_DIR/dry.cfdir" \
  "$DEMO_DIR/full.cfdir" --format json \
  >"$DEMO_DIR/filter-dry.json" \
  2>"$DEMO_DIR/filter-dry.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/filter-dry.json'))
assert obj.get('ok') is True, obj
assert obj.get('dry_run') is True, obj
print('F: dry_run JSON ok; files=', obj['files'])
"
if [[ -e "$DEMO_DIR/dry.cfdir" ]]; then
  echo "error: --dry-run must not create -o" >&2
  exit 1
fi
set +e
"$BIN" filter --path pkgs/foo -o "$DEMO_DIR/foo.cfdir" \
  "$DEMO_DIR/full.cfdir" \
  >"$DEMO_DIR/filter-noforce.out" \
  2>"$DEMO_DIR/filter-noforce.err"
RC_NF=$?
set -e
if [[ "$RC_NF" -eq 0 ]]; then
  echo "error: existing -o without --force must be non-zero" >&2
  exit 1
fi
echo "F: dry-run no write + refuse existing -o: OK"

echo
echo "==> G. help / responsibility surfaces (filter ≠ prune ≠ gc-path ≠ pack)"
TOP_HELP="$("$BIN" --help)"
if ! grep -E '(^| )filter( |$)' <<<"$TOP_HELP" >/dev/null; then
  echo "error: top-level --help must list filter" >&2
  echo "$TOP_HELP" >&2
  exit 1
fi
echo "top-level --help has filter: OK"

FILT_HELP="$("$BIN" filter --help)"
for flag in --path --path-from --exclude --exclude-from --dry-run --force --format; do
  if ! grep -F -- "$flag" <<<"$FILT_HELP" >/dev/null; then
    echo "error: filter --help missing $flag" >&2
    echo "$FILT_HELP" >&2
    exit 1
  fi
done
echo "filter --help has path 四件套 + dry-run/force/format: OK"

GC_HELP="$("$BIN" gc --help)"
if grep -E -- '--path\b' <<<"$GC_HELP" >/dev/null; then
  echo "error: gc must NOT expose --path (filter ≠ gc-path hard ban)" >&2
  echo "$GC_HELP" >&2
  exit 1
fi
echo "gc --help has no --path: OK"

EXT_HELP="$("$BIN" extract --help)"
if grep -E -- '--delete\b' <<<"$EXT_HELP" >/dev/null; then
  echo "error: extract must NOT expose --delete (filter ≠ prune)" >&2
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
echo "==> H. version 1.13.0 + Cargo 1.13.0 + compat_1_13 (Phase24-M5)"
VER="$("$BIN" --version)"
echo "version: $VER"
if ! grep -F '1.13.0' <<<"$VER" >/dev/null; then
  echo "error: expected chunkforge 1.13.0 (M7 bumps 1.14.0); got $VER" >&2
  exit 1
fi
if ! grep -E '^version = "1\.13\.0"' "$ROOT/Cargo.toml" >/dev/null; then
  echo "error: workspace Cargo.toml version must be 1.13.0 until M7" >&2
  grep -E '^version' "$ROOT/Cargo.toml" >&2 || true
  exit 1
fi
# Existing gate that already exists must stay present (1_12 from Phase23).
COMPAT112="$ROOT/scripts/check_compat_1_12.sh"
if [[ ! -f "$COMPAT112" ]]; then
  echo "error: check_compat_1_12.sh must exist" >&2
  exit 1
fi
if [[ ! -x "$COMPAT112" ]]; then
  echo "error: check_compat_1_12.sh must be executable" >&2
  exit 1
fi
# Phase24-M5: compat_1_13 must exist + executable (same pattern as
# demo_diff_tree_symlink ↔ compat_1_12). Do not force-run the long gate here.
COMPAT113="$ROOT/scripts/check_compat_1_13.sh"
if [[ ! -f "$COMPAT113" ]]; then
  echo "error: check_compat_1_13.sh must exist" >&2
  exit 1
fi
if [[ ! -x "$COMPAT113" ]]; then
  echo "error: check_compat_1_13.sh must be executable" >&2
  exit 1
fi
echo "H: version 1.13.0 / Cargo 1.13.0 / compat_1_12 + compat_1_13 present+executable: OK"

echo
echo "demo_filter_listing: ALL OK / PASS"
echo "proved: archive --symlinks record full → filter --path pkgs/foo → verify green;"
echo "        empty filter ≡ identity (diff);"
echo "        Symlink keep → v2; all Symlinks filtered → v1 via encode;"
echo "        dry-run / refuse overwrite;"
echo "        filter ≠ prune ≠ gc-path ≠ sync ≠ write mount ≠ pack ≠ archive --path"
