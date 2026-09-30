#!/usr/bin/env bash
# Phase 25 / 1.14+ compat gate (G6 / M5 / V4): runs the 1.13 gate, then asserts
# 1.15 additive `ls` subcommand + `cat --path` (`.cfdir`) still appear in
# top-level / ls --help / cat --help.
# Thin functional: full.cfdir → filter --path … -o sub.cfdir → ls shows
# retained path; cat --path green (tiny local tree). Does **not** assert
# absolute throughput / SLA numbers and does **not** force-run long demos.
# Local only — no real internet. Keeps check_compat_1_0.sh … 1_13
# independently runnable.
# Usage: ./scripts/check_compat_1_14.sh
# Requires: cargo, python3 (demos via 1_0…1_13), bash.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Phase 25 / 1.14+ compat gate"
echo "==> building chunkforge-cli (shared binary for 1_13 + ls/cat --path assertions)"
cargo build -p chunkforge-cli --quiet
BIN="${CHUNKFORGE_BIN:-$ROOT/target/debug/chunkforge}"
if [[ ! -x "$BIN" ]]; then
  echo "error: binary not found: $BIN" >&2
  exit 1
fi
export CHUNKFORGE_BIN="$BIN"

echo
echo "==> running check_compat_1_13.sh (CHUNKFORGE_BIN=$CHUNKFORGE_BIN)"
bash "$ROOT/scripts/check_compat_1_13.sh"

echo
echo "==> 1.15 ls + cat --path assertions (help text)"

TOP_HELP="$("$BIN" --help)"
if ! grep -Eiq '(^|[[:space:]])ls([[:space:]]|$)' <<<"$TOP_HELP"; then
  echo "error: top-level --help missing ls subcommand" >&2
  echo "$TOP_HELP" >&2
  exit 1
fi
echo "  top-level: ls OK"

LS_HELP="$("$BIN" ls --help)"
if ! grep -Eiq 'List paths|inventory|\.cfdir|\.cfidx' <<<"$LS_HELP"; then
  echo "error: ls --help missing expected ls narrative" >&2
  echo "$LS_HELP" >&2
  exit 1
fi
# path 四件套 + --format + --chunks
for flag in --path --path-from --exclude --exclude-from --format --chunks; do
  if ! grep -Fq -- "$flag" <<<"$LS_HELP"; then
    echo "error: ls --help missing $flag" >&2
    echo "$LS_HELP" >&2
    exit 1
  fi
done
echo "  ls: --help + path 四件套 + --format/--chunks OK"

CAT_HELP="$("$BIN" cat --help)"
if ! grep -Fq -- '--path' <<<"$CAT_HELP"; then
  echo "error: cat --help missing --path (required for .cfdir single File)" >&2
  echo "$CAT_HELP" >&2
  exit 1
fi
if ! grep -Eiq '\.cfdir' <<<"$CAT_HELP"; then
  echo "error: cat --help missing .cfdir narrative for --path" >&2
  echo "$CAT_HELP" >&2
  exit 1
fi
echo "  cat: --help exposes --path for .cfdir OK"

echo
echo "==> thin functional: archive --symlinks record → filter --path → ls path + cat --path green"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/cf-compat-1_14.XXXXXX")"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT

mkdir -p "$TMP/src/pkgs/foo" "$TMP/src/pkgs/bar" "$TMP/store"
echo 'hello-foo' > "$TMP/src/pkgs/foo/a.txt"
ln -s a.txt "$TMP/src/pkgs/foo/link.txt"
echo 'hello-bar' > "$TMP/src/pkgs/bar/b.txt"

"$BIN" store create --store "$TMP/store"
"$BIN" archive --store "$TMP/store" -o "$TMP/full.cfdir" \
  --symlinks record "$TMP/src" >/dev/null

"$BIN" filter --path pkgs/foo -o "$TMP/sub.cfdir" "$TMP/full.cfdir" >/dev/null

"$BIN" ls "$TMP/sub.cfdir" >"$TMP/ls.out" 2>"$TMP/ls.err"
if ! grep -F $'file\tpkgs/foo/a.txt\t' "$TMP/ls.out" >/dev/null; then
  echo "error: ls subset missing retained File pkgs/foo/a.txt" >&2
  cat "$TMP/ls.out" >&2 || true
  cat "$TMP/ls.err" >&2 || true
  exit 1
fi
if ! grep -F $'symlink\tpkgs/foo/link.txt\t' "$TMP/ls.out" >/dev/null; then
  echo "error: ls subset missing retained Symlink pkgs/foo/link.txt" >&2
  cat "$TMP/ls.out" >&2 || true
  exit 1
fi
if grep -E 'pkgs/bar' "$TMP/ls.out" >/dev/null; then
  echo "error: ls subset must not list filtered-out pkgs/bar" >&2
  cat "$TMP/ls.out" >&2 || true
  exit 1
fi
echo "  filter --path → ls retained path OK"

set +e
"$BIN" cat --store "$TMP/store" --path pkgs/foo/a.txt \
  -o "$TMP/a.bin" "$TMP/sub.cfdir" \
  >"$TMP/cat.out" 2>"$TMP/cat.err"
RC_CAT=$?
set -e
if [[ "$RC_CAT" -ne 0 ]]; then
  echo "error: cat --path … sub.cfdir must exit 0 (got $RC_CAT)" >&2
  cat "$TMP/cat.err" >&2 || true
  cat "$TMP/cat.out" >&2 || true
  exit 1
fi
if ! cmp -s "$TMP/src/pkgs/foo/a.txt" "$TMP/a.bin"; then
  echo "error: cat --path bytes must ≡ source pkgs/foo/a.txt" >&2
  exit 1
fi
echo "  cat --path green (bytes ≡ source) OK"

echo
echo "==> thin non-goal re-asserts (no extract --delete / no pack / no LRU/store trim / no aws-sdk / no default zstd / no store recompress / no push --fallback / no gc --path / no write-mount / no default record symlink / no abs perf SLA)"

# extract has no --delete / prune
EXTRACT_HELP="$("$BIN" extract --help)"
if grep -Eiq -- '(^|[[:space:]])--delete([[:space:]=]|$)' <<<"$EXTRACT_HELP"; then
  echo "error: extract --help advertises --delete (prune is forbidden)" >&2
  exit 1
fi
echo "  extract: no --delete / prune OK"

# No pack subcommand
if grep -Eiq '(^|[[:space:]])pack([[:space:]]|$)' <<<"$TOP_HELP"; then
  echo "error: top-level --help advertises a pack command (pack is deferred)" >&2
  exit 1
fi
echo "  no pack subcommand OK"

# No LRU / store trim
for cmd in cat verify extract mount; do
  CMD_HELP="$("$BIN" "$cmd" --help)"
  if grep -Eiq -- '--cache-lru([[:space:]=]|$)' <<<"$CMD_HELP"; then
    echo "error: $cmd --help advertises --cache-lru (LRU is forbidden)" >&2
    exit 1
  fi
done
STORE_HELP="$("$BIN" store --help)"
if grep -Eiq '(^|[[:space:]])trim([[:space:]]|$)' <<<"$STORE_HELP"; then
  echo "error: store --help advertises trim (store trim is forbidden)" >&2
  exit 1
fi
echo "  no --cache-lru / store trim OK"

# No aws-sdk
if [[ -f "$ROOT/Cargo.lock" ]] && grep -Eiq '^name = "aws-sdk' "$ROOT/Cargo.lock"; then
  echo "error: Cargo.lock lists aws-sdk (forbidden)" >&2
  exit 1
fi
echo "  Cargo.lock: no aws-sdk OK"

# No default zstd
MAKE_HELP="$("$BIN" make --help)"
CREATE_HELP="$("$BIN" store create --help)"
if grep -Eiq 'default[[:space:]]+zstd|defaults?[[:space:]]+to[[:space:]]+zstd' <<<"$MAKE_HELP"; then
  echo "error: make --help advertises default zstd (forbidden; create default is none)" >&2
  exit 1
fi
if grep -Eiq 'default[[:space:]]+zstd|defaults?[[:space:]]+to[[:space:]]+zstd' <<<"$CREATE_HELP"; then
  echo "error: store create --help advertises default zstd (forbidden)" >&2
  exit 1
fi
echo "  make/store create: no default zstd OK"

# No store recompress
if grep -Eiq '(^|[[:space:]])recompress([[:space:]]|$)' <<<"$STORE_HELP"; then
  echo "error: store --help advertises recompress (forbidden)" >&2
  exit 1
fi
echo "  store: no recompress OK"

# push has no --fallback
PUSH_HELP="$("$BIN" push --help)"
if grep -Eiq -- '(^|[[:space:]])--fallback([[:space:]=]|$)' <<<"$PUSH_HELP"; then
  echo "error: push --help advertises --fallback (write-side multi-dest is forbidden)" >&2
  exit 1
fi
echo "  push: no --fallback OK"

# gc has no --path / --path-from (hard ban)
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

# Write-mount thin check: mount stays RO
MOUNT_HELP="$("$BIN" mount --help)"
if grep -Eiq -- '(^|[[:space:]])--(write|writable|rw)([[:space:]=]|$)' <<<"$MOUNT_HELP"; then
  echo "error: mount --help advertises write/writable/--rw (write mount is forbidden)" >&2
  exit 1
fi
echo "  mount: no --write/--writable/--rw OK"

# Default record symlink forbidden: archive + diff still default skip
ARCH_HELP="$("$BIN" archive --help)"
if grep -Eiq '\[default:[[:space:]]*record\]' <<<"$ARCH_HELP"; then
  echo "error: archive --help claims default record (forbidden; default must be skip)" >&2
  exit 1
fi
if ! grep -Eiq '\[default:[[:space:]]*skip\]' <<<"$ARCH_HELP"; then
  if ! grep -Eiq 'default.*skip|skip \(default' <<<"$ARCH_HELP"; then
    echo "error: archive --help missing default skip narrative" >&2
    exit 1
  fi
fi
echo "  archive: default skip (no default record) OK"

DIFF_HELP="$("$BIN" diff --help)"
if grep -Eiq '\[default:[[:space:]]*record\]' <<<"$DIFF_HELP"; then
  echo "error: diff --help claims default record (forbidden)" >&2
  exit 1
fi
if ! grep -Eiq '\[default:[[:space:]]*skip\]' <<<"$DIFF_HELP"; then
  if ! grep -Eiq 'default.*skip|skip \(default' <<<"$DIFF_HELP"; then
    echo "error: diff --help missing default skip narrative" >&2
    exit 1
  fi
fi
echo "  diff: default skip (no default record) OK"

# Absolute perf is intentionally NOT asserted here (no wall_s / chunk_per_s SLA).
echo "  (skip absolute perf SLA — by design) OK"

echo
echo "==> demo_ls_cat_path presence (G5/O3; do not force-run long demo)"
if [[ ! -f "$ROOT/scripts/demo_ls_cat_path.sh" ]]; then
  echo "error: scripts/demo_ls_cat_path.sh missing" >&2
  exit 1
fi
if [[ ! -x "$ROOT/scripts/demo_ls_cat_path.sh" ]]; then
  echo "error: scripts/demo_ls_cat_path.sh not executable" >&2
  exit 1
fi
echo "  demo_ls_cat_path.sh present + executable OK"

echo
echo "OK: check_compat_1_14"
