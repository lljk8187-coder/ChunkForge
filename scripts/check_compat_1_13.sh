#!/usr/bin/env bash
# Phase 24 / 1.13+ compat gate (G6 / M5 / V4): runs the 1.12 gate, then asserts
# 1.14 additive `filter` subcommand still appears in top-level / filter --help.
# Thin functional: full.cfdir → filter --path … -o sub.cfdir → verify green
# (tiny local tree; optionally with --symlinks record so Symlink keep is
# covered lightly). Does **not** assert absolute throughput / SLA numbers and
# does **not** force-run long demos.
# Local only — no real internet. Keeps check_compat_1_0.sh … 1_12
# independently runnable.
# Usage: ./scripts/check_compat_1_13.sh
# Requires: cargo, python3 (demos via 1_0…1_12), bash.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Phase 24 / 1.13+ compat gate"
echo "==> building chunkforge-cli (shared binary for 1_12 + filter assertions)"
cargo build -p chunkforge-cli --quiet
BIN="${CHUNKFORGE_BIN:-$ROOT/target/debug/chunkforge}"
if [[ ! -x "$BIN" ]]; then
  echo "error: binary not found: $BIN" >&2
  exit 1
fi
export CHUNKFORGE_BIN="$BIN"

echo
echo "==> running check_compat_1_12.sh (CHUNKFORGE_BIN=$CHUNKFORGE_BIN)"
bash "$ROOT/scripts/check_compat_1_12.sh"

echo
echo "==> 1.14 filter subcommand assertions (help text)"

TOP_HELP="$("$BIN" --help)"
if ! grep -Eiq '(^|[[:space:]])filter([[:space:]]|$)' <<<"$TOP_HELP"; then
  echo "error: top-level --help missing filter subcommand" >&2
  echo "$TOP_HELP" >&2
  exit 1
fi
echo "  top-level: filter OK"

FILT_HELP="$("$BIN" filter --help)"
if ! grep -Eiq 'Persist a path-scoped subset|filter_dir_archive|\.cfdir' <<<"$FILT_HELP"; then
  echo "error: filter --help missing expected filter narrative" >&2
  echo "$FILT_HELP" >&2
  exit 1
fi
# path 四件套 + -o
for flag in --path --path-from --exclude --exclude-from --output; do
  if ! grep -Fq -- "$flag" <<<"$FILT_HELP"; then
    echo "error: filter --help missing $flag" >&2
    echo "$FILT_HELP" >&2
    exit 1
  fi
done
echo "  filter: --help + path 四件套 + --output OK"

echo
echo "==> thin functional: archive --symlinks record → filter --path → verify green"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/cf-compat-1_13.XXXXXX")"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT

mkdir -p "$TMP/src/pkgs/foo" "$TMP/src/pkgs/bar" "$TMP/store"
echo 'hello-foo' > "$TMP/src/pkgs/foo/a.txt"
ln -s a.txt "$TMP/src/pkgs/foo/link.txt"
echo 'hello-bar' > "$TMP/src/pkgs/bar/b.txt"

"$BIN" store create --store "$TMP/store"
"$BIN" archive --store "$TMP/store" -o "$TMP/full.cfdir" \
  --symlinks record "$TMP/src" >/dev/null

"$BIN" filter --path pkgs/foo -o "$TMP/sub.cfdir" "$TMP/full.cfdir" >/dev/null

set +e
"$BIN" verify --store "$TMP/store" "$TMP/sub.cfdir" \
  >"$TMP/verify.out" 2>"$TMP/verify.err"
RC_VER=$?
set -e
if [[ "$RC_VER" -ne 0 ]]; then
  echo "error: verify --store … sub.cfdir must exit 0 (got $RC_VER)" >&2
  cat "$TMP/verify.err" >&2 || true
  cat "$TMP/verify.out" >&2 || true
  exit 1
fi
echo "  filter --path → verify green OK"

# Light Symlink-keep coverage: subset listing should still decode as v2 when
# the kept path includes a recorded Symlink (link.txt under pkgs/foo).
if command -v python3 >/dev/null 2>&1; then
  python3 - "$TMP/sub.cfdir" <<'PY'
import struct, sys
path = sys.argv[1]
data = open(path, "rb").read()
assert data[:8] == b"CFDIR\0\0\1", data[:8]
fmt_ver = struct.unpack_from("<H", data, 8)[0]
assert fmt_ver == 2, ("expected format_version=2 for Symlink-keeping subset", fmt_ver)
entry_count = struct.unpack_from("<Q", data, 16)[0]
off = 24
syms = 0
for _ in range(entry_count):
    path_len = struct.unpack_from("<H", data, off)[0]; off += 2
    off += path_len
    kind = data[off]; off += 1
    if kind == 1:  # File
        off += 4 + 8 + 8 + 32
        chunk_count = struct.unpack_from("<Q", data, off)[0]; off += 8
        off += chunk_count * 40
    elif kind == 2:  # Dir
        off += 4
    elif kind == 3:  # Symlink
        syms += 1
        off += 4
        tlen = struct.unpack_from("<H", data, off)[0]; off += 2
        off += tlen
    else:
        raise SystemExit(f"unknown kind {kind}")
assert syms >= 1, ("expected ≥1 Symlink kept under --path pkgs/foo", syms)
print("  Symlink keep (format_version=2, symlinks≥1) OK")
PY
else
  echo "  Symlink keep → skipped (python3 unavailable)"
fi

echo
echo "==> thin non-goal re-asserts (no gc --path / no write-mount / no extract --delete/prune / no default record symlink / no abs perf SLA)"

# gc has no --path / --path-from (hard ban)
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

# Write-mount thin check: mount stays RO; help must not advertise write/rw flags
MOUNT_HELP="$("$BIN" mount --help)"
if grep -Eiq -- '(^|[[:space:]])--(write|writable|rw)([[:space:]=]|$)' <<<"$MOUNT_HELP"; then
  echo "error: mount --help advertises write/writable/--rw (write mount is forbidden)" >&2
  exit 1
fi
echo "  mount: no --write/--writable/--rw OK"

# extract has no --delete / prune
EXTRACT_HELP="$("$BIN" extract --help)"
if grep -Eiq -- '(^|[[:space:]])--delete([[:space:]=]|$)' <<<"$EXTRACT_HELP"; then
  echo "error: extract --help advertises --delete (prune is forbidden)" >&2
  exit 1
fi
echo "  extract: no --delete / prune OK"

# Default record symlink forbidden: archive + diff --tree still default skip
ARCH_HELP="$("$BIN" archive --help)"
if grep -Eiq '\[default:[[:space:]]*record\]' <<<"$ARCH_HELP"; then
  echo "error: archive --help claims default record (forbidden; default must be skip)" >&2
  exit 1
fi
if ! grep -Eiq '\[default:[[:space:]]*skip\]' <<<"$ARCH_HELP"; then
  if ! grep -Eiq 'default.*skip|skip \(default' <<<"$ARCH_HELP"; then
    echo "error: archive --help missing default skip narrative" >&2
    exit 1
  fi
fi
echo "  archive: default skip (no default record) OK"

DIFF_HELP="$("$BIN" diff --help)"
if grep -Eiq '\[default:[[:space:]]*record\]' <<<"$DIFF_HELP"; then
  echo "error: diff --help claims default record (forbidden)" >&2
  exit 1
fi
if ! grep -Eiq '\[default:[[:space:]]*skip\]' <<<"$DIFF_HELP"; then
  if ! grep -Eiq 'default.*skip|skip \(default' <<<"$DIFF_HELP"; then
    echo "error: diff --help missing default skip narrative" >&2
    exit 1
  fi
fi
echo "  diff: default skip (no default record) OK"

# Absolute perf is intentionally NOT asserted here (no wall_s / chunk_per_s SLA).
echo "  (skip absolute perf SLA — by design) OK"

echo
echo "==> demo_filter_listing presence (G5/O4; do not force-run long demo)"
if [[ ! -f "$ROOT/scripts/demo_filter_listing.sh" ]]; then
  echo "error: scripts/demo_filter_listing.sh missing" >&2
  exit 1
fi
if [[ ! -x "$ROOT/scripts/demo_filter_listing.sh" ]]; then
  echo "error: scripts/demo_filter_listing.sh not executable" >&2
  exit 1
fi
echo "  demo_filter_listing.sh present + executable OK"

echo
echo "OK: check_compat_1_13"
