#!/usr/bin/env bash
# Phase 9 P1 / O1: loose-layout HTTP push/pull wall-clock baseline (local stub).
# Usage: ./scripts/bench_loose_http.sh [--also-jobs-4] [--chunks N] [--chunk-bytes B]
# Env:   CHUNKFORGE_BENCH_DIR, CHUNKFORGE_BENCH_CHUNKS, CHUNKFORGE_BENCH_CHUNK_BYTES,
#        CHUNKFORGE_BENCH_PORT, CHUNKFORGE_BIN, CHUNKFORGE_PYTHON
# Does NOT change product defaults (--jobs 1, --http-retries 0).
# Pack is NOT implemented; this script only measures loose one-object-per-chunk I/O.
# stdout: one machine-parseable line per timed op; stderr: progress.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEMO_DIR="${CHUNKFORGE_BENCH_DIR:-/tmp/cf-bench-loose-http}"
BIN="${CHUNKFORGE_BIN:-}"
PORT="${CHUNKFORGE_BENCH_PORT:-8770}"
PYTHON="${CHUNKFORGE_PYTHON:-python3}"
CHUNKS="${CHUNKFORGE_BENCH_CHUNKS:-64}"
CHUNK_BYTES="${CHUNKFORGE_BENCH_CHUNK_BYTES:-65536}"
ALSO_JOBS_4=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --also-jobs-4) ALSO_JOBS_4=1; shift ;;
    --chunks)
      CHUNKS="${2:?--chunks needs N}"; shift 2 ;;
    --chunk-bytes)
      CHUNK_BYTES="${2:?--chunk-bytes needs B}"; shift 2 ;;
    -h|--help)
      sed -n '2,10p' "$0" | sed 's/^# //;s/^#//'
      exit 0
      ;;
    *)
      echo "error: unknown arg: $1" >&2
      exit 2
      ;;
  esac
done

if ! [[ "$CHUNKS" =~ ^[1-9][0-9]*$ ]]; then
  echo "error: --chunks must be a positive integer (got: $CHUNKS)" >&2
  exit 2
fi
if ! [[ "$CHUNK_BYTES" =~ ^[1-9][0-9]*$ ]]; then
  echo "error: --chunk-bytes must be a positive integer (got: $CHUNK_BYTES)" >&2
  exit 2
fi
# FastCDC requires even sizes and min<=avg<=max; keep B even.
if (( CHUNK_BYTES % 2 != 0 )); then
  echo "error: --chunk-bytes must be even (FastCDC constraint)" >&2
  exit 2
fi

if ! command -v "$PYTHON" >/dev/null 2>&1; then
  echo "error: $PYTHON not on PATH" >&2
  exit 1
fi

echo "==> building chunkforge" >&2
cargo build -p chunkforge-cli --quiet
if [[ -z "$BIN" ]]; then
  BIN="$ROOT/target/debug/chunkforge"
fi

rm -rf "$DEMO_DIR"
STORE_PUSH="$DEMO_DIR/store-push"
STORE_PULL="$DEMO_DIR/store-pull"
MIRROR="$DEMO_DIR/mirror"
IDX="$DEMO_DIR/bench.cfidx"
PAYLOAD="$DEMO_DIR/payload.bin"
mkdir -p "$STORE_PUSH" "$STORE_PULL" "$MIRROR"

STUB_PID=""
cleanup() {
  if [[ -n "$STUB_PID" ]] && kill -0 "$STUB_PID" 2>/dev/null; then
    kill "$STUB_PID" 2>/dev/null || true
    wait "$STUB_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

TOTAL_BYTES=$((CHUNKS * CHUNK_BYTES))
echo "==> generating ${CHUNKS}×${CHUNK_BYTES}B payload (${TOTAL_BYTES} bytes)" >&2
"$PYTHON" - "$PAYLOAD" "$CHUNKS" "$CHUNK_BYTES" <<'PY'
import hashlib, pathlib, sys
path = pathlib.Path(sys.argv[1])
n = int(sys.argv[2])
b = int(sys.argv[3])
seed = b"chunkforge-bench-loose-http-v1"
with path.open("wb") as f:
    for i in range(n):
        # Unique per-chunk block so FastCDC does not collapse identical chunks.
        h = hashlib.sha256(seed + i.to_bytes(8, "little")).digest()
        block = (h * ((b // len(h)) + 1))[:b]
        f.write(block)
print(f"wrote {path} ({path.stat().st_size} bytes)", file=sys.stderr)
PY

CHUNK_SPEC="${CHUNK_BYTES}:${CHUNK_BYTES}:${CHUNK_BYTES}"
echo "==> make --chunk-size ${CHUNK_SPEC}" >&2
"$BIN" make --store "$STORE_PUSH" -o "$IDX" --chunk-size "$CHUNK_SPEC" "$PAYLOAD" >&2

ACTUAL_CHUNKS="$("$PYTHON" - "$STORE_PUSH" <<'PY'
import pathlib, sys
root = pathlib.Path(sys.argv[1]) / "chunks"
ids = list(root.glob("*/*.cnk")) if root.is_dir() else []
print(len(ids))
PY
)"
if [[ "$ACTUAL_CHUNKS" -lt 1 ]]; then
  echo "error: expected ≥1 chunk in store after make; got $ACTUAL_CHUNKS" >&2
  exit 1
fi
BYTES_ACTUAL=$((ACTUAL_CHUNKS * CHUNK_BYTES))
echo "==> store has ${ACTUAL_CHUNKS} chunks (~${BYTES_ACTUAL} bytes plaintext)" >&2

echo "==> start put_stub on 127.0.0.1:${PORT}" >&2
"$PYTHON" "$ROOT/scripts/put_stub.py" --root "$MIRROR" --port "$PORT" \
  >"$DEMO_DIR/stub.log" 2>&1 &
STUB_PID=$!

for _ in $(seq 1 50); do
  if ! kill -0 "$STUB_PID" 2>/dev/null; then
    wait "$STUB_PID" || true
    echo "error: put_stub exited early; log:" >&2
    cat "$DEMO_DIR/stub.log" >&2 || true
    exit 1
  fi
  if "$PYTHON" -c "import socket; s=socket.create_connection(('127.0.0.1',$PORT),1); s.close()" 2>/dev/null; then
    break
  fi
  sleep 0.1
done
if ! "$PYTHON" -c "import socket; s=socket.create_connection(('127.0.0.1',$PORT),1); s.close()" 2>/dev/null; then
  echo "error: timed out waiting for put_stub on port $PORT" >&2
  cat "$DEMO_DIR/stub.log" >&2 || true
  exit 1
fi

DEST="http://127.0.0.1:${PORT}"

# wall_s via python for portable subsecond timing; prints one parseable line.
run_timed() {
  local op="$1"
  local jobs="$2"
  shift 2
  # remaining args = command
  "$PYTHON" - "$op" "$ACTUAL_CHUNKS" "$BYTES_ACTUAL" "$jobs" "$@" <<'PY'
import subprocess, sys, time
op, chunks, nbytes, jobs = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
cmd = sys.argv[5:]
t0 = time.perf_counter()
proc = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
wall = time.perf_counter() - t0
sys.stderr.write(proc.stdout)
if proc.returncode != 0:
    sys.stderr.write(f"error: {op} jobs={jobs} exited {proc.returncode}\n")
    sys.exit(proc.returncode or 1)
cps = (chunks / wall) if wall > 0 else 0.0
# Machine-parseable one-liner (stdout only).
print(
    f"bench_loose_http op={op} chunks={chunks} bytes={nbytes} "
    f"jobs={jobs} wall_s={wall:.3f} chunk_per_s={cps:.2f} retries=0"
)
sys.exit(0)
PY
}

bench_pair() {
  local jobs="$1"
  echo >&2
  echo "==> push --jobs ${jobs} (http-retries default 0)" >&2
  # Fresh mirror for each jobs pass so upload count == all chunks.
  rm -rf "$MIRROR"
  mkdir -p "$MIRROR"
  run_timed push "$jobs" \
    "$BIN" push --store "$STORE_PUSH" --dest "$DEST" --jobs "$jobs" "$IDX"

  echo >&2
  echo "==> pull --jobs ${jobs} into fresh store" >&2
  local pull_store="$DEMO_DIR/store-pull-j${jobs}"
  rm -rf "$pull_store"
  mkdir -p "$pull_store"
  run_timed pull "$jobs" \
    "$BIN" pull --store "$pull_store" --source "$DEST" --jobs "$jobs" "$IDX"
}

bench_pair 1

if [[ "$ALSO_JOBS_4" -eq 1 ]]; then
  bench_pair 4
fi

echo >&2
echo "bench_loose_http OK (pack NOT implemented; defaults unchanged)" >&2
echo "  see docs/perf.md for pack promotion checklist" >&2
