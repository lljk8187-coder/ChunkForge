#!/usr/bin/env bash
# Phase 9 extract-skip smoke: first extract → --skip-unchanged (skipped=all,
# zero chunk GET via put_stub) → change one file → skipped=N-1 wrote=1 →
# optional dry-run glance. Local only — no real internet.
# Usage: ./scripts/demo_extract_skip.sh
# Requires: cargo, python3.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_EXTRACT_SKIP_DEMO_DIR:-/tmp/cf-extract-skip-demo}"
BIN="${CHUNKFORGE_BIN:-}"
FIXTURE="${CHUNKFORGE_EXTRACT_SKIP_FIXTURE:-$ROOT/fixtures/hello.txt}"
PORT="${CHUNKFORGE_EXTRACT_SKIP_PORT:-8770}"
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
OUT="$DEMO_DIR/out"
V1="$DEMO_DIR/v1.cfdir"
V2="$DEMO_DIR/v2.cfdir"
mkdir -p "$SRC" "$STORE"

echo 'hello-extract-v1' > "$SRC/a.txt"
cp "$FIXTURE" "$SRC/b.txt"
# N=2 regular files in the listing
N_FILES=2

STUB_PID=""
cleanup() {
  if [[ -n "${STUB_PID:-}" ]] && kill -0 "$STUB_PID" 2>/dev/null; then
    kill "$STUB_PID" 2>/dev/null || true
    wait "$STUB_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

wait_for_port() {
  local port="$1"
  for _ in $(seq 1 50); do
    if ! kill -0 "$STUB_PID" 2>/dev/null; then
      wait "$STUB_PID" || true
      echo "error: put_stub exited early; log:" >&2
      cat "$DEMO_DIR/stub.log" >&2 || true
      exit 1
    fi
    if "$PYTHON" -c "import socket; s=socket.create_connection(('127.0.0.1',$port),1); s.close()" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  echo "error: timed out waiting for put_stub on port $port" >&2
  cat "$DEMO_DIR/stub.log" >&2 || true
  exit 1
}

count_gets() {
  # put_stub logs: 127.0.0.1 - "GET /chunks/… HTTP/1.1" 200 -
  # grep -c prints 0 but exits 1 when there are no matches — ignore status.
  grep -c ' "GET ' "$DEMO_DIR/stub.log" 2>/dev/null || true
}

echo
echo "==> archive $SRC → $V1"
"$BIN" archive --store "$STORE" -o "$V1" "$SRC"

echo
echo "==> first extract → $OUT (expect wrote ${N_FILES} files)"
FIRST="$("$BIN" extract --store "$STORE" -o "$OUT" "$V1" 2>&1)"
echo "$FIRST"
if ! echo "$FIRST" | grep -Eq "wrote ${N_FILES} files|wrote=.*${N_FILES}"; then
  # 0.8.0-compat line: "extract: wrote … (N files, …)"
  if ! echo "$FIRST" | grep -Eq "\( *${N_FILES} files"; then
    echo "error: expected first extract to write ${N_FILES} files; got:" >&2
    echo "$FIRST" >&2
    exit 1
  fi
fi
cmp "$SRC/a.txt" "$OUT/a.txt"
cmp "$SRC/b.txt" "$OUT/b.txt"
echo "first extract: OK"

echo
echo "==> start put_stub on 127.0.0.1:${PORT} (root=$STORE)"
: >"$DEMO_DIR/stub.log"
"$PYTHON" "$ROOT/scripts/put_stub.py" --root "$STORE" --port "$PORT" \
  >"$DEMO_DIR/stub.log" 2>&1 &
STUB_PID=$!
wait_for_port "$PORT"
SOURCE="http://127.0.0.1:${PORT}"

echo
echo "==> second extract --skip-unchanged --force via $SOURCE (expect skipped=${N_FILES} wrote=0, zero GET)"
# Truncate after stub is up so startup noise (if any) is ignored.
: >"$DEMO_DIR/stub.log"
SKIP_ALL="$("$BIN" extract --source "$SOURCE" -o "$OUT" \
  --skip-unchanged --force "$V1" 2>&1)"
echo "$SKIP_ALL"
if ! echo "$SKIP_ALL" | grep -Eq "skipped=${N_FILES}"; then
  echo "error: expected skipped=${N_FILES} on unchanged tree; stderr above" >&2
  exit 1
fi
if ! echo "$SKIP_ALL" | grep -Eq 'wrote=0'; then
  echo "error: expected wrote=0 when all files skip; stderr above" >&2
  exit 1
fi
GETS="$(count_gets)"
# count_gets may print "0\n" from `|| echo 0` when grep finds nothing on some greps;
# normalize to integer
GETS="${GETS//$'\n'/}"
if [[ "$GETS" != "0" ]]; then
  echo "error: expected 0 chunk GET on full skip; got GETS=$GETS; stub log:" >&2
  cat "$DEMO_DIR/stub.log" >&2 || true
  exit 1
fi
echo "skip-all + zero GET: OK (GETS=$GETS)"

echo
echo "==> change one file (a.txt) + archive --seed → $V2"
echo 'hello-extract-v2' > "$SRC/a.txt"
"$BIN" archive --store "$STORE" -o "$V2" --seed "$V1" "$SRC"

echo
echo "==> extract --skip-unchanged --force v2 (expect skipped=$((N_FILES - 1)) wrote=1)"
# Reset GET counter; one file rewrite may GET that file's chunk(s).
: >"$DEMO_DIR/stub.log"
SKIP_ONE="$("$BIN" extract --source "$SOURCE" -o "$OUT" \
  --skip-unchanged --force "$V2" 2>&1)"
echo "$SKIP_ONE"
if ! echo "$SKIP_ONE" | grep -Eq "skipped=$((N_FILES - 1))"; then
  echo "error: expected skipped=$((N_FILES - 1)) after one-file change; stderr above" >&2
  exit 1
fi
if ! echo "$SKIP_ONE" | grep -Eq 'wrote=1'; then
  echo "error: expected wrote=1 after one-file change; stderr above" >&2
  exit 1
fi
cmp "$SRC/a.txt" "$OUT/a.txt"
cmp "$SRC/b.txt" "$OUT/b.txt"
# Unchanged b.txt must not have been fetched again — only a.txt chunks.
# (Allow ≥1 GET for the rewritten file; assert b's content still matches.)
echo "skip N-1 + wrote=1: OK"

echo
echo "==> dry-run --skip-unchanged on matching tree (expect would_skip=${N_FILES}, no writes)"
MTIME_BEFORE="$(stat -c '%Y' "$OUT/a.txt" 2>/dev/null || stat -f '%m' "$OUT/a.txt")"
: >"$DEMO_DIR/stub.log"
DRY="$("$BIN" extract --source "$SOURCE" -o "$OUT" \
  --skip-unchanged --dry-run "$V2" 2>&1)"
echo "$DRY"
if ! echo "$DRY" | grep -Eq 'dry-run:'; then
  echo "error: expected dry-run summary line; stderr above" >&2
  exit 1
fi
if ! echo "$DRY" | grep -Eq "would_skip=${N_FILES}"; then
  echo "error: expected would_skip=${N_FILES}; stderr above" >&2
  exit 1
fi
if ! echo "$DRY" | grep -Eq 'would_write=0'; then
  echo "error: expected would_write=0 on matching tree; stderr above" >&2
  exit 1
fi
DRY_GETS="$(count_gets)"
DRY_GETS="${DRY_GETS//$'\n'/}"
if [[ "$DRY_GETS" != "0" ]]; then
  echo "error: dry-run+skip must not GET chunks; got GETS=$DRY_GETS" >&2
  cat "$DEMO_DIR/stub.log" >&2 || true
  exit 1
fi
MTIME_AFTER="$(stat -c '%Y' "$OUT/a.txt" 2>/dev/null || stat -f '%m' "$OUT/a.txt")"
if [[ "$MTIME_BEFORE" != "$MTIME_AFTER" ]]; then
  echo "error: dry-run must not touch dest mtime" >&2
  exit 1
fi
echo "dry-run glance: OK (would_skip=${N_FILES}, GETS=0)"

echo
echo "==> compat: no --skip-unchanged on existing tree must fail (≡ 0.8.0)"
set +e
COMPAT="$("$BIN" extract --store "$STORE" -o "$OUT" "$V1" 2>&1)"
COMPAT_RC=$?
set -e
echo "$COMPAT"
if [[ "$COMPAT_RC" -eq 0 ]]; then
  echo "error: expected non-zero without --skip-unchanged / --force on existing files" >&2
  exit 1
fi
echo "0.8.0 conflict path: OK (exit=$COMPAT_RC)"

echo
echo "extract-skip demo OK"
echo "  store=$STORE"
echo "  v1=$V1"
echo "  v2=$V2"
echo "  out=$OUT"
echo "  source=$SOURCE"
