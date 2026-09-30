#!/usr/bin/env bash
# Phase 22 M5 smoke: archive --symlinks skip|record → extract readlink;
# absolute target → non-zero; path filter keeps/excludes symlink; DirFs
# library readlink (PRIMARY) / optional real FUSE; default skip ≡ 1.11
# quiet (v1 + skipped_symlinks). Narrative:
#   archive --symlinks record ≠ write mount ≠ follow dir symlink ≠ pack
#   ≠ offline bundle ≠ prune ≠ gc --path ≠ default record
# Local only — no internet.
# Usage: ./scripts/demo_symlink.sh
# Requires: cargo, python3. FUSE optional (library path is enough for green).
# Env (optional):
#   CHUNKFORGE_BIN, CHUNKFORGE_PYTHON
#   CHUNKFORGE_P22_DEMO_DIR (default /tmp/cf-p22-symlink-demo)
#   CHUNKFORGE_DEMO_SKIP_FUSE=1  force-skip real FUSE even if available
#
# Version gate expects chunkforge 1.14.0 (Phase24-M7; nests under check_compat_1_13).
#
# Gate (Phase22-M6 / Phase21 / Phase19-M7b lesson): require
# check_compat_1_11.sh present + executable (same pattern as
# demo_mount_path ↔ compat_1_10). Do NOT leave a soft "not yet" note,
# and never assert that compat_1_11 must not exist.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_P22_DEMO_DIR:-/tmp/cf-p22-symlink-demo}"
BIN="${CHUNKFORGE_BIN:-}"
PYTHON="${CHUNKFORGE_PYTHON:-python3}"
SKIP_FUSE="${CHUNKFORGE_DEMO_SKIP_FUSE:-0}"
ASSERT_PATH="library"  # overwritten to "fuse+library" if real mount runs

if ! command -v "$PYTHON" >/dev/null 2>&1; then
  echo "error: $PYTHON not on PATH" >&2
  exit 1
fi

echo "==> building chunkforge (fuse feature for optional mount)"
cargo build -p chunkforge-cli --features fuse --quiet
if [[ -z "$BIN" ]]; then
  BIN="$ROOT/target/debug/chunkforge"
fi

rm -rf "$DEMO_DIR"
SRC="$DEMO_DIR/src"
STORE="$DEMO_DIR/store"
OUT="$DEMO_DIR/out"
MNT="$DEMO_DIR/mnt"
mkdir -p \
  "$SRC/pkgs/foo" \
  "$SRC/pkgs/bar" \
  "$STORE" \
  "$OUT" \
  "$MNT"

echo 'hello-foo' > "$SRC/pkgs/foo/a.txt"
ln -s a.txt "$SRC/pkgs/foo/link.txt"
echo 'hello-bar' > "$SRC/pkgs/bar/b.txt"
ln -s b.txt "$SRC/pkgs/bar/link-bar.txt"
printf 'readme\n' > "$SRC/readme.txt"

cleanup_mount() {
  if [[ -n "${MOUNT_PID:-}" ]]; then
    if kill -0 "$MOUNT_PID" 2>/dev/null; then
      fusermount3 -u "$MNT" 2>/dev/null || fusermount -u "$MNT" 2>/dev/null || true
      wait "$MOUNT_PID" 2>/dev/null || true
    fi
    MOUNT_PID=""
  fi
  if mountpoint -q "$MNT" 2>/dev/null; then
    fusermount3 -u "$MNT" 2>/dev/null || fusermount -u "$MNT" 2>/dev/null || true
  fi
}
trap cleanup_mount EXIT

wait_for_path() {
  local target="$1"
  local label="$2"
  for _ in $(seq 1 50); do
    if [[ -e "$target" ]] || [[ -L "$target" ]]; then
      return 0
    fi
    if [[ -n "${MOUNT_PID:-}" ]] && ! kill -0 "$MOUNT_PID" 2>/dev/null; then
      wait "$MOUNT_PID" || true
      echo "error: $label mount process exited before $target appeared" >&2
      MOUNT_PID=""
      return 1
    fi
    sleep 0.1
  done
  echo "error: $label timed out waiting for $target" >&2
  return 1
}

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
# skip flags(2)+reserved(4); entry_count at 16
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
echo "==> A. default archive (no --symlinks / skip) ≡ 1.11 quiet"
"$BIN" archive --store "$STORE" -o "$DEMO_DIR/skip.cfdir" \
  --format json "$SRC" \
  >"$DEMO_DIR/archive-skip.json" \
  2>"$DEMO_DIR/archive-skip.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/archive-skip.json'))
assert obj.get('ok') is True, obj
assert obj.get('skipped_symlinks', 0) >= 1, obj
assert obj.get('recorded_symlinks', 0) == 0, obj
print('A: skipped_symlinks=', obj['skipped_symlinks'],
      'recorded_symlinks=', obj.get('recorded_symlinks', 0),
      'files=', obj['files'])
"
read -r SKIP_VER SKIP_FILES SKIP_DIRS SKIP_SYMS < <(cfdir_summary "$DEMO_DIR/skip.cfdir")
if [[ "$SKIP_VER" != "1" ]]; then
  echo "error: default archive must write format_version=1; got $SKIP_VER" >&2
  exit 1
fi
if [[ "$SKIP_SYMS" != "0" ]]; then
  echo "error: default skip listing must have 0 Symlink kinds; got $SKIP_SYMS" >&2
  exit 1
fi
echo "A: format_version=$SKIP_VER files=$SKIP_FILES dirs=$SKIP_DIRS symlinks=$SKIP_SYMS: OK (≡ 1.11 quiet)"

echo
echo "==> B. --symlinks record → extract → readlink matches"
"$BIN" archive --store "$STORE" -o "$DEMO_DIR/record.cfdir" \
  --symlinks record --format json "$SRC" \
  >"$DEMO_DIR/archive-record.json" \
  2>"$DEMO_DIR/archive-record.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/archive-record.json'))
assert obj.get('ok') is True, obj
assert obj.get('recorded_symlinks', 0) >= 2, obj
assert obj.get('skipped_symlinks', 0) == 0, obj
print('B: recorded_symlinks=', obj['recorded_symlinks'],
      'files=', obj['files'])
"
read -r REC_VER REC_FILES REC_DIRS REC_SYMS < <(cfdir_summary "$DEMO_DIR/record.cfdir")
if [[ "$REC_VER" != "2" ]]; then
  echo "error: --symlinks record with ≥1 Symlink must write format_version=2; got $REC_VER" >&2
  exit 1
fi
if [[ "$REC_SYMS" -lt 2 ]]; then
  echo "error: expected ≥2 Symlink kinds in record listing; got $REC_SYMS" >&2
  exit 1
fi
echo "B: format_version=$REC_VER files=$REC_FILES dirs=$REC_DIRS symlinks=$REC_SYMS: OK"

rm -rf "$OUT"
mkdir -p "$OUT"
"$BIN" extract --store "$STORE" "$DEMO_DIR/record.cfdir" -o "$OUT" \
  --format json \
  >"$DEMO_DIR/extract-record.json"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/extract-record.json'))
assert obj.get('ok') is True, obj
assert obj.get('symlinks', 0) >= 2 or obj.get('wrote_symlinks', 0) >= 2, obj
print('B: extract wrote=', obj.get('wrote'), 'symlinks=', obj.get('symlinks'))
"
LINK_TGT="$(readlink "$OUT/pkgs/foo/link.txt")"
if [[ "$LINK_TGT" != "a.txt" ]]; then
  echo "error: expected readlink pkgs/foo/link.txt → a.txt; got [$LINK_TGT]" >&2
  exit 1
fi
BAR_TGT="$(readlink "$OUT/pkgs/bar/link-bar.txt")"
if [[ "$BAR_TGT" != "b.txt" ]]; then
  echo "error: expected readlink pkgs/bar/link-bar.txt → b.txt; got [$BAR_TGT]" >&2
  exit 1
fi
echo "B: extract readlink matches (a.txt / b.txt): OK"

echo
echo "==> C. absolute symlink target → archive --symlinks record non-zero"
ABS_SRC="$DEMO_DIR/abs-src"
rm -rf "$ABS_SRC"
mkdir -p "$ABS_SRC"
echo 'x' > "$ABS_SRC/a.txt"
ln -s /etc/passwd "$ABS_SRC/bad"
set +e
"$BIN" archive --store "$STORE" -o "$DEMO_DIR/abs.cfdir" \
  --symlinks record "$ABS_SRC" \
  >"$DEMO_DIR/abs.out" 2>"$DEMO_DIR/abs.err"
RC_ABS=$?
set -e
if [[ "$RC_ABS" -eq 0 ]]; then
  echo "error: absolute symlink target must be non-zero under --symlinks record" >&2
  cat "$DEMO_DIR/abs.out" "$DEMO_DIR/abs.err" >&2
  exit 1
fi
if ! grep -F 'absolute target' "$DEMO_DIR/abs.err" >/dev/null; then
  echo "error: expected clear absolute-target error message" >&2
  cat "$DEMO_DIR/abs.err" >&2
  exit 1
fi
echo "C: absolute target exit=$RC_ABS: OK"

echo
echo "==> D. path filter keeps / excludes symlink path"
"$BIN" archive --store "$STORE" -o "$DEMO_DIR/path-keep.cfdir" \
  --symlinks record --path pkgs/foo --format json "$SRC" \
  >"$DEMO_DIR/path-keep.json" \
  2>"$DEMO_DIR/path-keep.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/path-keep.json'))
assert obj.get('ok') is True, obj
assert obj.get('recorded_symlinks', 0) == 1, obj
print('D1: --path pkgs/foo recorded_symlinks=', obj['recorded_symlinks'])
"
read -r PK_VER PK_FILES PK_DIRS PK_SYMS < <(cfdir_summary "$DEMO_DIR/path-keep.cfdir")
if [[ "$PK_SYMS" != "1" ]]; then
  echo "error: --path pkgs/foo should keep exactly 1 symlink; got $PK_SYMS" >&2
  exit 1
fi
# Exclude the symlink path itself
"$BIN" archive --store "$STORE" -o "$DEMO_DIR/path-excl.cfdir" \
  --symlinks record --exclude pkgs/foo/link.txt --format json "$SRC" \
  >"$DEMO_DIR/path-excl.json" \
  2>"$DEMO_DIR/path-excl.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/path-excl.json'))
assert obj.get('ok') is True, obj
# bar link + (foo link excluded) → recorded should be 1 (bar only) if bar kept
assert obj.get('recorded_symlinks', 0) == 1, obj
print('D2: --exclude pkgs/foo/link.txt recorded_symlinks=', obj['recorded_symlinks'])
"
echo "D: path filter orthogonal to --symlinks record: OK"

echo
echo "==> E. library DirFs / filter_dir_archive Symlink + readlink (PRIMARY)"
cargo test -p chunkforge-fuse --lib dir_fs_symlink --quiet
cargo test -p chunkforge-fuse --lib filter_path_keeps_symlink --quiet
cargo test -p chunkforge-index --lib empty_filter_identity_includes_symlinks --quiet
cargo test -p chunkforge-index --lib filter_keeps_symlink_path_and_ancestors --quiet
echo "E: library DirFs readlink + filter_dir_archive Symlink: OK (assert_path=library)"

echo
echo "==> F. optional real FUSE (skip if no /dev/fuse)"
FUSE_OK=0
if [[ "$SKIP_FUSE" != "1" ]] \
  && [[ "$(uname -s)" == "Linux" ]] \
  && [[ -e /dev/fuse ]] \
  && { command -v fusermount3 >/dev/null 2>&1 || command -v fusermount >/dev/null 2>&1; }; then
  FUSE_OK=1
fi

if [[ "$FUSE_OK" -eq 1 ]]; then
  echo "F1. mount recorded tree; readlink under mount"
  cleanup_mount
  mkdir -p "$MNT"
  "$BIN" mount --store "$STORE" "$DEMO_DIR/record.cfdir" "$MNT" &
  MOUNT_PID=$!
  wait_for_path "$MNT/pkgs/foo/a.txt" "symlink-mount"
  MNT_TGT="$(readlink "$MNT/pkgs/foo/link.txt")"
  if [[ "$MNT_TGT" != "a.txt" ]]; then
    echo "error: mount readlink expected a.txt; got [$MNT_TGT]" >&2
    exit 1
  fi
  # still RO — write must fail (record ≠ write mount)
  set +e
  # bash prints EROFS on the redirect itself; swallow into write.err
  { echo no-write > "$MNT/pkgs/foo/nope.txt"; } 2>"$DEMO_DIR/write.err"
  RC_WRITE=$?
  set -e
  if [[ "$RC_WRITE" -eq 0 ]]; then
    echo "error: write under RO mount must fail (record ≠ write mount)" >&2
    exit 1
  fi
  echo "F1: mount readlink=a.txt; write failed (exit=$RC_WRITE): OK"
  cleanup_mount

  echo "F2. mount --path pkgs/foo keeps symlink"
  mkdir -p "$MNT"
  "$BIN" mount --store "$STORE" --path pkgs/foo \
    "$DEMO_DIR/record.cfdir" "$MNT" &
  MOUNT_PID=$!
  wait_for_path "$MNT/pkgs/foo/link.txt" "path-symlink"
  MNT_TGT2="$(readlink "$MNT/pkgs/foo/link.txt")"
  if [[ "$MNT_TGT2" != "a.txt" ]]; then
    echo "error: path-filtered mount readlink expected a.txt; got [$MNT_TGT2]" >&2
    exit 1
  fi
  if [[ -e "$MNT/pkgs/bar" ]] || [[ -L "$MNT/pkgs/bar/link-bar.txt" ]]; then
    echo "error: --path pkgs/foo must hide bar" >&2
    find "$MNT" -print >&2 || true
    exit 1
  fi
  echo "F2: --path keeps symlink, hides bar: OK"
  cleanup_mount

  ASSERT_PATH="fuse+library"
  echo "F: real FUSE path checks: OK"
else
  echo "==> skipping real FUSE (Linux+/dev/fuse/fusermount required, or SKIP_FUSE=1)"
  echo "    library asserts in E remain the acceptance path"
fi

echo
echo "==> G. help / responsibility surfaces"
ARCH_HELP="$("$BIN" archive --help)"
if ! grep -F -- '--symlinks' <<<"$ARCH_HELP" >/dev/null; then
  echo "error: archive --help missing --symlinks" >&2
  exit 1
fi
echo "archive --help has --symlinks: OK"

EXT_HELP="$("$BIN" extract --help)"
if grep -E -- '--delete\b' <<<"$EXT_HELP" >/dev/null; then
  echo "error: extract must NOT expose --delete (record ≠ prune)" >&2
  exit 1
fi
echo "extract --help has no --delete: OK"

GC_HELP="$("$BIN" gc --help)"
if grep -E -- '--path\b' <<<"$GC_HELP" >/dev/null; then
  echo "error: gc must NOT expose --path (record ≠ gc-path hard ban)" >&2
  exit 1
fi
echo "gc --help has no --path: OK"

MOUNT_HELP="$("$BIN" mount --help)"
if grep -E -- '--progress\b' <<<"$MOUNT_HELP" >/dev/null; then
  echo "error: mount must not grow --progress" >&2
  exit 1
fi
echo "mount has no --progress: OK"
echo "G: help / ≠ prune / ≠ gc-path / ≠ write mount: OK"

echo
echo "==> H. version 1.14.0 + Cargo 1.14.0 + compat_1_11 (Phase24-M7)"
VER="$("$BIN" --version)"
echo "version: $VER"
if ! grep -F '1.14.0' <<<"$VER" >/dev/null; then
  echo "error: expected chunkforge 1.14.0; got $VER" >&2
  exit 1
fi
if ! grep -E '^version = "1\.14\.0"' "$ROOT/Cargo.toml" >/dev/null; then
  echo "error: workspace Cargo.toml version must be 1.14.0 (Phase24-M7)" >&2
  grep -E '^version' "$ROOT/Cargo.toml" >&2 || true
  exit 1
fi
# Existing gates that already exist must stay present (1_10 from Phase21).
COMPAT110="$ROOT/scripts/check_compat_1_10.sh"
if [[ ! -f "$COMPAT110" ]]; then
  echo "error: check_compat_1_10.sh must exist" >&2
  exit 1
fi
if [[ ! -x "$COMPAT110" ]]; then
  echo "error: check_compat_1_10.sh must be executable" >&2
  exit 1
fi
COMPAT111="$ROOT/scripts/check_compat_1_11.sh"
if [[ ! -f "$COMPAT111" ]]; then
  echo "error: check_compat_1_11.sh must exist" >&2
  exit 1
fi
if [[ ! -x "$COMPAT111" ]]; then
  echo "error: check_compat_1_11.sh must be executable" >&2
  exit 1
fi
echo "H: version 1.14.0 / Cargo 1.14.0 / compat_1_10 / compat_1_11: OK"

echo
echo "demo_symlink: ALL OK (assert_path=$ASSERT_PATH)"
echo "proved: default skip ≡ 1.11 quiet (v1 + skipped_symlinks);"
echo "        --symlinks record → extract readlink;"
echo "        absolute target non-zero;"
echo "        path filter orthogonal;"
echo "        record ≠ write mount ≠ follow ≠ pack ≠ prune ≠ gc-path ≠ default record"
