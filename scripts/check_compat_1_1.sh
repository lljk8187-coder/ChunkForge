#!/usr/bin/env bash
# Phase 12 / 1.1+ compat gate (G5 / M5): runs the 1.0 gate, then asserts
# 1.1 / 1.2 additive flags still appear in --help. Does **not** assert
# absolute throughput / SLA numbers and does **not** run bench_loose_http.sh.
# Local only — no real internet. Keeps check_compat_1_0.sh independently runnable.
# Usage: ./scripts/check_compat_1_1.sh
# Requires: cargo, python3 (demos via 1_0), bash.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Phase 12 / 1.1+ compat gate"
echo "==> building chunkforge-cli (shared binary for 1_0 + 1.1 assertions)"
cargo build -p chunkforge-cli --quiet
BIN="${CHUNKFORGE_BIN:-$ROOT/target/debug/chunkforge}"
if [[ ! -x "$BIN" ]]; then
  echo "error: binary not found: $BIN" >&2
  exit 1
fi
export CHUNKFORGE_BIN="$BIN"

echo
echo "==> running check_compat_1_0.sh (CHUNKFORGE_BIN=$CHUNKFORGE_BIN)"
bash "$ROOT/scripts/check_compat_1_0.sh"

echo
echo "==> 1.1 / 1.2 flag assertions (help text)"

EXTRACT_HELP="$("$BIN" extract --help)"
for flag in --skip-trust-mtime --format; do
  if ! echo "$EXTRACT_HELP" | grep -Fq -- "$flag"; then
    echo "error: extract --help missing $flag" >&2
    exit 1
  fi
done
echo "  extract: --skip-trust-mtime / --format OK"

PUSH_HELP="$("$BIN" push --help)"
PULL_HELP="$("$BIN" pull --help)"
if ! echo "$PUSH_HELP" | grep -Fq -- '--format'; then
  echo "error: push --help missing --format" >&2
  exit 1
fi
if ! echo "$PULL_HELP" | grep -Fq -- '--format'; then
  echo "error: pull --help missing --format" >&2
  exit 1
fi
echo "  push/pull: --format OK"

MOUNT_HELP="$("$BIN" mount --help)"
if ! echo "$MOUNT_HELP" | grep -Fq -- '--prefetch-chunks'; then
  echo "error: mount --help missing --prefetch-chunks" >&2
  exit 1
fi
echo "  mount: --prefetch-chunks OK"

GC_HELP="$("$BIN" gc --help)"
for flag in --jobs --format; do
  if ! echo "$GC_HELP" | grep -Fq -- "$flag"; then
    echo "error: gc --help missing $flag" >&2
    exit 1
  fi
done
echo "  gc: --jobs / --format OK"

SCRUB_HELP="$("$BIN" store scrub --help)"
if ! echo "$SCRUB_HELP" | grep -Fq -- '--format'; then
  echo "error: store scrub --help missing --format" >&2
  exit 1
fi
echo "  store scrub: --format OK"

echo
echo "==> thin non-goal asserts (no pack subcommand; aws-sdk covered by 1_0)"
# Help must not advertise a first-class `pack` subcommand / fake pack flags.
# (docs/perf.md may mention pack as deferred — that is fine; CLI must not.)
TOP_HELP="$("$BIN" --help)"
if echo "$TOP_HELP" | grep -Eiq '(^|[[:space:]])pack([[:space:]]|$)'; then
  echo "error: top-level --help advertises a pack command (pack is deferred)" >&2
  exit 1
fi
# No --pack / --packfile style flags on push/pull/gc (false promises).
for cmd_help in "$PUSH_HELP" "$PULL_HELP" "$GC_HELP"; do
  if echo "$cmd_help" | grep -Eiq -- '--pack(file)?([[:space:]=]|$)'; then
    echo "error: CLI help advertises --pack / --packfile (pack is deferred)" >&2
    exit 1
  fi
done
echo "  no pack subcommand / --pack* flags OK"

echo
echo "OK: check_compat_1_1"
