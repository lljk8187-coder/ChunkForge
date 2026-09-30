#!/usr/bin/env bash
# Phase 26 / 1.15+ compat gate (G6 / M5 / V4): runs the 1.14 gate, then asserts
# 1.16 additive surfaces still appear: archive --empty-dirs, store get, and
# leaf-Dir path fidelity (ls/filter --path keep empty leaf Dir).
# Thin functional: archive --empty-dirs → ls --path <empty_leaf> non-empty
# `dir\t…`; filter --path <empty_leaf> keeps Dir; optional thin store get
# bytes match (tiny local tree). Does **not** assert absolute throughput /
# SLA numbers and does **not** force-run long demos.
# Local only — no real internet. Keeps check_compat_1_0.sh … 1_14
# independently runnable.
# Usage: ./scripts/check_compat_1_15.sh
# Requires: cargo, python3 (demos via 1_0…1_14), bash.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Phase 26 / 1.15+ compat gate"
echo "==> building chunkforge-cli (shared binary for 1_14 + empty-dirs/store get assertions)"
cargo build -p chunkforge-cli --quiet
BIN="${CHUNKFORGE_BIN:-$ROOT/target/debug/chunkforge}"
if [[ ! -x "$BIN" ]]; then
  echo "error: binary not found: $BIN" >&2
  exit 1
fi
export CHUNKFORGE_BIN="$BIN"

echo
echo "==> running check_compat_1_14.sh (CHUNKFORGE_BIN=$CHUNKFORGE_BIN)"
bash "$ROOT/scripts/check_compat_1_14.sh"

echo
echo "==> 1.16 empty-dirs + store get assertions (help text)"

ARCH_HELP="$("$BIN" archive --help)"
if ! grep -Fq -- '--empty-dirs' <<<"$ARCH_HELP"; then
  echo "error: archive --help missing --empty-dirs" >&2
  echo "$ARCH_HELP" >&2
  exit 1
fi
echo "  archive: --help exposes --empty-dirs OK"

# store get must exist as a store subcommand
set +e
GET_HELP="$("$BIN" store get --help 2>&1)"
RC_GET_HELP=$?
set -e
if [[ "$RC_GET_HELP" -ne 0 ]]; then
  echo "error: store get --help must exit 0 (got $RC_GET_HELP); store get missing?" >&2
  echo "$GET_HELP" >&2
  exit 1
fi
if ! grep -Eiq 'store get|Fetch one chunk|plaintext' <<<"$GET_HELP"; then
  echo "error: store get --help missing expected narrative" >&2
  echo "$GET_HELP" >&2
  exit 1
fi
# Help nails ≠ scrub ≠ cat ≠ extract (and friends if present)
for needle in scrub cat extract; do
  if ! grep -F "$needle" <<<"$GET_HELP" >/dev/null; then
    echo "error: store get --help must nail ≠ $needle" >&2
    echo "$GET_HELP" >&2
    exit 1
  fi
done
for flag in --store --verify --format; do
  if ! grep -Fq -- "$flag" <<<"$GET_HELP"; then
    echo "error: store get --help missing $flag" >&2
    echo "$GET_HELP" >&2
    exit 1
  fi
done
if ! grep -E -- '(^|[[:space:]])-o,' <<<"$GET_HELP" >/dev/null && \
   ! grep -Fq -- '--output' <<<"$GET_HELP"; then
  echo "error: store get --help missing -o/--output" >&2
  echo "$GET_HELP" >&2
  exit 1
fi
echo "  store get: --help exists + ≠ scrub ≠ cat ≠ extract + flags OK"

STORE_HELP="$("$BIN" store --help)"
if ! grep -Eiq '(^|[[:space:]])get([[:space:]]|$)' <<<"$STORE_HELP"; then
  echo "error: store --help missing get subcommand" >&2
  echo "$STORE_HELP" >&2
  exit 1
fi
echo "  store: get subcommand listed OK"

echo
echo "==> thin functional: archive --empty-dirs → ls --path empty_leaf + filter keeps Dir (+ optional store get)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/cf-compat-1_15.XXXXXX")"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT

mkdir -p "$TMP/src/empty_leaf" "$TMP/src/other_empty" "$TMP/store"
# Tiny payload so FastCDC yields one chunk (store get bytes ≡ source).
printf 'compat-1_15-payload\n' > "$TMP/src/payload.txt"

"$BIN" store create --store "$TMP/store"
"$BIN" archive --store "$TMP/store" --empty-dirs -o "$TMP/full.cfdir" \
  "$TMP/src" >/dev/null

"$BIN" ls --path empty_leaf "$TMP/full.cfdir" >"$TMP/ls-path.out" 2>"$TMP/ls-path.err"
if [[ ! -s "$TMP/ls-path.out" ]]; then
  echo "error: ls --path empty_leaf produced empty listing (leaf Dir dropped)" >&2
  cat "$TMP/ls-path.err" >&2 || true
  exit 1
fi
if ! grep -F $'dir\tempty_leaf' "$TMP/ls-path.out" >/dev/null; then
  echo "error: ls --path empty_leaf missing dir\\tempty_leaf row" >&2
  cat "$TMP/ls-path.out" >&2 || true
  exit 1
fi
if grep -E 'other_empty|payload\.txt' "$TMP/ls-path.out" >/dev/null; then
  echo "error: ls --path empty_leaf must not list unrelated paths" >&2
  cat "$TMP/ls-path.out" >&2 || true
  exit 1
fi
echo "  ls --path empty_leaf → non-empty dir\\t… OK"

"$BIN" filter --path empty_leaf -o "$TMP/empty.cfdir" "$TMP/full.cfdir" >/dev/null
"$BIN" ls "$TMP/empty.cfdir" >"$TMP/ls-filt.out" 2>"$TMP/ls-filt.err"
if [[ ! -s "$TMP/ls-filt.out" ]]; then
  echo "error: filter --path empty_leaf → ls empty (leaf Dir dropped)" >&2
  cat "$TMP/ls-filt.err" >&2 || true
  exit 1
fi
if ! grep -F $'dir\tempty_leaf' "$TMP/ls-filt.out" >/dev/null; then
  echo "error: filtered ls missing kept Dir empty_leaf" >&2
  cat "$TMP/ls-filt.out" >&2 || true
  exit 1
fi
if grep -E 'other_empty|payload\.txt' "$TMP/ls-filt.out" >/dev/null; then
  echo "error: filtered listing kept unrelated paths" >&2
  cat "$TMP/ls-filt.out" >&2 || true
  exit 1
fi
echo "  filter --path empty_leaf keeps Dir OK"

# Optional thin: store get write bytes match
ID="$("$BIN" store list --store "$TMP/store" | head -1 | tr -d '[:space:]')"
if [[ -z "$ID" ]]; then
  echo "error: store list returned no chunk id (archive should have written payload)" >&2
  exit 1
fi
set +e
"$BIN" store get --store "$TMP/store" "$ID" -o "$TMP/got.bin" \
  >"$TMP/get.out" 2>"$TMP/get.err"
RC_GET=$?
set -e
if [[ "$RC_GET" -ne 0 ]]; then
  echo "error: store get must exit 0 (got $RC_GET)" >&2
  cat "$TMP/get.err" >&2 || true
  cat "$TMP/get.out" >&2 || true
  exit 1
fi
if ! cmp -s "$TMP/src/payload.txt" "$TMP/got.bin"; then
  echo "error: store get -o bytes must ≡ source payload.txt" >&2
  exit 1
fi
echo "  store get -o bytes ≡ source OK"

echo
echo "==> thin non-goal re-asserts (no extract --delete / no pack / no LRU/store trim / no aws-sdk / no default zstd / no store recompress / no push --fallback / no gc --path / no write-mount / no default record symlink / no abs perf SLA)"

TOP_HELP="$("$BIN" --help)"

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
echo "==> demo_empty_dir_path_store_get presence (G5/O2; do not force-run long demo)"
if [[ ! -f "$ROOT/scripts/demo_empty_dir_path_store_get.sh" ]]; then
  echo "error: scripts/demo_empty_dir_path_store_get.sh missing" >&2
  exit 1
fi
if [[ ! -x "$ROOT/scripts/demo_empty_dir_path_store_get.sh" ]]; then
  echo "error: scripts/demo_empty_dir_path_store_get.sh not executable" >&2
  exit 1
fi
echo "  demo_empty_dir_path_store_get.sh present + executable OK"

echo
echo "OK: check_compat_1_15"
