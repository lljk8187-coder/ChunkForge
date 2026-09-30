#!/usr/bin/env bash
# Phase 19 / 1.8+ compat gate (G5 / M5): runs the 1.7 gate, then asserts
# 1.9 additive flags still appear in --help. Does **not** assert
# absolute throughput / SLA numbers and does **not** force-run long demos.
# Local only — no real internet. Keeps check_compat_1_0.sh … 1_7
# independently runnable.
# Usage: ./scripts/check_compat_1_8.sh
# Requires: cargo, python3 (demos via 1_0…1_7), bash.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Phase 19 / 1.8+ compat gate"
echo "==> building chunkforge-cli (shared binary for 1_7 + 1.9 assertions)"
cargo build -p chunkforge-cli --quiet
BIN="${CHUNKFORGE_BIN:-$ROOT/target/debug/chunkforge}"
if [[ ! -x "$BIN" ]]; then
  echo "error: binary not found: $BIN" >&2
  exit 1
fi
export CHUNKFORGE_BIN="$BIN"

echo
echo "==> running check_compat_1_7.sh (CHUNKFORGE_BIN=$CHUNKFORGE_BIN)"
bash "$ROOT/scripts/check_compat_1_7.sh"

echo
echo "==> 1.9 flag assertions (help text)"

# store create (store --help exposes create; store create --help runnable)
STORE_HELP="$("$BIN" store --help)"
if ! echo "$STORE_HELP" | grep -Eiq '(^|[[:space:]])create([[:space:]]|$)'; then
  echo "error: store --help missing create subcommand" >&2
  exit 1
fi
echo "  store: create subcommand OK"

CREATE_HELP="$("$BIN" store create --help)"
if ! echo "$CREATE_HELP" | grep -Fq -- '--store'; then
  echo "error: store create --help missing --store" >&2
  exit 1
fi
if ! echo "$CREATE_HELP" | grep -Fq -- '--compression'; then
  echo "error: store create --help missing --compression" >&2
  exit 1
fi
echo "  store create: --store / --compression OK"

# pull --compression (create-time; omit ≡ none ≡ 1.8)
PULL_HELP="$("$BIN" pull --help)"
if ! echo "$PULL_HELP" | grep -Fq -- '--compression'; then
  echo "error: pull --help missing --compression" >&2
  exit 1
fi
echo "  pull: --compression OK"

# diff --progress (default off ≡ 1.8)
DIFF_HELP="$("$BIN" diff --help)"
if ! echo "$DIFF_HELP" | grep -Fq -- '--progress'; then
  echo "error: diff --help missing --progress" >&2
  exit 1
fi
echo "  diff: --progress OK"

echo
echo "==> thin non-goal re-asserts (no --delete / no pack / no LRU / no aws-sdk / no default zstd / no recompress / no push --fallback)"
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
if echo "$MAKE_HELP" | grep -Eiq 'default[[:space:]]+zstd|defaults?[[:space:]]+to[[:space:]]+zstd'; then
  echo "error: make --help advertises default zstd (forbidden; create default is none)" >&2
  exit 1
fi
if ! echo "$MAKE_HELP" | grep -Eiq 'default[[:space:]]+none|≡[[:space:]]*1\.[5678]|omit.*none'; then
  if ! echo "$MAKE_HELP" | grep -Fiq 'none'; then
    echo "error: make --help compression narrative missing opt-in/none default hint" >&2
    exit 1
  fi
fi
# store create narrative: omit ≡ none, not default zstd
if echo "$CREATE_HELP" | grep -Eiq 'default[[:space:]]+zstd|defaults?[[:space:]]+to[[:space:]]+zstd'; then
  echo "error: store create --help advertises default zstd (forbidden)" >&2
  exit 1
fi
echo "  make/store create: --compression opt-in / no default zstd OK"

# No store recompress
if echo "$STORE_HELP" | grep -Eiq '(^|[[:space:]])recompress([[:space:]]|$)'; then
  echo "error: store --help advertises recompress (forbidden)" >&2
  exit 1
fi
echo "  store: no recompress OK"

# push has no --fallback (write-side single dest)
PUSH_HELP="$("$BIN" push --help)"
if echo "$PUSH_HELP" | grep -Eiq -- '(^|[[:space:]])--fallback([[:space:]=]|$)'; then
  echo "error: push --help advertises --fallback (write-side multi-dest is forbidden)" >&2
  exit 1
fi
echo "  push: no --fallback OK"

echo
echo "==> demo_store_create_pull_compression presence (P1 O3; do not force-run long demo)"
if [[ ! -f "$ROOT/scripts/demo_store_create_pull_compression.sh" ]]; then
  echo "error: scripts/demo_store_create_pull_compression.sh missing" >&2
  exit 1
fi
if [[ ! -x "$ROOT/scripts/demo_store_create_pull_compression.sh" ]]; then
  echo "error: scripts/demo_store_create_pull_compression.sh not executable" >&2
  exit 1
fi
echo "  demo_store_create_pull_compression.sh present + executable OK"

echo
echo "OK: check_compat_1_8"
