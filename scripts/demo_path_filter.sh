#!/usr/bin/env bash
# Phase 13 M5 path-filter smoke: archive --exclude + --format json →
# extract --path (non-prune) → pull --path (subset unique_chunks).
# Local only — no real internet (put_stub on 127.0.0.1).
# Usage: ./scripts/demo_path_filter.sh
# Requires: cargo, python3.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_PATH_FILTER_DEMO_DIR:-/tmp/cf-path-filter-demo}"
BIN="${CHUNKFORGE_BIN:-}"
FIXTURE="${CHUNKFORGE_PATH_FILTER_FIXTURE:-$ROOT/fixtures/hello.txt}"
PORT="${CHUNKFORGE_PATH_FILTER_PORT:-8773}"
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
MIRROR="$DEMO_DIR/mirror"
PULL_STORE="$DEMO_DIR/pull-store"
PULL_STORE_FULL="$DEMO_DIR/pull-store-full"
APP="$DEMO_DIR/app.cfdir"
mkdir -p \
  "$SRC/packages/foo" \
  "$SRC/packages/bar" \
  "$SRC/.git" \
  "$SRC/junk" \
  "$STORE" \
  "$MIRROR" \
  "$PULL_STORE" \
  "$PULL_STORE_FULL"

echo 'foo-payload-v1' > "$SRC/packages/foo/a.txt"
cp "$FIXTURE" "$SRC/packages/foo/b.txt"
echo 'bar-payload-v1' > "$SRC/packages/bar/b.txt"
echo 'git-junk' > "$SRC/.git/config"
echo 'junk-file' > "$SRC/junk/a.txt"

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

parse_json_ok() {
  # stdin → must be one JSON object; assert keys via optional python snippet arg
  local check="${1:-}"
  "$PYTHON" -c "
import json, sys
obj = json.load(sys.stdin)
assert isinstance(obj, dict), obj
$check
print('json parse: OK', obj)
"
}

echo
echo "==> A1. archive --exclude .git/ --exclude junk/ --format json → $APP"
ARCH_JSON="$("$BIN" archive --store "$STORE" -o "$APP" \
  --exclude '.git/' --exclude 'junk/' --format json "$SRC")"
echo "$ARCH_JSON"
echo "$ARCH_JSON" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('dry_run') is False, obj
assert isinstance(obj.get('files'), int) and obj['files'] >= 2, obj
assert 'dirs' in obj and 'chunks' in obj, obj
assert 'written' in obj and 'reused' in obj, obj
assert 'seed_reused_files' in obj and 'rechunked_files' in obj, obj
assert 'skipped_symlinks' in obj and 'skipped_special' in obj, obj
assert isinstance(obj.get('excluded'), int) and obj['excluded'] >= 1, obj
"
echo "archive --format json (excluded≥1): OK"

echo
echo "==> A2. extract full filtered listing → assert no junk / .git; has packages/*"
FULL_OUT="$DEMO_DIR/full-out"
"$BIN" extract --store "$STORE" -o "$FULL_OUT" "$APP" >/dev/null
test -f "$FULL_OUT/packages/foo/a.txt"
test -f "$FULL_OUT/packages/foo/b.txt"
test -f "$FULL_OUT/packages/bar/b.txt"
if [[ -e "$FULL_OUT/.git" ]] || [[ -e "$FULL_OUT/junk" ]]; then
  echo "error: listing still contains excluded junk/.git paths under $FULL_OUT" >&2
  find "$FULL_OUT" -type f >&2 || true
  exit 1
fi
# Paths must not appear as strings in the .cfdir either (defense in depth)
if grep -a -F 'junk/a.txt' "$APP" >/dev/null 2>&1; then
  echo "error: junk/a.txt still present in .cfdir bytes" >&2
  exit 1
fi
if grep -a -F '.git/config' "$APP" >/dev/null 2>&1; then
  echo "error: .git/config still present in .cfdir bytes" >&2
  exit 1
fi
echo "listing without junk/.git: OK"

echo
echo "==> B1. pre-seed extra dest file (prove non-prune)"
mkdir -p "$OUT"
echo 'keep-me-extra' > "$OUT/extra.txt"

echo
echo "==> B2. extract --path packages/foo --force → subset only; extra stays"
"$BIN" extract --store "$STORE" -o "$OUT" --path packages/foo --force "$APP" >/dev/null
test -f "$OUT/packages/foo/a.txt"
test -f "$OUT/packages/foo/b.txt"
cmp "$SRC/packages/foo/a.txt" "$OUT/packages/foo/a.txt"
test -f "$OUT/extra.txt"
cmp <(echo 'keep-me-extra') "$OUT/extra.txt"
if [[ -e "$OUT/packages/bar" ]]; then
  echo "error: extract --path packages/foo wrote packages/bar (should not)" >&2
  find "$OUT" -type f >&2 || true
  exit 1
fi
echo "extract --path (subset + non-prune): OK"

echo
echo "==> C1. start put_stub on 127.0.0.1:${PORT} (root=$MIRROR)"
: >"$DEMO_DIR/stub.log"
"$PYTHON" "$ROOT/scripts/put_stub.py" --root "$MIRROR" --port "$PORT" \
  >"$DEMO_DIR/stub.log" 2>&1 &
STUB_PID=$!
wait_for_port "$PORT"
DEST="http://127.0.0.1:${PORT}"

echo
echo "==> C2. push filtered listing chunks → $DEST"
PUSH_JSON="$("$BIN" push --store "$STORE" --dest "$DEST" --format json "$APP")"
echo "$PUSH_JSON"
echo "$PUSH_JSON" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('failed') == 0, obj
assert obj.get('uploaded', 0) >= 1, obj
assert isinstance(obj.get('unique_chunks'), int) and obj['unique_chunks'] >= 1, obj
"
FULL_UNIQUE="$("$PYTHON" -c "import json,sys; print(json.load(sys.stdin)['unique_chunks'])" <<<"$PUSH_JSON")"
echo "push: unique_chunks=$FULL_UNIQUE OK"

echo
echo "==> C3. pull --path packages/foo --format json (empty store) → subset unique_chunks"
PULL_JSON="$("$BIN" pull --store "$PULL_STORE" --source "$DEST" \
  --path packages/foo --format json "$APP")"
echo "$PULL_JSON"
echo "$PULL_JSON" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('dry_run') is False, obj
assert obj.get('failed') == 0, obj
assert obj.get('fetched', 0) >= 1, obj
assert 'unique_chunks' in obj, obj
assert isinstance(obj['unique_chunks'], int) and obj['unique_chunks'] >= 1, obj
assert obj['unique_chunks'] < $FULL_UNIQUE, (
    f\"expected unique_chunks < full ($FULL_UNIQUE), got {obj['unique_chunks']}\")
"
SUB_UNIQUE="$("$PYTHON" -c "import json,sys; print(json.load(sys.stdin)['unique_chunks'])" <<<"$PULL_JSON")"
echo "pull --path: unique_chunks=$SUB_UNIQUE (< $FULL_UNIQUE): OK"

echo
echo "==> C4. contrast: pull full reference set → unique_chunks ≡ push"
PULL_FULL_JSON="$("$BIN" pull --store "$PULL_STORE_FULL" --source "$DEST" \
  --format json "$APP")"
echo "$PULL_FULL_JSON"
echo "$PULL_FULL_JSON" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('failed') == 0, obj
assert obj.get('unique_chunks') == $FULL_UNIQUE, obj
"
echo "pull full unique_chunks=$FULL_UNIQUE: OK"

echo
echo "path-filter demo OK"
echo "  store=$STORE"
echo "  listing=$APP"
echo "  out=$OUT (extra.txt kept; packages/foo only)"
echo "  dest=$DEST"
echo "  pull-store=$PULL_STORE (subset unique_chunks=$SUB_UNIQUE)"
