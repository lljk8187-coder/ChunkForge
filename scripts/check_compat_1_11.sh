#!/usr/bin/env bash
# Phase 22 / 1.11+ compat gate (G6 / M6 / V4): runs the 1.10 gate, then asserts
# 1.12 additive archive --symlinks flags still appear in --help (default skip
# ≡ 1.11; do **not** claim default record). Does **not** assert absolute
# throughput / SLA numbers and does **not** force-run long demos.
# Local only — no real internet. Keeps check_compat_1_0.sh … 1_10
# independently runnable.
# Usage: ./scripts/check_compat_1_11.sh
# Requires: cargo, python3 (demos via 1_0…1_10), bash.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Phase 22 / 1.11+ compat gate"
echo "==> building chunkforge-cli (shared binary for 1_10 + symlink assertions)"
cargo build -p chunkforge-cli --quiet
BIN="${CHUNKFORGE_BIN:-$ROOT/target/debug/chunkforge}"
if [[ ! -x "$BIN" ]]; then
  echo "error: binary not found: $BIN" >&2
  exit 1
fi
export CHUNKFORGE_BIN="$BIN"

echo
echo "==> running check_compat_1_10.sh (CHUNKFORGE_BIN=$CHUNKFORGE_BIN)"
bash "$ROOT/scripts/check_compat_1_10.sh"

echo
echo "==> 1.12 archive --symlinks flag assertions (help text)"

ARCHIVE_HELP="$("$BIN" archive --help)"

# --symlinks on archive (1.12 additive)
if ! grep -Eiq -- '(^|[[:space:]])--symlinks([[:space:]=]|$)' <<<"$ARCHIVE_HELP"; then
  echo "error: archive --help missing --symlinks" >&2
  exit 1
fi
echo "  archive: --symlinks OK"

# Default skip narrative: help shows skip as default / ≡ 1.11
# clap prints "[default: skip]" and prose about default skip ≡ 1.11.
if ! grep -Eiq '\[default:[[:space:]]*skip\]' <<<"$ARCHIVE_HELP"; then
  # Fallback: prose that default is skip ≡ 1.11
  if ! grep -Eiq 'default.*skip.*(≡|=).*1\.11|skip \(default|default.*skip\+warn' <<<"$ARCHIVE_HELP"; then
    echo "error: archive --help missing default skip / ≡ 1.11 narrative" >&2
    exit 1
  fi
fi
echo "  archive: default skip ≡ 1.11 OK"

# Must NOT present record as the default
if grep -Eiq '\[default:[[:space:]]*record\]' <<<"$ARCHIVE_HELP"; then
  echo "error: archive --help claims default record (forbidden; default must be skip)" >&2
  exit 1
fi
if grep -Eiq 'default[[:space:]]+(is[[:space:]]+)?record|record[[:space:]]+\(default' <<<"$ARCHIVE_HELP"; then
  echo "error: archive --help claims record as default (forbidden)" >&2
  exit 1
fi
echo "  archive: no default-record claim OK"

echo
echo "==> thin non-goal re-asserts (no gc --path / no push --fallback / no write-mount / no default record / no abs perf SLA)"

# gc has no --path / --path-from (hard ban; re-assert even if 1_10 already did)
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

# Absolute perf is intentionally NOT asserted here (no wall_s / chunk_per_s SLA).
echo "  (skip absolute perf SLA — by design) OK"

echo
echo "==> demo_symlink presence (G5/O3; do not force-run long demo)"
if [[ ! -f "$ROOT/scripts/demo_symlink.sh" ]]; then
  echo "error: scripts/demo_symlink.sh missing" >&2
  exit 1
fi
if [[ ! -x "$ROOT/scripts/demo_symlink.sh" ]]; then
  echo "error: scripts/demo_symlink.sh not executable" >&2
  exit 1
fi
echo "  demo_symlink.sh present + executable OK"

echo
echo "OK: check_compat_1_11"
