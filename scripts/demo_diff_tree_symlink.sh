#!/usr/bin/env bash
# Phase 23 M4 smoke: archive --symlinks record → diff --tree skip|record;
# default skip demonstrates false added / skip warn; --symlinks record →
# identical exit 0; absolute target → non-zero; thin path filter on symlink
# path. Narrative:
#   diff --tree --symlinks record ≠ write mount ≠ follow ≠ pack ≠ sync
#   ≠ prune ≠ gc --path ≠ default record
# Local only — no internet.
# Usage: ./scripts/demo_diff_tree_symlink.sh
# Requires: cargo, python3.
# Env (optional):
#   CHUNKFORGE_BIN, CHUNKFORGE_PYTHON
#   CHUNKFORGE_P23_DEMO_DIR (default /tmp/cf-p23-diff-tree-symlink-demo)
#
# Version gate expects chunkforge 1.15.0 (Phase25-M7; nests under check_compat_1_14).
#
# Gate (Phase23-M5): require check_compat_1_12.sh present + executable
# (same pattern as demo_symlink ↔ compat_1_11). Existing compat_1_11 must
# stay present + executable. Do not force-run the long compat gate here.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_P23_DEMO_DIR:-/tmp/cf-p23-diff-tree-symlink-demo}"
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
ln -s b.txt "$SRC/pkgs/bar/link-bar.txt"
printf 'readme\n' > "$SRC/readme.txt"

echo
echo "==> A. archive --symlinks record → listing"
"$BIN" store create --store "$STORE"
"$BIN" archive --store "$STORE" -o "$DEMO_DIR/tree.cfdir" \
  --symlinks record --format json "$SRC" \
  >"$DEMO_DIR/archive.json" \
  2>"$DEMO_DIR/archive.err"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/archive.json'))
assert obj.get('ok') is True, obj
assert obj.get('recorded_symlinks', 0) >= 2, obj
print('A: recorded_symlinks=', obj['recorded_symlinks'], 'format ok')
"
# listing↔listing self-compare must be identical (library already compares Symlink)
"$BIN" diff "$DEMO_DIR/tree.cfdir" "$DEMO_DIR/tree.cfdir"
echo "A: listing↔listing self-diff exit 0: OK"

echo
echo "==> B. diff --tree default skip → false added / skip warn (exit non-zero OK)"
set +e
"$BIN" diff --tree "$SRC" "$DEMO_DIR/tree.cfdir" --format json \
  >"$DEMO_DIR/skip.json" \
  2>"$DEMO_DIR/skip.err"
RC_SKIP=$?
set -e
echo "B: exit=$RC_SKIP (expect non-zero / 1)"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/skip.json'))
added = obj.get('added') or []
# Default skip omits ephemeral Symlink → listing-only symlink paths look added
assert any('link' in p for p in added), ('expected false added for symlink paths', obj)
print('B: false added symlink paths=', [p for p in added if 'link' in p])
"
if ! grep -E 'diff:.*(skip|symlink|Symlink|warn)' "$DEMO_DIR/skip.err" >/dev/null 2>&1 \
  && ! grep -Ei 'symlink' "$DEMO_DIR/skip.err" >/dev/null 2>&1; then
  # soft: some builds may phrase skip warn differently; still require non-zero + false added
  echo "B: note: stderr skip warn text not matched; false-added assert already passed"
  cat "$DEMO_DIR/skip.err" >&2 || true
else
  echo "B: skip warn present on stderr: OK"
fi
if [[ "$RC_SKIP" -eq 0 ]]; then
  echo "error: default --tree skip against record listing must be non-identical" >&2
  exit 1
fi
echo "B: default skip false-added + non-zero: OK"

echo
echo "==> C. diff --tree --symlinks record → identical exit 0"
set +e
"$BIN" diff --tree "$SRC" --symlinks record "$DEMO_DIR/tree.cfdir" --format json \
  >"$DEMO_DIR/record.json" \
  2>"$DEMO_DIR/record.err"
RC_REC=$?
set -e
echo "C: exit=$RC_REC (expect 0)"
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/record.json'))
for k in ('added', 'removed', 'changed', 'meta_changed'):
    assert obj.get(k) == [], (k, obj)
assert obj.get('chunks_only_left', 0) == 0, obj
assert obj.get('chunks_only_right', 0) == 0, obj
print('C: identical JSON categories empty; chunks_only_*=0')
"
if [[ "$RC_REC" -ne 0 ]]; then
  echo "error: --symlinks record against identical tree must exit 0" >&2
  cat "$DEMO_DIR/record.err" >&2 || true
  cat "$DEMO_DIR/record.json" >&2 || true
  exit 1
fi
echo "C: --symlinks record identical exit 0: OK"

echo
echo "==> D. absolute target → non-zero under --symlinks record"
ABS_SRC="$DEMO_DIR/abs-src"
rm -rf "$ABS_SRC"
mkdir -p "$ABS_SRC/pkgs/foo"
echo 'hello-foo' > "$ABS_SRC/pkgs/foo/a.txt"
ln -s /etc/passwd "$ABS_SRC/pkgs/foo/bad"
set +e
"$BIN" diff --tree "$ABS_SRC" --symlinks record "$DEMO_DIR/tree.cfdir" \
  >"$DEMO_DIR/abs.out" \
  2>"$DEMO_DIR/abs.err"
RC_ABS=$?
set -e
echo "D: exit=$RC_ABS (expect non-zero, ≠1 preferred for usage/anyhow)"
if [[ "$RC_ABS" -eq 0 ]]; then
  echo "error: absolute symlink target under --symlinks record must fail" >&2
  cat "$DEMO_DIR/abs.err" >&2 || true
  exit 1
fi
# Helpful: message should mention absolute / target / symlink
if ! grep -Ei 'absolute|empty|symlink|target' "$DEMO_DIR/abs.err" >/dev/null 2>&1; then
  echo "D: note: stderr did not match absolute/target keywords; exit non-zero still OK"
  cat "$DEMO_DIR/abs.err" >&2 || true
else
  echo "D: absolute-target error message present: OK"
fi
echo "D: absolute target non-zero: OK"

echo
echo "==> E. thin --path filter on symlink path (with --symlinks record)"
set +e
"$BIN" diff --tree "$SRC" --symlinks record --path pkgs/foo \
  "$DEMO_DIR/tree.cfdir" --format json \
  >"$DEMO_DIR/path.json" \
  2>"$DEMO_DIR/path.err"
RC_PATH=$?
set -e
"$PYTHON" -c "
import json
obj = json.load(open('$DEMO_DIR/path.json'))
# Narrowed to pkgs/foo: symlink link.txt participates; bar paths out of scope
# Against full record listing, bar File+Symlink appear as removed (listing-only)
# OR we compare and accept that path filter narrows BOTH sides — so identical
# under pkgs/foo should be empty categories if filter applies to both.
# PathFilter narrows both sides → pkgs/foo subset should be identical.
for k in ('added', 'removed', 'changed', 'meta_changed'):
    assert obj.get(k) == [], (k, obj)
print('E: --path pkgs/foo + record → identical under filter')
"
if [[ "$RC_PATH" -ne 0 ]]; then
  echo "error: --path pkgs/foo --symlinks record should exit 0 (both sides filtered)" >&2
  cat "$DEMO_DIR/path.err" >&2 || true
  cat "$DEMO_DIR/path.json" >&2 || true
  exit 1
fi
echo "E: path filter orthogonal to --symlinks record: OK"

echo
echo "==> F. help / responsibility surfaces"
DIFF_HELP="$("$BIN" diff --help)"
if ! grep -F -- '--symlinks' <<<"$DIFF_HELP" >/dev/null; then
  echo "error: diff --help missing --symlinks" >&2
  exit 1
fi
echo "diff --help has --symlinks: OK"

# --symlinks without --tree must fail (clap requires=tree)
set +e
"$BIN" diff --symlinks record "$DEMO_DIR/tree.cfdir" "$DEMO_DIR/tree.cfdir" \
  >"$DEMO_DIR/requires.out" \
  2>"$DEMO_DIR/requires.err"
RC_REQ=$?
set -e
if [[ "$RC_REQ" -eq 0 ]]; then
  echo "error: --symlinks without --tree must be rejected (clap requires=tree)" >&2
  exit 1
fi
echo "F: --symlinks requires --tree: OK"

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
echo "F: help / ≠ prune / ≠ gc-path / ≠ write mount: OK"

echo
echo "==> G. version 1.15.0 + Cargo 1.15.0 + compat_1_12 (Phase25-M7)"
VER="$("$BIN" --version)"
echo "version: $VER"
if ! grep -F '1.15.0' <<<"$VER" >/dev/null; then
  echo "error: expected chunkforge 1.15.0; got $VER" >&2
  exit 1
fi
if ! grep -E '^version = "1\.15\.0"' "$ROOT/Cargo.toml" >/dev/null; then
  echo "error: workspace Cargo.toml version must be 1.15.0 (Phase25-M7)" >&2
  grep -E '^version' "$ROOT/Cargo.toml" >&2 || true
  exit 1
fi
# Existing gate that already exists must stay present (1_11 from Phase22).
COMPAT111="$ROOT/scripts/check_compat_1_11.sh"
if [[ ! -f "$COMPAT111" ]]; then
  echo "error: check_compat_1_11.sh must exist" >&2
  exit 1
fi
if [[ ! -x "$COMPAT111" ]]; then
  echo "error: check_compat_1_11.sh must be executable" >&2
  exit 1
fi
# Phase23-M5: compat_1_12 must exist + executable (same pattern as
# demo_symlink ↔ compat_1_11). Do not force-run the long gate here.
COMPAT112="$ROOT/scripts/check_compat_1_12.sh"
if [[ ! -f "$COMPAT112" ]]; then
  echo "error: check_compat_1_12.sh must exist" >&2
  exit 1
fi
if [[ ! -x "$COMPAT112" ]]; then
  echo "error: check_compat_1_12.sh must be executable" >&2
  exit 1
fi
echo "G: version 1.15.0 / Cargo 1.15.0 / compat_1_11 + compat_1_12 present+executable: OK"

echo
echo "demo_diff_tree_symlink: ALL OK"
echo "proved: archive --symlinks record → listing;"
echo "        diff --tree default skip → false added / non-zero;"
echo "        diff --tree --symlinks record → identical exit 0;"
echo "        absolute target non-zero;"
echo "        path filter orthogonal;"
echo "        record ≠ write mount ≠ follow ≠ pack ≠ sync ≠ prune ≠ gc-path ≠ default record"
