#!/usr/bin/env bash
# Phase 23 / 1.12+ compat gate (G6 / M5 / V4): runs the 1.11 gate, then asserts
# 1.13 additive diff --symlinks flags still appear in --help (default skip
# ≡ 1.12; do **not** claim default record). Thin functional: archive
# --symlinks record → diff --tree --symlinks record identical exit 0 (tiny
# local tree). Does **not** assert absolute throughput / SLA numbers and
# does **not** force-run long demos.
# Local only — no real internet. Keeps check_compat_1_0.sh … 1_11
# independently runnable.
# Usage: ./scripts/check_compat_1_12.sh
# Requires: cargo, python3 (demos via 1_0…1_11), bash.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Phase 23 / 1.12+ compat gate"
echo "==> building chunkforge-cli (shared binary for 1_11 + diff --symlinks assertions)"
cargo build -p chunkforge-cli --quiet
BIN="${CHUNKFORGE_BIN:-$ROOT/target/debug/chunkforge}"
if [[ ! -x "$BIN" ]]; then
  echo "error: binary not found: $BIN" >&2
  exit 1
fi
export CHUNKFORGE_BIN="$BIN"

echo
echo "==> running check_compat_1_11.sh (CHUNKFORGE_BIN=$CHUNKFORGE_BIN)"
bash "$ROOT/scripts/check_compat_1_11.sh"

echo
echo "==> 1.13 diff --symlinks flag assertions (help text)"

DIFF_HELP="$("$BIN" diff --help)"

# --symlinks on diff (1.13 additive; primarily for --tree)
if ! grep -Eiq -- '(^|[[:space:]])--symlinks([[:space:]=]|$)' <<<"$DIFF_HELP"; then
  echo "error: diff --help missing --symlinks" >&2
  exit 1
fi
echo "  diff: --symlinks OK"

# Default skip narrative: help shows skip as default / ≡ 1.12
# clap prints "[default: skip]" and prose about default skip ≡ 1.12.
if ! grep -Eiq '\[default:[[:space:]]*skip\]' <<<"$DIFF_HELP"; then
  # Fallback: prose that default is skip ≡ 1.12
  if ! grep -Eiq 'default.*skip.*(≡|=).*1\.12|skip \(default|default.*skip\+warn' <<<"$DIFF_HELP"; then
    echo "error: diff --help missing default skip / ≡ 1.12 narrative" >&2
    exit 1
  fi
fi
echo "  diff: default skip ≡ 1.12 OK"

# Must NOT present record as the default
if grep -Eiq '\[default:[[:space:]]*record\]' <<<"$DIFF_HELP"; then
  echo "error: diff --help claims default record (forbidden; default must be skip)" >&2
  exit 1
fi
if grep -Eiq 'default[[:space:]]+(is[[:space:]]+)?record|record[[:space:]]+\(default' <<<"$DIFF_HELP"; then
  echo "error: diff --help claims record as default (forbidden)" >&2
  exit 1
fi
echo "  diff: no default-record claim OK"

echo
echo "==> thin functional: archive --symlinks record → diff --tree --symlinks record identical"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/cf-compat-1_12.XXXXXX")"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT

mkdir -p "$TMP/src/pkgs/foo" "$TMP/store"
echo 'hello-foo' > "$TMP/src/pkgs/foo/a.txt"
ln -s a.txt "$TMP/src/pkgs/foo/link.txt"

"$BIN" store create --store "$TMP/store"
"$BIN" archive --store "$TMP/store" -o "$TMP/tree.cfdir" \
  --symlinks record "$TMP/src" >/dev/null

set +e
"$BIN" diff --tree "$TMP/src" --symlinks record "$TMP/tree.cfdir" \
  >"$TMP/record.out" 2>"$TMP/record.err"
RC_REC=$?
set -e
if [[ "$RC_REC" -ne 0 ]]; then
  echo "error: diff --tree --symlinks record against identical tree must exit 0 (got $RC_REC)" >&2
  cat "$TMP/record.err" >&2 || true
  cat "$TMP/record.out" >&2 || true
  exit 1
fi
echo "  record → identical exit 0 OK"

# Optional thin: default diff --tree against that listing shows false-added or non-zero
# (documents skip ≡ 1.12)
set +e
"$BIN" diff --tree "$TMP/src" "$TMP/tree.cfdir" --format json \
  >"$TMP/skip.json" 2>"$TMP/skip.err"
RC_SKIP=$?
set -e
if [[ "$RC_SKIP" -eq 0 ]]; then
  echo "error: default diff --tree against record listing must be non-identical (skip ≡ 1.12)" >&2
  cat "$TMP/skip.json" >&2 || true
  exit 1
fi
# Prefer false-added containing link path when JSON is parseable; soft if not.
if command -v python3 >/dev/null 2>&1; then
  python3 -c "
import json, sys
obj = json.load(open('$TMP/skip.json'))
added = obj.get('added') or []
assert any('link' in p for p in added), ('expected false added for symlink paths', obj)
print('  default skip → false-added symlink path OK')
" || {
    echo "error: default skip JSON missing false-added symlink path" >&2
    cat "$TMP/skip.json" >&2 || true
    exit 1
  }
else
  echo "  default skip → non-zero exit OK (python3 unavailable; skip false-added parse)"
fi

echo
echo "==> thin non-goal re-asserts (no gc --path / no push --fallback / no write-mount / no extract --delete / no default record / no abs perf SLA)"

# gc has no --path / --path-from (hard ban; re-assert even if 1_11 already did)
GC_HELP="$("$BIN" gc --help)"
if grep -Eiq -- '(^|[[:space:]])--path([[:space:]=]|$)' <<<"$GC_HELP"; then
  echo "error: gc --help advertises --path (gc --path is hard-banned)" >&2
  exit 1
fi
if grep -Fq -- '--path-from' <<<"$GC_HELP"; then
  echo "error: gc --help advertises --path-from (forbidden)" >&2
  exit 1
fi
echo "  gc: no --path / --path-from OK"

# push has no --fallback (write-side single dest)
PUSH_HELP="$("$BIN" push --help)"
if grep -Eiq -- '(^|[[:space:]])--fallback([[:space:]=]|$)' <<<"$PUSH_HELP"; then
  echo "error: push --help advertises --fallback (write-side multi-dest is forbidden)" >&2
  exit 1
fi
echo "  push: no --fallback OK"

# Write-mount thin check: mount stays RO; help must not advertise write/rw flags
MOUNT_HELP="$("$BIN" mount --help)"
if grep -Eiq -- '(^|[[:space:]])--(write|writable|rw)([[:space:]=]|$)' <<<"$MOUNT_HELP"; then
  echo "error: mount --help advertises write/writable/--rw (write mount is forbidden)" >&2
  exit 1
fi
echo "  mount: no --write/--writable/--rw OK"

# extract has no --delete / prune (prior gates assert; re-assert)
EXTRACT_HELP="$("$BIN" extract --help)"
if grep -Eiq -- '(^|[[:space:]])--delete([[:space:]=]|$)' <<<"$EXTRACT_HELP"; then
  echo "error: extract --help advertises --delete (prune is forbidden)" >&2
  exit 1
fi
echo "  extract: no --delete / prune OK"

# Absolute perf is intentionally NOT asserted here (no wall_s / chunk_per_s SLA).
echo "  (skip absolute perf SLA — by design) OK"

echo
echo "==> demo_diff_tree_symlink presence (G5/O3; do not force-run long demo)"
if [[ ! -f "$ROOT/scripts/demo_diff_tree_symlink.sh" ]]; then
  echo "error: scripts/demo_diff_tree_symlink.sh missing" >&2
  exit 1
fi
if [[ ! -x "$ROOT/scripts/demo_diff_tree_symlink.sh" ]]; then
  echo "error: scripts/demo_diff_tree_symlink.sh not executable" >&2
  exit 1
fi
echo "  demo_diff_tree_symlink.sh present + executable OK"

echo
echo "OK: check_compat_1_12"
