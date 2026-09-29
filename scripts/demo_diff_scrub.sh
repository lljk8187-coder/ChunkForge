#!/usr/bin/env bash
# Phase 7 diff + scrub smoke: two archives → diff (changed) →
# diff --tree → healthy store scrub → flip one .cnk byte → scrub corrupt
# (optional restore). Local only — no real internet.
# Usage: ./scripts/demo_diff_scrub.sh
# Requires: cargo, python3.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_DIFF_DEMO_DIR:-/tmp/cf-diff-scrub-demo}"
BIN="${CHUNKFORGE_BIN:-}"
FIXTURE="${CHUNKFORGE_DIFF_FIXTURE:-$ROOT/fixtures/hello.txt}"
PYTHON="${CHUNKFORGE_PYTHON:-python3}"

if [[ ! -f "$FIXTURE" ]]; then
  echo "error: fixture not found: $FIXTURE" >&2
  exit 1
fi

if ! command -v "$PYTHON" >/dev/null 2>&1; then
  echo "error: $PYTHON not on PATH" >&2
  exit 1
fi

echo "==> building chunkforge"
cargo build -p chunkforge-cli --quiet
if [[ -z "$BIN" ]]; then
  BIN="$ROOT/target/debug/chunkforge"
fi

rm -rf "$DEMO_DIR"
SRC="$DEMO_DIR/src"
STORE="$DEMO_DIR/store"
V1="$DEMO_DIR/v1.cfdir"
V2="$DEMO_DIR/v2.cfdir"
mkdir -p "$SRC/sub" "$STORE"

echo 'hello-diff-v1' > "$SRC/a.txt"
cp "$FIXTURE" "$SRC/sub/b.txt"

echo
echo "==> first archive $SRC → $V1"
"$BIN" archive --store "$STORE" -o "$V1" "$SRC"

echo
echo "==> change one file (a.txt) and archive --seed → $V2"
echo 'hello-diff-v2' > "$SRC/a.txt"
"$BIN" archive --store "$STORE" -o "$V2" --seed "$V1" "$SRC"

echo
echo "==> diff v1 ↔ v2 (expect changed≥1, exit≠0)"
set +e
DIFF_OUT="$("$BIN" diff "$V1" "$V2" 2>&1)"
DIFF_RC=$?
set -e
echo "$DIFF_OUT"
if [[ "$DIFF_RC" -eq 0 ]]; then
  echo "error: expected non-zero exit when listings differ" >&2
  exit 1
fi
if ! echo "$DIFF_OUT" | grep -Eq 'changed=[1-9]'; then
  echo "error: expected changed≥1 in summary line" >&2
  exit 1
fi
echo "diff listing↔listing: OK (exit=$DIFF_RC)"

echo
echo "==> diff --tree $SRC ↔ v2 (expect identical, exit 0)"
TREE_OUT="$("$BIN" diff --tree "$SRC" "$V2")"
echo "$TREE_OUT"
if ! echo "$TREE_OUT" | grep -Eq 'added=0 removed=0 changed=0'; then
  echo "error: expected tree↔v2 identical (added=0 removed=0 changed=0)" >&2
  exit 1
fi
echo "diff --tree: OK"

echo
echo "==> store scrub (healthy — expect corrupt=0)"
SCRUB_OK="$("$BIN" store scrub --store "$STORE" 2>&1)"
echo "$SCRUB_OK"
if ! echo "$SCRUB_OK" | grep -Eq 'corrupt=0'; then
  echo "error: expected corrupt=0 on healthy store" >&2
  exit 1
fi
if ! echo "$SCRUB_OK" | grep -Eq 'unreadable=0'; then
  echo "error: expected unreadable=0 on healthy store" >&2
  exit 1
fi
echo "scrub healthy: OK"

echo
echo "==> flip one .cnk byte"
CORRUPT_PATH="$("$PYTHON" - <<PY
from pathlib import Path
root = Path("$STORE") / "chunks"
cnks = sorted(root.glob("*/*.cnk"))
assert cnks, "no chunks"
p = cnks[0]
data = bytearray(p.read_bytes())
# keep a backup alongside for optional restore
bak = p.with_suffix(".cnk.bak")
bak.write_bytes(bytes(data))
data[0] ^= 0xFF
p.write_bytes(data)
print(p)
PY
)"
echo "corrupted $CORRUPT_PATH"

echo
echo "==> store scrub (expect corrupt≥1, non-zero exit)"
set +e
SCRUB_BAD="$("$BIN" store scrub --store "$STORE" 2>&1)"
SCRUB_RC=$?
set -e
echo "$SCRUB_BAD"
if [[ "$SCRUB_RC" -eq 0 ]]; then
  echo "error: expected non-zero exit when corrupt chunks present" >&2
  exit 1
fi
if ! echo "$SCRUB_BAD" | grep -Eq 'corrupt=[1-9]'; then
  echo "error: expected corrupt≥1 after flipping a .cnk byte" >&2
  exit 1
fi
echo "scrub corrupt: OK (exit=$SCRUB_RC)"

echo
echo "==> restore corrupted chunk from .bak"
"$PYTHON" - <<PY
from pathlib import Path
p = Path("$CORRUPT_PATH")
bak = p.with_suffix(".cnk.bak")
assert bak.is_file(), bak
p.write_bytes(bak.read_bytes())
bak.unlink()
print("restored", p)
PY

echo
echo "==> store scrub after restore (expect corrupt=0)"
SCRUB_REST="$("$BIN" store scrub --store "$STORE" 2>&1)"
echo "$SCRUB_REST"
if ! echo "$SCRUB_REST" | grep -Eq 'corrupt=0'; then
  echo "error: expected corrupt=0 after restore" >&2
  exit 1
fi

echo
echo "diff+scrub demo OK"
echo "  store=$STORE"
echo "  v1=$V1"
echo "  v2=$V2"
