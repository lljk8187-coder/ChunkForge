#!/usr/bin/env bash
# Phase 14 / 1.3+ compat gate (G6 / M6): runs the 1.2 gate, then asserts
# 1.4 additive flags still appear in --help. Does **not** assert
# absolute throughput / SLA numbers and does **not** run bench_loose_http.sh.
# Local only — no real internet. Keeps check_compat_1_0.sh / 1_1 / 1_2
# independently runnable.
# Usage: ./scripts/check_compat_1_3.sh
# Requires: cargo, python3 (demos via 1_0/1_2), bash.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Phase 14 / 1.3+ compat gate"
echo "==> building chunkforge-cli (shared binary for 1_2 + 1.4 assertions)"
cargo build -p chunkforge-cli --quiet
BIN="${CHUNKFORGE_BIN:-$ROOT/target/debug/chunkforge}"
if [[ ! -x "$BIN" ]]; then
  echo "error: binary not found: $BIN" >&2
  exit 1
fi
export CHUNKFORGE_BIN="$BIN"

echo
echo "==> running check_compat_1_2.sh (CHUNKFORGE_BIN=$CHUNKFORGE_BIN)"
bash "$ROOT/scripts/check_compat_1_2.sh"

echo
echo "==> 1.4 flag assertions (help text)"

PUSH_HELP="$("$BIN" push --help)"
# G6: --path **or** --exclude (either satisfies; both expected in practice)
if ! echo "$PUSH_HELP" | grep -Fq -- '--path' && ! echo "$PUSH_HELP" | grep -Fq -- '--exclude'; then
  echo "error: push --help missing both --path and --exclude" >&2
  exit 1
fi
echo "  push: --path|--exclude OK"

# store stats and/or store du --help exist and include --format
STORE_HELP="$("$BIN" store --help)"
HAS_STATS=0
HAS_DU=0
if echo "$STORE_HELP" | grep -Eiq '(^|[[:space:]])stats([[:space:]]|$)'; then
  HAS_STATS=1
fi
if echo "$STORE_HELP" | grep -Eiq '(^|[[:space:]])du([[:space:]]|$)'; then
  HAS_DU=1
fi
if [[ "$HAS_STATS" -eq 0 && "$HAS_DU" -eq 0 ]]; then
  echo "error: store --help missing both stats and du subcommands" >&2
  exit 1
fi

if [[ "$HAS_STATS" -eq 1 ]]; then
  STATS_HELP="$("$BIN" store stats --help)"
else
  STATS_HELP="$("$BIN" store du --help)"
fi
if ! echo "$STATS_HELP" | grep -Fq -- '--format'; then
  echo "error: store stats/du --help missing --format" >&2
  exit 1
fi
echo "  store stats|du: --format OK"

# archive / extract / pull / push --help all contain --exclude-from
ARCHIVE_HELP="$("$BIN" archive --help)"
if ! echo "$ARCHIVE_HELP" | grep -Fq -- '--exclude-from'; then
  echo "error: archive --help missing --exclude-from" >&2
  exit 1
fi
echo "  archive: --exclude-from OK"

EXTRACT_HELP="$("$BIN" extract --help)"
if ! echo "$EXTRACT_HELP" | grep -Fq -- '--exclude-from'; then
  echo "error: extract --help missing --exclude-from" >&2
  exit 1
fi
echo "  extract: --exclude-from OK"

PULL_HELP="$("$BIN" pull --help)"
if ! echo "$PULL_HELP" | grep -Fq -- '--exclude-from'; then
  echo "error: pull --help missing --exclude-from" >&2
  exit 1
fi
echo "  pull: --exclude-from OK"

if ! echo "$PUSH_HELP" | grep -Fq -- '--exclude-from'; then
  echo "error: push --help missing --exclude-from" >&2
  exit 1
fi
echo "  push: --exclude-from OK"

echo
echo "==> thin non-goal asserts (inherit 1_2: no --delete / no pack; aws-sdk via 1_0)"
# Re-assert extract has no --delete / prune (1_2 already checked; keep local clarity).
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
GC_HELP="$("$BIN" gc --help)"
for cmd_help in "$PUSH_HELP" "$PULL_HELP" "$GC_HELP"; do
  if echo "$cmd_help" | grep -Eiq -- '--pack(file)?([[:space:]=]|$)'; then
    echo "error: CLI help advertises --pack / --packfile (pack is deferred)" >&2
    exit 1
  fi
done
echo "  no pack subcommand / --pack* flags OK"

echo
echo "==> demo_push_path_store_stats presence (optional smoke file; 1_2 already ran demo_path_filter)"
if [[ ! -f "$ROOT/scripts/demo_push_path_store_stats.sh" ]]; then
  echo "error: scripts/demo_push_path_store_stats.sh missing" >&2
  exit 1
fi
if [[ ! -x "$ROOT/scripts/demo_push_path_store_stats.sh" ]]; then
  echo "error: scripts/demo_push_path_store_stats.sh not executable" >&2
  exit 1
fi
echo "  demo_push_path_store_stats.sh present + executable OK"
# Full demo is covered by M5 / manual; do not force-run here (HTTP stub).

echo
echo "OK: check_compat_1_3"
