#!/usr/bin/env bash
# Generate deterministic large fixtures for ChunkForge dedup demos (offline).
# Usage: ./scripts/gen_large.sh [out_dir] [size_mib]
# Env:   CHUNKFORGE_GEN_MIB — override size in MiB (default 64)
# Depends on: python3, bash. No curl/wget.
set -euo pipefail

OUT="${1:-fixtures/gen}"
SIZE_MIB="${2:-${CHUNKFORGE_GEN_MIB:-64}}"
SEED="${CHUNKFORGE_GEN_SEED:-chunkforge-phase1-v1}"

if ! [[ "$SIZE_MIB" =~ ^[1-9][0-9]*$ ]]; then
  echo "error: size_mib must be a positive integer (got: $SIZE_MIB)" >&2
  exit 1
fi

mkdir -p "$OUT"

python3 - "$OUT" "$SEED" "$SIZE_MIB" <<'PY'
import hashlib
import pathlib
import shutil
import sys

out = pathlib.Path(sys.argv[1])
seed = sys.argv[2].encode()
size_mib = int(sys.argv[3])
mib = 1024 * 1024

path = out / f"large-{size_mib}m.bin"
with path.open("wb") as f:
    # Each 1MiB block is derived from seed||counter (SHA-256 digest tiled).
    for i in range(size_mib):
        digest = hashlib.sha256(seed + i.to_bytes(8, "little")).digest()
        block = (digest * (mib // len(digest)))[:mib]
        f.write(block)

mut = out / f"large-{size_mib}m-mut.bin"
shutil.copyfile(path, mut)
# Mid-file mutation: rewrite 4KiB at the halfway point.
with mut.open("r+b") as f:
    f.seek((size_mib // 2) * mib)
    f.write(b"X" * 4096)

print(f"wrote {path} ({path.stat().st_size} bytes)")
print(f"wrote {mut} (mutated 4KiB at offset {(size_mib // 2) * mib})")
PY
