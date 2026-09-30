#!/usr/bin/env bash
# Phase 21 / 1.10+ compat gate (G4 / M4): runs the 1.9 gate, then asserts
# 1.11 additive mount path flags still appear in --help. Does **not** assert
# absolute throughput / SLA numbers and does **not** force-run long demos.
# Local only — no real internet. Keeps check_compat_1_0.sh … 1_9
# independently runnable.
# Usage: ./scripts/check_compat_1_10.sh
# Requires: cargo, python3 (demos via 1_0…1_9), bash.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Phase 21 / 1.10+ compat gate"
echo "==> building chunkforge-cli (shared binary for 1_9 + mount path assertions)"
cargo build -p chunkforge-cli --quiet
BIN="${CHUNKFORGE_BIN:-$ROOT/target/debug/chunkforge}"
if [[ ! -x "$BIN" ]]; then
  echo "error: binary not found: $BIN" >&2
  exit 1
fi
export CHUNKFORGE_BIN="$BIN"

echo
echo "==> running check_compat_1_9.sh (CHUNKFORGE_BIN=$CHUNKFORGE_BIN)"
bash "$ROOT/scripts/check_compat_1_9.sh"

echo
echo "==> 1.11 mount path flag assertions (help text)"

MOUNT_HELP="$("$BIN" mount --help)"

# --path / --path-from / --exclude / --exclude-from on mount (1.11 additive)
if ! grep -Eiq -- '(^|[[:space:]])--path([[:space:]=]|$)' <<<"$MOUNT_HELP"; then
  echo "error: mount --help missing --path" >&2
  exit 1
fi
echo "  mount: --path OK"

if ! grep -Fq -- '--path-from' <<<"$MOUNT_HELP"; then
  echo "error: mount --help missing --path-from" >&2
  exit 1
fi
echo "  mount: --path-from OK"

if ! grep -Eiq -- '(^|[[:space:]])--exclude([[:space:]=]|$)' <<<"$MOUNT_HELP"; then
  echo "error: mount --help missing --exclude" >&2
  exit 1
fi
echo "  mount: --exclude OK"

if ! grep -Fq -- '--exclude-from' <<<"$MOUNT_HELP"; then
  echo "error: mount --help missing --exclude-from" >&2
  exit 1
fi
echo "  mount: --exclude-from OK"

echo
echo "==> thin non-goal re-asserts (no gc --path / no extract --delete / no push --fallback / no write-mount / no pack claim / no abs perf SLA)"

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

# extract has no --delete / prune
EXTRACT_HELP="$("$BIN" extract --help)"
if grep -Eiq -- '(^|[[:space:]])--delete([[:space:]=]|$)' <<<"$EXTRACT_HELP"; then
  echo "error: extract --help advertises --delete (prune is forbidden)" >&2
  exit 1
fi
if grep -Eiq '(^|[[:space:]])prune([[:space:]]|$)' <<<"$EXTRACT_HELP"; then
  echo "error: extract --help advertises prune (forbidden)" >&2
  exit 1
fi
echo "  extract: no --delete / prune OK"

# push has no --fallback (write-side single dest)
PUSH_HELP="$("$BIN" push --help)"
if grep -Eiq -- '(^|[[:space:]])--fallback([[:space:]=]|$)' <<<"$PUSH_HELP"; then
  echo "error: push --help advertises --fallback (write-side multi-dest is forbidden)" >&2
  exit 1
fi
echo "  push: no --fallback OK"

# No pack subcommand / no pack "implemented" claim in top help
TOP_HELP="$("$BIN" --help)"
if grep -Eiq '(^|[[:space:]])pack([[:space:]]|$)' <<<"$TOP_HELP"; then
  echo "error: top-level --help advertises a pack command (pack is deferred)" >&2
  exit 1
fi
echo "  no pack subcommand OK"

# Write-mount thin check: mount stays RO; help must not advertise write/rw flags;
# help or docs should still carry read-only / not-write-mount narrative.
if grep -Eiq -- '(^|[[:space:]])--(write|writable|rw)([[:space:]=]|$)' <<<"$MOUNT_HELP"; then
  echo "error: mount --help advertises write/writable/--rw (write mount is forbidden)" >&2
  exit 1
fi
if ! grep -Eiq 'read-only|read only|not write-mount|≠ write' <<<"$MOUNT_HELP"; then
  echo "error: mount --help missing read-only / not-write-mount narrative" >&2
  exit 1
fi
if [[ -f "$ROOT/docs/mount.md" ]]; then
  # Thin doc check: must not claim writable mounts are delivered
  if grep -Eiq 'writable mounts? (are |is )?(now |fully )?(supported|implemented|available)|write-back (is |are )?(supported|implemented)' "$ROOT/docs/mount.md"; then
    # Allow negation lines ("not write-back", "Writable mounts … not")
    if ! grep -Eiq 'not (implemented|supported)|≠ write|is not write' "$ROOT/docs/mount.md"; then
      echo "error: docs/mount.md appears to claim writable/write-back mounts (forbidden)" >&2
      exit 1
    fi
  fi
  if ! grep -Eiq 'read-only|EROFS|≠ write mount|not write' "$ROOT/docs/mount.md"; then
    echo "error: docs/mount.md missing read-only / not-write-mount narrative" >&2
    exit 1
  fi
fi
echo "  mount: RO / no write-mount OK"

# Absolute perf is intentionally NOT asserted here (no wall_s / chunk_per_s SLA).
echo "  (skip absolute perf SLA — by design) OK"

echo
echo "==> demo_mount_path presence (G3/O4; do not force-run long demo)"
if [[ ! -f "$ROOT/scripts/demo_mount_path.sh" ]]; then
  echo "error: scripts/demo_mount_path.sh missing" >&2
  exit 1
fi
if [[ ! -x "$ROOT/scripts/demo_mount_path.sh" ]]; then
  echo "error: scripts/demo_mount_path.sh not executable" >&2
  exit 1
fi
echo "  demo_mount_path.sh present + executable OK"

echo
echo "OK: check_compat_1_10"
