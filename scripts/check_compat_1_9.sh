#!/usr/bin/env bash
# Phase 20 / 1.9+ compat gate (G5 / M5): runs the 1.8 gate, then asserts
# 1.10 additive flags still appear in --help. Does **not** assert
# absolute throughput / SLA numbers and does **not** force-run long demos.
# Local only — no real internet. Keeps check_compat_1_0.sh … 1_8
# independently runnable.
# Usage: ./scripts/check_compat_1_9.sh
# Requires: cargo, python3 (demos via 1_0…1_8), bash.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Phase 20 / 1.9+ compat gate"
echo "==> building chunkforge-cli (shared binary for 1_8 + 1.10 assertions)"
cargo build -p chunkforge-cli --quiet
BIN="${CHUNKFORGE_BIN:-$ROOT/target/debug/chunkforge}"
if [[ ! -x "$BIN" ]]; then
  echo "error: binary not found: $BIN" >&2
  exit 1
fi
export CHUNKFORGE_BIN="$BIN"

echo
echo "==> running check_compat_1_8.sh (CHUNKFORGE_BIN=$CHUNKFORGE_BIN)"
bash "$ROOT/scripts/check_compat_1_8.sh"

echo
echo "==> 1.10 flag assertions (help text)"

# --path-from on path-scoped commands (spot-check: archive + full set)
for cmd in archive extract push pull diff doctor verify; do
  CMD_HELP="$("$BIN" "$cmd" --help)"
  if ! grep -Fq -- '--path-from' <<<"$CMD_HELP"; then
    echo "error: $cmd --help missing --path-from" >&2
    exit 1
  fi
  echo "  $cmd: --path-from OK"
done

# doctor / verify expose --path (and --path-from already covered)
for cmd in doctor verify; do
  CMD_HELP="$("$BIN" "$cmd" --help)"
  if ! grep -Eiq -- '(^|[[:space:]])--path([[:space:]=]|$)' <<<"$CMD_HELP"; then
    echo "error: $cmd --help missing --path" >&2
    exit 1
  fi
  echo "  $cmd: --path OK"
done

echo
echo "==> thin non-goal re-asserts (no --delete / no pack / no LRU / no aws-sdk / no default zstd / no recompress / no push --fallback / no gc --path)"
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

# Compression remains create-time opt-in; default none (not silent default-zstd).
MAKE_HELP="$("$BIN" make --help)"
CREATE_HELP="$("$BIN" store create --help)"
if ! grep -Fq -- '--compression' <<<"$MAKE_HELP"; then
  echo "error: make --help missing --compression" >&2
  exit 1
fi
if grep -Eiq 'default[[:space:]]+zstd|defaults?[[:space:]]+to[[:space:]]+zstd' <<<"$MAKE_HELP"; then
  echo "error: make --help advertises default zstd (forbidden; create default is none)" >&2
  exit 1
fi
if ! grep -Eiq 'default[[:space:]]+none|≡[[:space:]]*1\.[56789]|omit.*none' <<<"$MAKE_HELP"; then
  if ! grep -Fiq 'none' <<<"$MAKE_HELP"; then
    echo "error: make --help compression narrative missing opt-in/none default hint" >&2
    exit 1
  fi
fi
if grep -Eiq 'default[[:space:]]+zstd|defaults?[[:space:]]+to[[:space:]]+zstd' <<<"$CREATE_HELP"; then
  echo "error: store create --help advertises default zstd (forbidden)" >&2
  exit 1
fi
echo "  make/store create: --compression opt-in / no default zstd OK"

# No store recompress
if grep -Eiq '(^|[[:space:]])recompress([[:space:]]|$)' <<<"$STORE_HELP"; then
  echo "error: store --help advertises recompress (forbidden)" >&2
  exit 1
fi
echo "  store: no recompress OK"

# push has no --fallback (write-side single dest)
PUSH_HELP="$("$BIN" push --help)"
if grep -Eiq -- '(^|[[:space:]])--fallback([[:space:]=]|$)' <<<"$PUSH_HELP"; then
  echo "error: push --help advertises --fallback (write-side multi-dest is forbidden)" >&2
  exit 1
fi
echo "  push: no --fallback OK"

# gc has no --path / --path-from (hard ban: shrinking keep-set mis-deletes)
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

echo
echo "==> demo_path_from_doctor_verify presence (P1 O3; do not force-run long demo)"
if [[ ! -f "$ROOT/scripts/demo_path_from_doctor_verify.sh" ]]; then
  echo "error: scripts/demo_path_from_doctor_verify.sh missing" >&2
  exit 1
fi
if [[ ! -x "$ROOT/scripts/demo_path_from_doctor_verify.sh" ]]; then
  echo "error: scripts/demo_path_from_doctor_verify.sh not executable" >&2
  exit 1
fi
echo "  demo_path_from_doctor_verify.sh present + executable OK"

echo
echo "OK: check_compat_1_9"
