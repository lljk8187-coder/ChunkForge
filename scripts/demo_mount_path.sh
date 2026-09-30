#!/usr/bin/env bash
# Phase 21 M3 smoke: mount --path / --path-from / --exclude / --exclude-from
# subset (DirFs / filter_dir_archive library asserts PRIMARY; real FUSE
# optional when /dev/fuse + fusermount3 present); default no flags ≡ 1.10
# full tree; exclude-from + path-from combine; .cfidx + path → non-zero;
# missing path-from → non-zero; mount path ≠ write mount ≠ prune ≠ gc-path.
# Local only — no internet.
# Usage: ./scripts/demo_mount_path.sh
# Requires: cargo, python3. FUSE optional (library path is enough for green).
# Env (optional):
#   CHUNKFORGE_BIN, CHUNKFORGE_PYTHON
#   CHUNKFORGE_P21_DEMO_DIR (default /tmp/cf-p21-mount-path-demo)
#   CHUNKFORGE_DEMO_SKIP_FUSE=1  force-skip real FUSE even if available
#
# Version gate expects chunkforge 1.10.0 (Phase21-M7 bumps to 1.11.0 later).
# Note (M7b lesson / M3 hard rule): do NOT hard-assert that
# check_compat_1_10.sh must exist or must not exist — that script is M4.
# Mentions of compat_1_10 below are comment / note-only.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_P21_DEMO_DIR:-/tmp/cf-p21-mount-path-demo}"
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
MNT="$DEMO_DIR/mnt"
mkdir -p \
  "$SRC/packages/foo" \
  "$SRC/packages/bar" \
  "$SRC/junk" \
  "$STORE" \
  "$MNT"

echo 'foo-a' > "$SRC/packages/foo/a.txt"
echo 'foo-b' > "$SRC/packages/foo/b.txt"
echo 'bar-c' > "$SRC/packages/bar/c.txt"
echo 'junk-x' > "$SRC/junk/x.txt"
printf 'readme\n' > "$SRC/readme.txt"

# path-from include: packages/foo + packages/bar (OR)
printf '%s\n' \
  '# include prefixes (path-from)' \
  'packages/foo' \
  '' \
  'packages/bar' \
  > "$DEMO_DIR/include.txt"

# exclude-from: drop packages/bar/ even when included via path-from
printf '%s\n' \
  '# exclude patterns' \
  'packages/bar/' \
  > "$DEMO_DIR/exclude.txt"

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
    if [[ -e "$target" ]]; then
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

echo
echo "==> A. archive full tree (baseline for mount path)"
"$BIN" archive --store "$STORE" -o "$DEMO_DIR/full.cfdir" \
  --format json "$SRC" \
  >"$DEMO_DIR/archive-full.json"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/archive-full.json'))
assert obj.get('ok') is True, obj
assert isinstance(obj.get('files'), int) and obj['files'] >= 4, obj
print('archive full files=', obj['files'])
"
# Blob index for .cfidx + path rejection
dd if=/dev/urandom of="$DEMO_DIR/blob.bin" bs=4096 count=1 status=none
"$BIN" make --store "$STORE" -o "$DEMO_DIR/blob.cfidx" \
  --chunk-size 4096:4096:4096 --format json "$DEMO_DIR/blob.bin" \
  >"$DEMO_DIR/make.json"
echo "A: full .cfdir + .cfidx: OK"

echo
echo "==> B. library DirFs / filter_dir_archive (PRIMARY assert path)"
# Named unit tests cover: include subset, exclude hide, empty filter ≡ full.
cargo test -p chunkforge-index --lib filter_dir --quiet
cargo test -p chunkforge-fuse --lib filter_ --quiet
echo "B: library filter_dir_archive + DirFs: OK (assert_path=library)"

echo
echo "==> C. optional real FUSE (skip if no /dev/fuse)"
FUSE_OK=0
if [[ "$SKIP_FUSE" != "1" ]] \
  && [[ "$(uname -s)" == "Linux" ]] \
  && [[ -e /dev/fuse ]] \
  && { command -v fusermount3 >/dev/null 2>&1 || command -v fusermount >/dev/null 2>&1; }; then
  FUSE_OK=1
fi

if [[ "$FUSE_OK" -eq 1 ]]; then
  echo "C1. mount --path packages/foo (subset)"
  cleanup_mount
  mkdir -p "$MNT"
  "$BIN" mount --store "$STORE" --path packages/foo \
    "$DEMO_DIR/full.cfdir" "$MNT" &
  MOUNT_PID=$!
  wait_for_path "$MNT/packages/foo/a.txt" "path-subset"
  cmp "$SRC/packages/foo/a.txt" "$MNT/packages/foo/a.txt"
  cmp "$SRC/packages/foo/b.txt" "$MNT/packages/foo/b.txt"
  if [[ -e "$MNT/packages/bar" ]] || [[ -e "$MNT/junk" ]] || [[ -e "$MNT/readme.txt" ]]; then
    echo "error: --path packages/foo must hide bar/junk/readme" >&2
    find "$MNT" -print >&2 || true
    exit 1
  fi
  # root should only show packages/
  ROOT_ENTRIES="$(find "$MNT" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort | tr '\n' ' ')"
  if [[ "$ROOT_ENTRIES" != "packages " ]]; then
    echo "error: expected only packages/ under mount; got: [$ROOT_ENTRIES]" >&2
    exit 1
  fi
  echo "C1: --path subset visible, non-matches absent: OK"
  cleanup_mount

  echo "C2. mount no path flags ≡ full tree (1.10 quiet)"
  mkdir -p "$MNT"
  "$BIN" mount --store "$STORE" "$DEMO_DIR/full.cfdir" "$MNT" &
  MOUNT_PID=$!
  wait_for_path "$MNT/readme.txt" "full-tree"
  cmp "$SRC/packages/foo/a.txt" "$MNT/packages/foo/a.txt"
  cmp "$SRC/packages/bar/c.txt" "$MNT/packages/bar/c.txt"
  cmp "$SRC/junk/x.txt" "$MNT/junk/x.txt"
  cmp "$SRC/readme.txt" "$MNT/readme.txt"
  echo "C2: no flags ≡ full tree: OK"
  cleanup_mount

  echo "C3. mount --path-from + --exclude-from combined"
  mkdir -p "$MNT"
  "$BIN" mount --store "$STORE" \
    --path-from "$DEMO_DIR/include.txt" \
    --exclude-from "$DEMO_DIR/exclude.txt" \
    "$DEMO_DIR/full.cfdir" "$MNT" &
  MOUNT_PID=$!
  wait_for_path "$MNT/packages/foo/a.txt" "path-from+exclude-from"
  cmp "$SRC/packages/foo/a.txt" "$MNT/packages/foo/a.txt"
  if [[ -e "$MNT/packages/bar" ]]; then
    echo "error: exclude-from packages/bar/ must hide bar under mount" >&2
    find "$MNT" -print >&2 || true
    exit 1
  fi
  if [[ -e "$MNT/junk" ]] || [[ -e "$MNT/readme.txt" ]]; then
    echo "error: path-from should not include junk/readme" >&2
    find "$MNT" -print >&2 || true
    exit 1
  fi
  echo "C3: path-from + exclude-from: OK"
  cleanup_mount

  # RO write-fail sanity (mount path ≠ write mount)
  echo "C4. write under mount → fail (still RO)"
  mkdir -p "$MNT"
  "$BIN" mount --store "$STORE" --path packages/foo \
    "$DEMO_DIR/full.cfdir" "$MNT" &
  MOUNT_PID=$!
  wait_for_path "$MNT/packages/foo/a.txt" "ro-check"
  set +e
  echo no-write > "$MNT/packages/foo/nope.txt" 2>"$DEMO_DIR/write.err"
  RC_WRITE=$?
  set -e
  if [[ "$RC_WRITE" -eq 0 ]]; then
    echo "error: write under RO mount must fail (mount path ≠ write mount)" >&2
    exit 1
  fi
  echo "C4: write failed as expected (exit=$RC_WRITE): OK"
  cleanup_mount

  ASSERT_PATH="fuse+library"
  echo "C: real FUSE path checks: OK"
else
  echo "==> skipping real FUSE (Linux+/dev/fuse/fusermount required, or SKIP_FUSE=1)"
  echo "    library asserts in B remain the acceptance path"
fi

echo
echo "==> D. .cfidx + path flag → non-zero; missing path-from → non-zero"
set +e
"$BIN" mount --store "$STORE" --path packages/foo \
  "$DEMO_DIR/blob.cfidx" "$MNT" \
  >"$DEMO_DIR/cfidx-path.out" \
  2>"$DEMO_DIR/cfidx-path.err"
RC_CFIDX=$?
set -e
if [[ "$RC_CFIDX" -eq 0 ]]; then
  echo "error: mount .cfidx + --path must be non-zero" >&2
  cat "$DEMO_DIR/cfidx-path.out" "$DEMO_DIR/cfidx-path.err" >&2
  exit 1
fi
echo "mount .cfidx + --path exit=$RC_CFIDX: OK"

set +e
"$BIN" mount --store "$STORE" \
  --path-from "$DEMO_DIR/does-not-exist.txt" \
  "$DEMO_DIR/full.cfdir" "$MNT" \
  >"$DEMO_DIR/missing-from.out" \
  2>"$DEMO_DIR/missing-from.err"
RC_MISS=$?
set -e
if [[ "$RC_MISS" -eq 0 ]]; then
  echo "error: missing path-from file must be non-zero" >&2
  cat "$DEMO_DIR/missing-from.out" "$DEMO_DIR/missing-from.err" >&2
  exit 1
fi
echo "missing path-from exit=$RC_MISS: OK"
echo "D: rejection paths: OK"

echo
echo "==> E. help / responsibility surfaces"
MOUNT_HELP="$("$BIN" mount --help)"
for flag in --path --path-from --exclude --exclude-from; do
  if ! grep -F -- "$flag" <<<"$MOUNT_HELP" >/dev/null; then
    echo "error: mount --help missing $flag" >&2
    echo "$MOUNT_HELP" >&2
    exit 1
  fi
done
echo "mount --help has path quartet: OK"

GC_HELP="$("$BIN" gc --help)"
if grep -E -- '--path\b' <<<"$GC_HELP" >/dev/null; then
  echo "error: gc must NOT expose --path (mount path ≠ gc-path hard ban)" >&2
  echo "$GC_HELP" >&2
  exit 1
fi
if grep -F -- '--path-from' <<<"$GC_HELP" >/dev/null; then
  echo "error: gc must NOT expose --path-from" >&2
  exit 1
fi
echo "gc --help has no --path / --path-from: OK"

# mount has no --format / no --progress (session-typed; ops-json stays absent)
if grep -E -- '--format\b' <<<"$MOUNT_HELP" >/dev/null; then
  echo "error: mount must not grow --format json (session-typed)" >&2
  exit 1
fi
if grep -E -- '--progress\b' <<<"$MOUNT_HELP" >/dev/null; then
  echo "error: mount must not grow --progress (session-typed; Phase21 non-goal)" >&2
  exit 1
fi
echo "mount has no --format / --progress: OK"
echo "E: help / ≠ gc-path / no session JSON-progress: OK"

echo
echo "==> F. version 1.10.0 + compat_1_9; compat_1_10 note-only (M4)"
VER="$("$BIN" --version)"
echo "version: $VER"
if ! grep -F '1.10.0' <<<"$VER" >/dev/null; then
  echo "error: expected chunkforge 1.10.0 (M7 bumps later); got $VER" >&2
  exit 1
fi
# Workspace Cargo.toml must still say 1.10.0
if ! grep -E '^version = "1\.10\.0"' "$ROOT/Cargo.toml" >/dev/null; then
  echo "error: workspace Cargo.toml version must stay 1.10.0 until M7" >&2
  grep -E '^version' "$ROOT/Cargo.toml" >&2 || true
  exit 1
fi
COMPAT19="$ROOT/scripts/check_compat_1_9.sh"
if [[ ! -f "$COMPAT19" ]]; then
  echo "error: check_compat_1_9.sh must exist" >&2
  exit 1
fi
if [[ ! -x "$COMPAT19" ]]; then
  echo "error: check_compat_1_9.sh must be executable" >&2
  exit 1
fi
# NOTE ONLY — do not assert presence or absence of check_compat_1_10.sh (M4).
echo "note: check_compat_1_10.sh is Phase21-M4 (not asserted here)"
echo "F: version 1.10.0 / Cargo 1.10.0 / compat_1_9: OK"

echo
echo "demo_mount_path: ALL OK (assert_path=$ASSERT_PATH)"
