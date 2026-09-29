# ChunkForge convenience targets (Phase 1).
.PHONY: build test gen-large demo-dedup demo-dedup-small demo-mount clean-gen

CHUNKFORGE_GEN_MIB ?= 64
CHUNKFORGE_GEN_DIR ?= fixtures/gen

build:
	cargo build -p chunkforge-cli

# Linux FUSE smoke (needs fuse3 + /dev/fuse).
demo-mount: build
	./scripts/demo_mount.sh

test:
	cargo test --workspace

# Offline ≥64MiB (default) deterministic fixtures; binaries are gitignored.
gen-large:
	./scripts/gen_large.sh "$(CHUNKFORGE_GEN_DIR)" "$(CHUNKFORGE_GEN_MIB)"

# Full incremental dedup demo (builds CLI, gen_large, make×3 with stats).
demo-dedup: build
	./scripts/demo_dedup.sh "$(CHUNKFORGE_GEN_MIB)"

# Faster smoke: 4MiB fixtures (still shows remake + mid-file reuse).
demo-dedup-small:
	$(MAKE) demo-dedup CHUNKFORGE_GEN_MIB=4

clean-gen:
	rm -rf fixtures/gen
