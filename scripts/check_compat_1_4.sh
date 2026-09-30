#!/usr/bin/env bash
# Phase 15 / 1.4+ compat gate (G6 / M6): runs the 1.3 gate, then asserts
# 1.5 additive flags still appear in --help. Does **not** assert
# absolute throughput / SLA numbers and does **not** run bench_loose_http.sh.
# Local only — no real internet. Keeps check_compat_1_0.sh / 1_1 / 1_2 / 1_3
# independently runnable.
# Usage: ./scripts/check_compat_1_4.sh
# Requires: cargo, python3 (demos via 1_0…1_3), bash.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Phase 15 / 1.4+ compat gate"
echo "==> building chunkforge-cli (shared binary for 1_3 + 1.5 assertions)"
cargo build -p chunkforge-cli --quiet
BIN="${CHUNKFORGE_BIN:-$ROOT/target/debug/chunkforge}"
if [[ ! -x "$BIN" ]]; then
  echo "error: binary not found: $BIN" >&2
  exit 1
fi
export CHUNKFORGE_BIN="$BIN"

echo
echo "==> running check_compat_1_3.sh (CHUNKFORGE_BIN=$CHUNKFORGE_BIN)"
bash "$ROOT/scripts/check_compat_1_3.sh"

echo
echo "==> 1.5 flag assertions (help text)"

MAKE_HELP="$("$BIN" make --help)"
if ! grep -Fq -- '--format' <<<"$MAKE_HELP"; then
  echo "error: make --help missing --format" >&2
  exit 1
fi
echo "  make: --format OK"

CAT_HELP="$("$BIN" cat --help)"
if ! grep -Fq -- '--format' <<<"$CAT_HELP"; then
  echo "error: cat --help missing --format" >&2
  exit 1
fi
echo "  cat: --format OK"

# --cache-max-bytes on cat / verify / extract / mount (all four preferred)
for cmd in cat verify extract mount; do
  CMD_HELP="$("$BIN" "$cmd" --help)"
  if ! grep -Fq -- '--cache-max-bytes' <<<"$CMD_HELP"; then
    echo "error: $cmd --help missing --cache-max-bytes" >&2
    exit 1
  fi
  echo "  $cmd: --cache-max-bytes OK"
done

echo
echo "==> thin non-goal asserts (inherit 1_3: no --delete / no pack / aws-sdk via 1_0; + no LRU)"
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

# No cache LRU / store trim advertising
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

echo
echo "==> demo_cache_budget_ops_json presence (optional smoke file; do not force-run HTTP/long demo)"
if [[ ! -f "$ROOT/scripts/demo_cache_budget_ops_json.sh" ]]; then
  echo "error: scripts/demo_cache_budget_ops_json.sh missing" >&2
  exit 1
fi
if [[ ! -x "$ROOT/scripts/demo_cache_budget_ops_json.sh" ]]; then
  echo "error: scripts/demo_cache_budget_ops_json.sh not executable" >&2
  exit 1
fi
echo "  demo_cache_budget_ops_json.sh present + executable OK"

echo
echo "OK: check_compat_1_4"
