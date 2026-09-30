#!/usr/bin/env bash
# Phase 17 / 1.6+ compat gate (G6 / M6): runs the 1.5 gate, then asserts
# 1.7 additive flags still appear in --help. Does **not** assert
# absolute throughput / SLA numbers and does **not** run bench_loose_http.sh
# or force-run long demos. Local only — no real internet. Keeps
# check_compat_1_0.sh … 1_5 independently runnable.
# Usage: ./scripts/check_compat_1_6.sh
# Requires: cargo, python3 (demos via 1_0…1_5), bash.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Phase 17 / 1.6+ compat gate"
echo "==> building chunkforge-cli (shared binary for 1_5 + 1.7 assertions)"
cargo build -p chunkforge-cli --quiet
BIN="${CHUNKFORGE_BIN:-$ROOT/target/debug/chunkforge}"
if [[ ! -x "$BIN" ]]; then
  echo "error: binary not found: $BIN" >&2
  exit 1
fi
export CHUNKFORGE_BIN="$BIN"

echo
echo "==> running check_compat_1_5.sh (CHUNKFORGE_BIN=$CHUNKFORGE_BIN)"
bash "$ROOT/scripts/check_compat_1_5.sh"

echo
echo "==> 1.7 flag assertions (help text)"

# --compression on make / archive (create-time opt-in; default none ≡ 1.6)
for cmd in make archive; do
  CMD_HELP="$("$BIN" "$cmd" --help)"
  if ! grep -Fq -- '--compression' <<<"$CMD_HELP"; then
    echo "error: $cmd --help missing --compression" >&2
    exit 1
  fi
  echo "  $cmd: --compression OK"
done

# --progress on archive / extract / make (default off ≡ 1.6)
for cmd in archive extract make; do
  CMD_HELP="$("$BIN" "$cmd" --help)"
  if ! grep -Fq -- '--progress' <<<"$CMD_HELP"; then
    echo "error: $cmd --help missing --progress" >&2
    exit 1
  fi
  echo "  $cmd: --progress OK"
done

echo
echo "==> thin non-goal re-asserts (no --delete / no pack / no LRU / no aws-sdk)"
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

TOP_HELP="$("$BIN" --help)"
if grep -Eiq '(^|[[:space:]])pack([[:space:]]|$)' <<<"$TOP_HELP"; then
  echo "error: top-level --help advertises a pack command (pack is deferred)" >&2
  exit 1
fi
echo "  no pack subcommand OK"

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

if [[ -f "$ROOT/Cargo.lock" ]] && grep -Eiq '^name = "aws-sdk' "$ROOT/Cargo.lock"; then
  echo "error: Cargo.lock lists aws-sdk (forbidden)" >&2
  exit 1
fi
echo "  Cargo.lock: no aws-sdk OK"

echo
echo "==> demo_zstd_progress presence (optional smoke file; do not force-run long demo)"
if [[ ! -f "$ROOT/scripts/demo_zstd_progress.sh" ]]; then
  echo "error: scripts/demo_zstd_progress.sh missing" >&2
  exit 1
fi
if [[ ! -x "$ROOT/scripts/demo_zstd_progress.sh" ]]; then
  echo "error: scripts/demo_zstd_progress.sh not executable" >&2
  exit 1
fi
echo "  demo_zstd_progress.sh present + executable OK"

echo
echo "OK: check_compat_1_6"
