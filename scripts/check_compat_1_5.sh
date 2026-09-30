#!/usr/bin/env bash
# Phase 16 / 1.5+ compat gate (G6 / M6): runs the 1.4 gate, then asserts
# 1.6 additive flags still appear in --help. Does **not** assert
# absolute throughput / SLA numbers and does **not** run bench_loose_http.sh.
# Local only — no real internet. Keeps check_compat_1_0.sh … 1_4
# independently runnable.
# Usage: ./scripts/check_compat_1_5.sh
# Requires: cargo, python3 (demos via 1_0…1_4), bash.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Phase 16 / 1.5+ compat gate"
echo "==> building chunkforge-cli (shared binary for 1_4 + 1.6 assertions)"
cargo build -p chunkforge-cli --quiet
BIN="${CHUNKFORGE_BIN:-$ROOT/target/debug/chunkforge}"
if [[ ! -x "$BIN" ]]; then
  echo "error: binary not found: $BIN" >&2
  exit 1
fi
export CHUNKFORGE_BIN="$BIN"

echo
echo "==> running check_compat_1_4.sh (CHUNKFORGE_BIN=$CHUNKFORGE_BIN)"
bash "$ROOT/scripts/check_compat_1_4.sh"

echo
echo "==> 1.6 flag assertions (help text)"

# --fallback on read-side commands (at least cat / pull / verify; also extract/mount/doctor)
for cmd in cat pull verify extract mount doctor; do
  CMD_HELP="$("$BIN" "$cmd" --help)"
  if ! echo "$CMD_HELP" | grep -Fq -- '--fallback'; then
    echo "error: $cmd --help missing --fallback" >&2
    exit 1
  fi
  echo "  $cmd: --fallback OK"
done

# --cache-max-bytes still on cat / mount (suffixes are additive parsing; flag stays)
for cmd in cat mount; do
  CMD_HELP="$("$BIN" "$cmd" --help)"
  if ! echo "$CMD_HELP" | grep -Fq -- '--cache-max-bytes'; then
    echo "error: $cmd --help missing --cache-max-bytes" >&2
    exit 1
  fi
  echo "  $cmd: --cache-max-bytes OK"
done

STATS_HELP="$("$BIN" store stats --help)"
if ! echo "$STATS_HELP" | grep -Eq -- '(--decode|bytes_plaintext)'; then
  echo "error: store stats --help missing --decode or bytes_plaintext" >&2
  exit 1
fi
echo "  store stats: --decode / bytes_plaintext OK"

echo
echo "==> thin non-goal asserts (no pack / no --delete / no LRU / optional no aws-sdk)"
EXTRACT_HELP="$("$BIN" extract --help)"
if echo "$EXTRACT_HELP" | grep -Eiq -- '(^|[[:space:]])--delete([[:space:]=]|$)'; then
  echo "error: extract --help advertises --delete (prune is forbidden)" >&2
  exit 1
fi
if echo "$EXTRACT_HELP" | grep -Eiq '(^|[[:space:]])prune([[:space:]]|$)'; then
  echo "error: extract --help advertises prune (forbidden)" >&2
  exit 1
fi
echo "  extract: no --delete / prune OK"

TOP_HELP="$("$BIN" --help)"
if echo "$TOP_HELP" | grep -Eiq '(^|[[:space:]])pack([[:space:]]|$)'; then
  echo "error: top-level --help advertises a pack command (pack is deferred)" >&2
  exit 1
fi
echo "  no pack subcommand OK"

for cmd in cat verify extract mount; do
  CMD_HELP="$("$BIN" "$cmd" --help)"
  if echo "$CMD_HELP" | grep -Eiq -- '--cache-lru([[:space:]=]|$)'; then
    echo "error: $cmd --help advertises --cache-lru (LRU is forbidden)" >&2
    exit 1
  fi
done
STORE_HELP="$("$BIN" store --help)"
if echo "$STORE_HELP" | grep -Eiq '(^|[[:space:]])trim([[:space:]]|$)'; then
  echo "error: store --help advertises trim (store trim is forbidden)" >&2
  exit 1
fi
echo "  no --cache-lru / store trim OK"

if [[ -f "$ROOT/Cargo.lock" ]] && grep -Eiq '^name = "aws-sdk' "$ROOT/Cargo.lock"; then
  echo "error: Cargo.lock lists aws-sdk (forbidden)" >&2
  exit 1
fi
echo "  Cargo.lock: no aws-sdk OK"

echo
echo "==> demo_fallback_bytes_suffix presence (optional smoke file; do not force-run long demo)"
if [[ ! -f "$ROOT/scripts/demo_fallback_bytes_suffix.sh" ]]; then
  echo "error: scripts/demo_fallback_bytes_suffix.sh missing" >&2
  exit 1
fi
if [[ ! -x "$ROOT/scripts/demo_fallback_bytes_suffix.sh" ]]; then
  echo "error: scripts/demo_fallback_bytes_suffix.sh not executable" >&2
  exit 1
fi
echo "  demo_fallback_bytes_suffix.sh present + executable OK"

echo
echo "OK: check_compat_1_5"
