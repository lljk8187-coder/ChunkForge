# Loose-layout HTTP performance baseline (Phase 9 P1 / O1)

This document records **how to measure** local loose-chunk HTTP push/pull
throughput. It exists so a future Phase can decide whether to introduce a
**packfile** (multi-chunk single object) using evidence — **not** to implement pack
now.

> **Pack is not implemented in this phase.** The on-disk / HTTP layout remains
> one object per chunk (`chunks/<2hex>/<62hex>.cnk`). Product defaults stay
> `--jobs 1` and `--http-retries 0`.

> **1.2.0 / Phase 12 still does not implement pack.** Evidence and the
> promotion checklist below continue to govern any future dual-mode layout;
> Phase 12 delivered local maint ops (`gc --jobs`, gc/scrub JSON) only.
>
> **1.3.0 / Phase 13 still does not implement pack.** Path-filter work
> (`archive`/`extract`/`pull --path`/`--exclude`, `archive --format json`)
> does not change the loose `.cnk` layout. Local stub bench
> (`scripts/bench_loose_http.sh`, 64×64KiB, jobs=1) remains near-zero RTT
> evidence — promotion checklist condition 1 (high RTT dominant) still unmet.
>
> **1.4.0 / Phase 14 still does not implement pack.** `push --path` /
> `store stats` / `--exclude-from` keep the loose `.cnk` layout and do not
> change product defaults (`jobs=1`, `retries=0`). Promotion checklist
> condition 1 remains unmet on local stub evidence; pack stays deferred.
>
> **1.5.0 / Phase 15 still does not implement pack.** Cache soft budget
> (`--cache-max-bytes` refuse-fill) and `make`/`cat --format json` do not
> change the loose `.cnk` layout or product defaults (`jobs=1`, `retries=0`).
> Promotion checklist condition 1 (high RTT dominant) remains unmet on local
> stub evidence; pack stays deferred.
>
> **1.6.0 / Phase 16 still does not implement pack.** `--fallback`, human
> byte suffixes on `--cache-max-bytes`, and `store stats`
> `bytes_plaintext`/`--decode` keep the loose `.cnk` layout and do not change
> product defaults (`jobs=1`, `retries=0`). Promotion checklist condition 1
> remains unmet; pack stays deferred.

> **1.7.0 / Phase 17 still does not implement pack.** CLI create-time
> `--compression none|zstd` (disk encoding only) and `archive`/`extract`/`make
> --progress` keep the loose `.cnk` layout and do not change product defaults
> (`jobs=1`, `retries=0`, create compression **none**, progress **off**).
> Local stub evidence (near-zero RTT) remains insufficient to promote pack;
> promotion checklist condition 1 stays unmet.
>
> **Phase18 / 1.8.0 still does not implement pack.** `pull --verify`,
> `--cache-stats` / ops-json `cache_*`, and `cat`/`verify --progress` are
> additive opt-in only; they keep the loose `.cnk` layout and do not change
> product defaults (`jobs=1`, `retries=0`, create compression **none**,
> progress **off**, no pull `--verify`, no `--cache-stats`). Promotion
> checklist condition 1 stays unmet on local stub evidence; pack stays
> deferred.

## What we measure

| Metric | Meaning |
|---|---|
| Wall-clock (s) | Elapsed time for one `push` or `pull` of N unique chunks |
| chunk/s | `chunks / wall_s` |
| Bytes | Approximate plaintext payload (`chunks × chunk_bytes`) |
| jobs | CLI `--jobs` for that run only (default product value remains **1**) |
| retries | Always **0** in the baseline script (no injected RTT / fail-transient) |

Optional human RTT injection (`tc`, stub sleep) is **out of band** — note it in
your results table if you use it; the stock script does not change product code.

## Quick run

```bash
# From repo root (builds debug CLI, starts scripts/put_stub.py, measures push+pull)
bash scripts/bench_loose_http.sh

# Optional knobs (env or flags — see script header):
#   CHUNKFORGE_BENCH_CHUNKS=64 CHUNKFORGE_BENCH_CHUNK_BYTES=65536
#   CHUNKFORGE_BENCH_JOBS=1          # also runs jobs=4 when --also-jobs-4
bash scripts/bench_loose_http.sh --also-jobs-4
```

Exit **0** on success. The script prints human progress on stderr and **one
machine-parseable summary line per timed op** on stdout, for example:

```text
bench_loose_http op=push chunks=64 bytes=4194304 jobs=1 wall_s=0.412 chunk_per_s=155.34 retries=0
bench_loose_http op=pull chunks=64 bytes=4194304 jobs=1 wall_s=0.388 chunk_per_s=164.95 retries=0
```

Defaults intentionally match product: `--jobs 1`, `--http-retries 0`. Passing
`--jobs 4` to the CLI **inside the script** does not change the installed
binary’s clap defaults.

## Method (manual equivalent)

1. Build: `cargo build -p chunkforge-cli`
2. Create a deterministic payload of `N × B` bytes and `make` with
   `--chunk-size B:B:B` so FastCDC yields ~N fixed-size chunks.
3. Start `scripts/put_stub.py --root <mirror> --port <port>` (local only).
4. Time `chunkforge push --store … --dest http://127.0.0.1:<port> --jobs 1 …`
5. Empty (or use a fresh) pull store; time
   `chunkforge pull --store … --source http://127.0.0.1:<port> --jobs 1 …`
6. Optionally repeat with `--jobs 4` for a concurrency datapoint — still not a
   product default change.

Use a quiet machine; treat numbers as **relative** evidence, not an SLA.

## Results table template

Copy into notes / issues when comparing hosts or arguing for pack:

| Host | Date | chunks | chunk_bytes | jobs | op | wall_s | chunk/s | notes |
|---|---|---|---|---|---|---|---|---|
| … | … | 64 | 65536 | 1 | push | … | … | stock stub, retries=0 |
| … | … | 64 | 65536 | 1 | pull | … | … | |
| … | … | 64 | 65536 | 4 | push | … | … | optional |

## Pack promotion checklist (decision aid only)

Pack remains **deferred**. Revisit only when **several** of the following hold
with measured baselines in hand:

| # | Condition | Why it matters |
|---|---|---|
| 1 | Typical link RTT is high (e.g. **> ~50–100 ms**) **and** loose push/pull wall time is dominated by per-chunk round-trips, not CPU/hash | Pack amortizes RTTs |
| 2 | Workloads prefer **many small chunks** (near min size) over few large ones | More objects → more RTT tax |
| 3 | Measured `chunk/s` at `jobs=1` stays low even when local stub (near-zero RTT) is fast — i.e. the **gap vs stub** on the real link is large | Isolates network, not local disk |
| 4 | Raising `--jobs` (opt-in) does not close the gap enough for the target deployment (connection limits, fair-share, SigV4 cost) | Concurrency is not enough |
| 5 | Operators accept breaking the “one chunk ↔ one object” mental model (new Source/Sink modes, URL templates, push/pull semantics) | Product / ops cost |
| 6 | A design exists that keeps `.cfidx` / `.cfdir` v1 **byte-frozen** and does not require `aws-sdk-*` / ListObjects | Phase constraints |

If the checklist fails (low RTT, large chunks, jobs-enough), **keep loose layout**.

## Non-goals

| Non-goal | Status |
|---|---|
| ❌ Implementing pack / multi-chunk objects | **Not this phase** |
| ❌ Changing default `--jobs` / `--http-retries` | Forbidden (silent behavior drift) |
| ❌ Criterion / CI performance gate with absolute SLAs | Noise; script is a local tool |
| ❌ Byte-range HTTP / multipart upload | Out of scope |

See also [remote-layout.md](remote-layout.md), [push.md](push.md), [pull.md](pull.md),
and [http-retry.md](http-retry.md).
