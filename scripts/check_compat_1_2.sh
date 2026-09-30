#!/usr/bin/env bash
# Phase 13 / 1.2+ compat gate (G7 / M6): runs the 1.1 gate, then asserts
# 1.3 additive flags still appear in --help. Does **not** assert
# absolute throughput / SLA numbers and does **not** run bench_loose_http.sh.
# Local only — no real internet. Keeps check_compat_1_0.sh / 1_1 independently runnable.
# Usage: ./scripts/check_compat_1_2.sh
# Requires: cargo, python3 (demos via 1_0), bash.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Phase 13 / 1.2+ compat gate"
echo "==> building chunkforge-cli (shared binary for 1_1 + 1.2 assertions)"
cargo build -p chunkforge-cli --quiet
BIN="${CHUNKFORGE_BIN:-$ROOT/target/debug/chunkforge}"
if [[ ! -x "$BIN" ]]; then
  echo "error: binary not found: $BIN" >&2
  exit 1
fi
export CHUNKFORGE_BIN="$BIN"

echo
echo "==> running check_compat_1_1.sh (CHUNKFORGE_BIN=$CHUNKFORGE_BIN)"
bash "$ROOT/scripts/check_compat_1_1.sh"

echo
echo "==> 1.3 flag assertions (help text)"

ARCHIVE_HELP="$("$BIN" archive --help)"
if ! grep -Fq -- '--format' <<<"$ARCHIVE_HELP"; then
  echo "error: archive --help missing --format" >&2
  exit 1
fi
# G7: --path **or** --exclude (either satisfies; both expected in practice)
if ! grep -Fq -- '--path' <<<"$ARCHIVE_HELP" && ! grep -Fq -- '--exclude' <<<"$ARCHIVE_HELP"; then
  echo "error: archive --help missing both --path and --exclude" >&2
  exit 1
fi
echo "  archive: --format / --path|--exclude OK"

EXTRACT_HELP="$("$BIN" extract --help)"
if ! grep -Fq -- '--path' <<<"$EXTRACT_HELP"; then
  echo "error: extract --help missing --path" >&2
  exit 1
fi
echo "  extract: --path OK"

PULL_HELP="$("$BIN" pull --help)"
if ! grep -Fq -- '--path' <<<"$PULL_HELP"; then
  echo "error: pull --help missing --path" >&2
  exit 1
fi
echo "  pull: --path OK"

echo
echo "==> thin non-goal asserts (no --delete on extract; no pack; aws-sdk via 1_0)"
# extract must not advertise prune / --delete (path filter ≠ prune).
if grep -Eiq -- '(^|[[:space:]])--delete([[:space:]=]|$)' <<<"$EXTRACT_HELP"; then
  echo "error: extract --help advertises --delete (prune is forbidden)" >&2
  exit 1
fi
if grep -Eiq '(^|[[:space:]])prune([[:space:]]|$)' <<<"$EXTRACT_HELP"; then
  echo "error: extract --help advertises prune (forbidden)" >&2
  exit 1
fi
echo "  extract: no --delete / prune OK"

# Help must not advertise a first-class `pack` subcommand / fake pack flags.
TOP_HELP="$("$BIN" --help)"
if grep -Eiq '(^|[[:space:]])pack([[:space:]]|$)' <<<"$TOP_HELP"; then
  echo "error: top-level --help advertises a pack command (pack is deferred)" >&2
  exit 1
fi
PUSH_HELP="$("$BIN" push --help)"
GC_HELP="$("$BIN" gc --help)"
for cmd_help in "$PUSH_HELP" "$PULL_HELP" "$GC_HELP"; do
  if grep -Eiq -- '--pack(file)?([[:space:]=]|$)' <<<"$cmd_help"; then
    echo "error: CLI help advertises --pack / --packfile (pack is deferred)" >&2
    exit 1
  fi
done
echo "  no pack subcommand / --pack* flags OK"

echo
echo "==> demo_path_filter smoke (O4: assert callable)"
if [[ ! -x "$ROOT/scripts/demo_path_filter.sh" ]]; then
  echo "error: scripts/demo_path_filter.sh missing or not executable" >&2
  exit 1
fi
bash "$ROOT/scripts/demo_path_filter.sh"

echo
echo "OK: check_compat_1_2"
