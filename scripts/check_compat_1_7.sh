#!/usr/bin/env bash
# Phase 18 / 1.7+ compat gate (G6 / M6): runs the 1.6 gate, then asserts
# 1.8 additive flags still appear in --help. Does **not** assert
# absolute throughput / SLA numbers and does **not** force-run long demos.
# Local only — no real internet. Keeps check_compat_1_0.sh … 1_6
# independently runnable.
# Usage: ./scripts/check_compat_1_7.sh
# Requires: cargo, python3 (demos via 1_0…1_6), bash.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Phase 18 / 1.7+ compat gate"
echo "==> building chunkforge-cli (shared binary for 1_6 + 1.8 assertions)"
cargo build -p chunkforge-cli --quiet
BIN="${CHUNKFORGE_BIN:-$ROOT/target/debug/chunkforge}"
if [[ ! -x "$BIN" ]]; then
  echo "error: binary not found: $BIN" >&2
  exit 1
fi
export CHUNKFORGE_BIN="$BIN"

echo
echo "==> running check_compat_1_6.sh (CHUNKFORGE_BIN=$CHUNKFORGE_BIN)"
bash "$ROOT/scripts/check_compat_1_6.sh"

echo
echo "==> 1.8 flag assertions (help text)"

# pull --verify (default off ≡ 1.7; symmetric to push --verify)
PULL_HELP="$("$BIN" pull --help)"
if ! echo "$PULL_HELP" | grep -Fq -- '--verify'; then
  echo "error: pull --help missing --verify" >&2
  exit 1
fi
echo "  pull: --verify OK"

# --cache-stats on at least one read command (cat or verify)
CACHE_STATS_OK=0
for cmd in cat verify; do
  CMD_HELP="$("$BIN" "$cmd" --help)"
  if echo "$CMD_HELP" | grep -Fq -- '--cache-stats'; then
    CACHE_STATS_OK=1
    echo "  $cmd: --cache-stats OK"
    break
  fi
done
if [[ "$CACHE_STATS_OK" -ne 1 ]]; then
  echo "error: neither cat nor verify --help contains --cache-stats" >&2
  exit 1
fi

# cat / verify --progress (default off ≡ 1.7)
for cmd in cat verify; do
  CMD_HELP="$("$BIN" "$cmd" --help)"
  if ! echo "$CMD_HELP" | grep -Fq -- '--progress'; then
    echo "error: $cmd --help missing --progress" >&2
    exit 1
  fi
  echo "  $cmd: --progress OK"
done

echo
echo "==> thin non-goal re-asserts (no --delete / no pack / no LRU / no aws-sdk / no default zstd)"
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

# Compression remains create-time opt-in; default none (not silent default-zstd).
MAKE_HELP="$("$BIN" make --help)"
if ! echo "$MAKE_HELP" | grep -Fq -- '--compression'; then
  echo "error: make --help missing --compression" >&2
  exit 1
fi
# Help narrative must still describe default none / opt-in (not "default zstd").
if echo "$MAKE_HELP" | grep -Eiq 'default[[:space:]]+zstd|defaults?[[:space:]]+to[[:space:]]+zstd'; then
  echo "error: make --help advertises default zstd (forbidden; create default is none)" >&2
  exit 1
fi
if ! echo "$MAKE_HELP" | grep -Eiq 'default[[:space:]]+none|≡[[:space:]]*1\.[56]|omit.*none'; then
  # Soft check: clap help usually carries "Default none" from the arg doc.
  if ! echo "$MAKE_HELP" | grep -Fiq 'none'; then
    echo "error: make --help compression narrative missing opt-in/none default hint" >&2
    exit 1
  fi
fi
echo "  make: --compression opt-in / no default zstd OK"

echo
echo "==> demo_pull_verify_cache_stats presence (P1 O3; do not force-run long demo)"
if [[ ! -f "$ROOT/scripts/demo_pull_verify_cache_stats.sh" ]]; then
  echo "error: scripts/demo_pull_verify_cache_stats.sh missing" >&2
  exit 1
fi
if [[ ! -x "$ROOT/scripts/demo_pull_verify_cache_stats.sh" ]]; then
  echo "error: scripts/demo_pull_verify_cache_stats.sh not executable" >&2
  exit 1
fi
echo "  demo_pull_verify_cache_stats.sh present + executable OK"

echo
echo "OK: check_compat_1_7"
