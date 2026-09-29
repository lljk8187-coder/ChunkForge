#!/usr/bin/env bash
# Incremental dedup demo: same input twice → no new chunks; mid-file mutate → reuse.
# Usage: ./scripts/demo_dedup.sh [size_mib]
# Requires: cargo (builds chunkforge), python3 (via gen_large.sh).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SIZE_MIB="${1:-${CHUNKFORGE_GEN_MIB:-64}}"
GEN_DIR="${CHUNKFORGE_GEN_DIR:-fixtures/gen}"
DEMO_DIR="${CHUNKFORGE_DEMO_DIR:-/tmp/cf-dedup-demo}"
BIN="${CHUNKFORGE_BIN:-}"

echo "==> building chunkforge"
cargo build -p chunkforge-cli --quiet
if [[ -z "$BIN" ]]; then
  BIN="$ROOT/target/debug/chunkforge"
fi

echo "==> generating ${SIZE_MIB}MiB fixtures under $GEN_DIR"
./scripts/gen_large.sh "$GEN_DIR" "$SIZE_MIB"

ORIG="$GEN_DIR/large-${SIZE_MIB}m.bin"
MUT="$GEN_DIR/large-${SIZE_MIB}m-mut.bin"
STORE="$DEMO_DIR/store"
rm -rf "$DEMO_DIR"
mkdir -p "$DEMO_DIR"

count_cnk() {
  find "$1" -type f -name '*.cnk' 2>/dev/null | wc -l | tr -d ' '
}

echo
echo "==> make #1 (original)"
"$BIN" make --store "$STORE" -o "$DEMO_DIR/v1.cfidx" "$ORIG"
CNK1=$(count_cnk "$STORE/chunks")
echo "store .cnk files after make #1: $CNK1"

echo
echo "==> make #2 (same original — expect new=0, .cnk count unchanged)"
"$BIN" make --store "$STORE" -o "$DEMO_DIR/v1b.cfidx" "$ORIG"
CNK2=$(count_cnk "$STORE/chunks")
echo "store .cnk files after make #2: $CNK2"
if [[ "$CNK1" -ne "$CNK2" ]]; then
  echo "error: expected identical .cnk count after identical remake ($CNK1 vs $CNK2)" >&2
  exit 1
fi

echo
echo "==> make #3 (mid-file mutated — expect some new + many reused)"
"$BIN" make --store "$STORE" -o "$DEMO_DIR/v2.cfidx" "$MUT"
CNK3=$(count_cnk "$STORE/chunks")
echo "store .cnk files after make #3: $CNK3"
if [[ "$CNK3" -le "$CNK2" ]]; then
  echo "error: expected .cnk count to grow after mutation ($CNK3 <= $CNK2)" >&2
  exit 1
fi

echo
echo "==> verify mutated index"
"$BIN" verify --store "$STORE" "$DEMO_DIR/v2.cfidx"

echo
echo "dedup demo OK"
echo "  identical remake: .cnk $CNK1 → $CNK2 (unchanged)"
echo "  after mid-file mutate: .cnk $CNK2 → $CNK3 (grew; check new=/reused= lines above)"
