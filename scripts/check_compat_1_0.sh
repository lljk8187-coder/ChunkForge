#!/usr/bin/env bash
# Phase 10 / 1.0 compat gate (G4): non-destructive defaults, no aws-sdk,
# key CLI flags still present, and a local demo subset. Does **not** assert
# absolute throughput / SLA numbers and does **not** run bench_loose_http.sh.
# Local only — no real internet.
# Usage: ./scripts/check_compat_1_0.sh
# Requires: cargo, python3 (demos), bash.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

BIN="${CHUNKFORGE_BIN:-}"

echo "==> Phase 10 / 1.0 compat gate"
echo "==> building chunkforge-cli"
cargo build -p chunkforge-cli --quiet
if [[ -z "$BIN" ]]; then
  BIN="$ROOT/target/debug/chunkforge"
fi
if [[ ! -x "$BIN" ]]; then
  echo "error: binary not found: $BIN" >&2
  exit 1
fi

echo
echo "==> CLI help: extract / mount / diff / push|pull (key flags present)"
EXTRACT_HELP="$("$BIN" extract --help)"
for flag in --skip-unchanged --dry-run --force; do
  if ! grep -Fq -- "$flag" <<<"$EXTRACT_HELP"; then
    echo "error: extract --help missing $flag" >&2
    exit 1
  fi
done
# Default must stay off for skip-unchanged (≡ 0.8.0 / 0.9.0); help should not
# claim it is enabled by default.
if grep -Ei 'skip-unchanged.*default[[:space:]]*(on|true|enabled)' <<<"$EXTRACT_HELP" >/dev/null; then
  echo "error: extract --help implies --skip-unchanged default on (breaking)" >&2
  exit 1
fi
echo "  extract: --skip-unchanged / --dry-run / --force OK (skip default off)"

MOUNT_HELP="$("$BIN" mount --help)"
if ! grep -Fq -- '--no-prefetch' <<<"$MOUNT_HELP"; then
  echo "error: mount --help missing --no-prefetch" >&2
  exit 1
fi
echo "  mount: --no-prefetch OK"

DIFF_HELP="$("$BIN" diff --help)"
if ! grep -Fq -- '--format' <<<"$DIFF_HELP"; then
  echo "error: diff --help missing --format" >&2
  exit 1
fi
echo "  diff: --format OK"

PUSH_HELP="$("$BIN" push --help)"
PULL_HELP="$("$BIN" pull --help)"
if ! grep -Fq -- '--http-retries' <<<"$PUSH_HELP" && ! grep -Fq -- '--http-retries' <<<"$PULL_HELP"; then
  echo "error: neither push nor pull --help mentions --http-retries" >&2
  exit 1
fi
if ! grep -Fq -- '--aws-sigv4' <<<"$PUSH_HELP" && ! grep -Fq -- '--aws-sigv4' <<<"$PULL_HELP"; then
  echo "error: neither push nor pull --help mentions --aws-sigv4" >&2
  exit 1
fi
echo "  push/pull: --http-retries / --aws-sigv4 OK"

echo
echo "==> no aws-sdk in dependency graph / Cargo.lock"
# Failure to resolve aws-sdk (= no such package) is the green path.
if cargo tree -i aws-sdk >/tmp/cf-compat-aws-tree.out 2>&1; then
  echo "error: cargo tree -i aws-sdk resolved a package (aws-sdk forbidden):" >&2
  cat /tmp/cf-compat-aws-tree.out >&2
  exit 1
fi
if ! grep -Eqi 'did not match|no matching|not found' /tmp/cf-compat-aws-tree.out; then
  # Unexpected cargo tree failure — surface and fail.
  echo "error: cargo tree -i aws-sdk failed unexpectedly:" >&2
  cat /tmp/cf-compat-aws-tree.out >&2
  exit 1
fi
if grep -Eiq 'aws-sdk' Cargo.lock; then
  echo "error: Cargo.lock contains aws-sdk*" >&2
  grep -Ei 'aws-sdk' Cargo.lock >&2 || true
  exit 1
fi
echo "  cargo tree / Cargo.lock: no aws-sdk OK"

echo
echo "==> fuse lib prefetch unit tests (no real mount)"
cargo test -p chunkforge-fuse --lib \
  sequential_read_prefetch_skips_reget_of_next_chunk -- --nocapture
cargo test -p chunkforge-fuse --lib \
  prefetch_disabled_get_count_equals_on_demand_path -- --nocapture

echo
echo "==> demo subset (must not silently skip)"
bash "$ROOT/scripts/demo_extract_skip.sh"
bash "$ROOT/scripts/demo_http_retry.sh"
bash "$ROOT/scripts/demo_diff_scrub.sh"

echo
echo "OK: check_compat_1_0"
