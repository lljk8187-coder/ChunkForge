#!/usr/bin/env bash
# Phase 20 M4 smoke: --path-from feeds archive; doctor/verify --path /
# --path-from subset; default no flags ≡ 1.9 full/quiet; exclude-from +
# path-from combine; missing path-from → non-zero; .cfidx + path → non-zero;
# path-from ≠ gc --path (gc --help has no --path).
# Local only — no internet.
# Usage: ./scripts/demo_path_from_doctor_verify.sh
# Requires: cargo, python3.
# Env (optional):
#   CHUNKFORGE_BIN, CHUNKFORGE_PYTHON
#   CHUNKFORGE_P20_DEMO_DIR (default /tmp/cf-p20-path-from-demo)
#
# Gate: require check_compat_1_9.sh present + executable (Phase19-M7b
# lesson: do not leave a soft "not yet" note). Also require check_compat_1_8.
# Version gate expects chunkforge 1.10.0 (Phase20-M7).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_P20_DEMO_DIR:-/tmp/cf-p20-path-from-demo}"
BIN="${CHUNKFORGE_BIN:-}"
PYTHON="${CHUNKFORGE_PYTHON:-python3}"

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
mkdir -p \
  "$SRC/packages/foo" \
  "$SRC/packages/bar" \
  "$SRC/junk" \
  "$STORE"

echo 'foo-a' > "$SRC/packages/foo/a.txt"
echo 'foo-b' > "$SRC/packages/foo/b.txt"
echo 'bar-c' > "$SRC/packages/bar/c.txt"
echo 'junk-x' > "$SRC/junk/x.txt"
printf 'readme\n' > "$SRC/readme.txt"

# path-from include: packages/foo + packages/bar (OR)
printf '%s\n' \
  '# include prefixes (path-from)' \
  'packages/foo' \
  '' \
  'packages/bar' \
  > "$DEMO_DIR/include.txt"

# exclude-from: drop junk/ even if somehow included
printf '%s\n' \
  '# exclude patterns' \
  'junk/' \
  > "$DEMO_DIR/exclude.txt"

parse_json_ok() {
  local check="${1:-}"
  "$PYTHON" -c "
import json, sys
obj = json.load(sys.stdin)
assert isinstance(obj, dict), obj
$check
print('json parse: OK', obj)
"
}

assert_no_progress() {
  local label="$1"
  local errfile="$2"
  if grep -E '^progress:' "$errfile" >/dev/null 2>&1; then
    echo "error: $label stderr must stay quiet (no progress:); got:" >&2
    cat "$errfile" >&2
    exit 1
  fi
}

echo
echo "==> A. archive --path-from include.txt --exclude-from exclude.txt"
ARCH_JSON="$("$BIN" archive --store "$STORE" -o "$DEMO_DIR/tree.cfdir" \
  --path-from "$DEMO_DIR/include.txt" \
  --exclude-from "$DEMO_DIR/exclude.txt" \
  --format json "$SRC")"
echo "$ARCH_JSON"
echo "$ARCH_JSON" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('dry_run') is False, obj
assert isinstance(obj.get('files'), int) and obj['files'] >= 3, obj
assert isinstance(obj.get('excluded'), int) and obj['excluded'] >= 1, obj
"
# Full tree without filter for doctor/verify full vs subset
"$BIN" archive --store "$STORE" -o "$DEMO_DIR/full.cfdir" \
  --format json "$SRC" \
  >"$DEMO_DIR/archive-full.json"
# Blob index for .cfidx + path rejection
dd if=/dev/urandom of="$DEMO_DIR/blob.bin" bs=4096 count=1 status=none
"$BIN" make --store "$STORE" -o "$DEMO_DIR/blob.cfidx" \
  --chunk-size 4096:4096:4096 --format json "$DEMO_DIR/blob.bin" \
  >"$DEMO_DIR/make.json"
echo "A: path-from + exclude-from archive: OK"

echo
echo "==> B. doctor/verify --path / --path-from subset (counts may shrink)"
DOC_FULL="$("$BIN" doctor --store "$STORE" --format json "$DEMO_DIR/full.cfdir")"
echo "$DOC_FULL"
echo "$DOC_FULL" | parse_json_ok "
assert obj.get('ok') is True, obj
assert isinstance(obj.get('checked'), int) and obj['checked'] >= 1, obj
assert obj.get('missing') == 0, obj
"
FULL_CHECKED="$("$PYTHON" -c "import json,sys; print(json.load(sys.stdin)['checked'])" <<<"$DOC_FULL")"

DOC_SUB="$("$BIN" doctor --store "$STORE" --path packages/foo --format json \
  "$DEMO_DIR/full.cfdir")"
echo "$DOC_SUB"
echo "$DOC_SUB" | parse_json_ok "
assert obj.get('ok') is True, obj
assert isinstance(obj.get('checked'), int) and obj['checked'] >= 1, obj
assert obj['checked'] < $FULL_CHECKED, (
    f\"expected checked < full ($FULL_CHECKED), got {obj['checked']}\")
assert obj.get('missing') == 0, obj
assert 'listings' in obj and 'deep' in obj, obj
"
echo "doctor --path subset (checked < full=$FULL_CHECKED): OK"

DOC_FROM="$("$BIN" doctor --store "$STORE" \
  --path-from "$DEMO_DIR/include.txt" --format json \
  "$DEMO_DIR/full.cfdir")"
echo "$DOC_FROM"
echo "$DOC_FROM" | parse_json_ok "
assert obj.get('ok') is True, obj
assert isinstance(obj.get('checked'), int) and obj['checked'] >= 1, obj
assert obj['checked'] <= $FULL_CHECKED, obj
assert obj.get('missing') == 0, obj
"
echo "doctor --path-from: OK"

VER_FULL="$("$BIN" verify --store "$STORE" --format json "$DEMO_DIR/full.cfdir")"
echo "$VER_FULL"
echo "$VER_FULL" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('kind') == 'cfdir', obj
assert isinstance(obj.get('files'), int) and obj['files'] >= 1, obj
assert isinstance(obj.get('chunks'), int) and obj['chunks'] >= 1, obj
"
FULL_FILES="$("$PYTHON" -c "import json,sys; print(json.load(sys.stdin)['files'])" <<<"$VER_FULL")"

VER_SUB="$("$BIN" verify --store "$STORE" --path packages/foo --format json \
  "$DEMO_DIR/full.cfdir")"
echo "$VER_SUB"
echo "$VER_SUB" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('kind') == 'cfdir', obj
assert isinstance(obj.get('files'), int) and obj['files'] >= 1, obj
assert obj['files'] < $FULL_FILES, (
    f\"expected files < full ($FULL_FILES), got {obj['files']}\")
"
echo "verify --path subset (files < full=$FULL_FILES): OK"

VER_FROM="$("$BIN" verify --store "$STORE" \
  --path-from "$DEMO_DIR/include.txt" --format json \
  "$DEMO_DIR/full.cfdir")"
echo "$VER_FROM"
echo "$VER_FROM" | parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('kind') == 'cfdir', obj
assert isinstance(obj.get('files'), int) and obj['files'] >= 1, obj
assert obj['files'] <= $FULL_FILES, obj
"
echo "verify --path-from: OK"
echo "B: doctor/verify path scope: OK"

echo
echo "==> C. default no path flags ≡ 1.9 full + quiet"
"$BIN" doctor --store "$STORE" --format json "$DEMO_DIR/full.cfdir" \
  >"$DEMO_DIR/doc-quiet.json" \
  2>"$DEMO_DIR/doc-quiet.err"
assert_no_progress "default doctor" "$DEMO_DIR/doc-quiet.err"
parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('checked') == $FULL_CHECKED, obj
" < "$DEMO_DIR/doc-quiet.json"

"$BIN" verify --store "$STORE" --format json "$DEMO_DIR/full.cfdir" \
  >"$DEMO_DIR/ver-quiet.json" \
  2>"$DEMO_DIR/ver-quiet.err"
assert_no_progress "default verify" "$DEMO_DIR/ver-quiet.err"
parse_json_ok "
assert obj.get('ok') is True, obj
assert obj.get('files') == $FULL_FILES, obj
" < "$DEMO_DIR/ver-quiet.json"
echo "C: no flags ≡ full + quiet: OK"

echo
echo "==> D. exclude-from + path-from combined on doctor"
DOC_COMBO="$("$BIN" doctor --store "$STORE" \
  --path-from "$DEMO_DIR/include.txt" \
  --exclude-from "$DEMO_DIR/exclude.txt" \
  --format json "$DEMO_DIR/full.cfdir")"
echo "$DOC_COMBO"
echo "$DOC_COMBO" | parse_json_ok "
assert obj.get('ok') is True, obj
assert isinstance(obj.get('checked'), int) and obj['checked'] >= 1, obj
assert obj.get('missing') == 0, obj
"
echo "D: path-from + exclude-from on doctor: OK"

echo
echo "==> E. missing path-from file → non-zero"
set +e
"$BIN" archive --store "$STORE" -o "$DEMO_DIR/missing.cfdir" \
  --path-from "$DEMO_DIR/does-not-exist.txt" \
  --format json "$SRC" \
  >"$DEMO_DIR/missing-arch.out" \
  2>"$DEMO_DIR/missing-arch.err"
RC_MISS=$?
set -e
if [[ "$RC_MISS" -eq 0 ]]; then
  echo "error: missing path-from file must be non-zero" >&2
  cat "$DEMO_DIR/missing-arch.out" "$DEMO_DIR/missing-arch.err" >&2
  exit 1
fi
echo "missing path-from exit=$RC_MISS (non-zero): OK"

set +e
"$BIN" doctor --store "$STORE" \
  --path-from "$DEMO_DIR/does-not-exist.txt" \
  --format json "$DEMO_DIR/full.cfdir" \
  >"$DEMO_DIR/missing-doc.out" \
  2>"$DEMO_DIR/missing-doc.err"
RC_DOC_MISS=$?
set -e
if [[ "$RC_DOC_MISS" -eq 0 ]]; then
  echo "error: doctor missing path-from must be non-zero" >&2
  cat "$DEMO_DIR/missing-doc.out" "$DEMO_DIR/missing-doc.err" >&2
  exit 1
fi
echo "doctor missing path-from exit=$RC_DOC_MISS: OK"
echo "E: missing path-from → non-zero: OK"

echo
echo "==> F. .cfidx + path flag → non-zero"
set +e
"$BIN" doctor --store "$STORE" --path packages/foo --format json \
  "$DEMO_DIR/blob.cfidx" \
  >"$DEMO_DIR/cfidx-doc.out" \
  2>"$DEMO_DIR/cfidx-doc.err"
RC_CFIDX=$?
set -e
if [[ "$RC_CFIDX" -eq 0 ]]; then
  echo "error: doctor .cfidx + --path must be non-zero" >&2
  cat "$DEMO_DIR/cfidx-doc.out" "$DEMO_DIR/cfidx-doc.err" >&2
  exit 1
fi
echo "doctor .cfidx + --path exit=$RC_CFIDX: OK"

set +e
"$BIN" verify --store "$STORE" --path-from "$DEMO_DIR/include.txt" \
  --format json "$DEMO_DIR/blob.cfidx" \
  >"$DEMO_DIR/cfidx-ver.out" \
  2>"$DEMO_DIR/cfidx-ver.err"
RC_CFIDX_V=$?
set -e
if [[ "$RC_CFIDX_V" -eq 0 ]]; then
  echo "error: verify .cfidx + --path-from must be non-zero" >&2
  cat "$DEMO_DIR/cfidx-ver.out" "$DEMO_DIR/cfidx-ver.err" >&2
  exit 1
fi
echo "verify .cfidx + --path-from exit=$RC_CFIDX_V: OK"
echo "F: .cfidx + path → non-zero: OK"

echo
echo "==> G. path-from ≠ gc --path (gc --help has no --path)"
GC_HELP="$("$BIN" gc --help)"
if grep -E -- '--path\b' <<<"$GC_HELP" >/dev/null; then
  echo "error: gc must NOT expose --path (path-from ≠ gc-path hard ban)" >&2
  echo "$GC_HELP" >&2
  exit 1
fi
if grep -F -- '--path-from' <<<"$GC_HELP" >/dev/null; then
  echo "error: gc must NOT expose --path-from" >&2
  exit 1
fi
echo "gc --help has no --path / --path-from: OK"
echo "G: path-from ≠ gc-path: OK"

echo
echo "==> H. help / version / compat surfaces"
for cmd in archive extract push pull diff doctor verify; do
  if ! "$BIN" "$cmd" --help | grep -F -- '--path-from' >/dev/null; then
    echo "error: $cmd --help missing --path-from" >&2
    exit 1
  fi
done
for cmd in doctor verify; do
  if ! "$BIN" "$cmd" --help | grep -E -- '--path\b' >/dev/null; then
    echo "error: $cmd --help missing --path" >&2
    exit 1
  fi
done
VER="$("$BIN" --version)"
echo "version: $VER"
if ! grep -F '1.10.0' <<<"$VER" >/dev/null; then
  echo "error: expected chunkforge 1.10.0; got $VER" >&2
  exit 1
fi
COMPAT18="$ROOT/scripts/check_compat_1_8.sh"
if [[ ! -f "$COMPAT18" ]]; then
  echo "error: check_compat_1_8.sh must exist" >&2
  exit 1
fi
if [[ ! -x "$COMPAT18" ]]; then
  echo "error: check_compat_1_8.sh must be executable" >&2
  exit 1
fi
COMPAT19="$ROOT/scripts/check_compat_1_9.sh"
if [[ ! -f "$COMPAT19" ]]; then
  echo "error: check_compat_1_9.sh must exist" >&2
  exit 1
fi
if [[ ! -x "$COMPAT19" ]]; then
  echo "error: check_compat_1_9.sh must be executable" >&2
  exit 1
fi
echo "H: help / version 1.10.0 / compat_1_8 / compat_1_9: OK"

echo
echo "demo_path_from_doctor_verify: ALL OK"
