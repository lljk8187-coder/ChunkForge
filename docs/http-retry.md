# HTTP retry & error classification (Phase 8)

Bounded whole-chunk retries for **transient** HTTP failures on
`HttpChunkSource` / `HttpChunkSink`. Default **`--http-retries 0`** ≡ 0.7.0
(single attempt). See also [remote-layout.md](remote-layout.md),
[push.md](push.md), [pull.md](pull.md), [sigv4.md](sigv4.md),
[diff.md](diff.md) (`--format json`).

## Status → class

| HTTP status / condition | Class | Retried? | Summary bucket |
|---|---|---|---|
| **404**, **410** | **Missing** | no | `failed_permanent` |
| **408**, **429**, **500–504**, **520–524** | **Transient** | yes (within budget) | `failed_transient` |
| **401**, **403**, other **4xx** | **Permanent** | no | `failed_permanent` |
| Timeout / connection reset / refused / aborted | **Transient** | yes | `failed_transient` |
| BLAKE3 / content hash mismatch | **Corrupt** | **no** | `failed_permanent` |
| Other protocol / TLS / bad URI | **Permanent** | no | `failed_permanent` |

Library helpers (no trait signature change):

- `http_status_is_transient(code)`
- `classify_http_status(code) → ErrorClass`
- `classify_source_error` / `classify_sink_error` / `classify_ureq_error`
- `ErrorClass::summary_bucket()` → transient vs permanent for CLI counters

**Naming scheme (stable):** push / pull stderr summaries use

```text
failed=<total> failed_transient=<T> failed_permanent=<P> retries=<N>
```

where `failed = failed_transient + failed_permanent`, and **Missing** +
**Corrupt** roll into `failed_permanent` (O3 two-counter split). Hash /
corrupt failures never increment the retry path (they are fatal after the
successful transport read that produced the body).

## CLI knobs

| Flag | Meaning |
|---|---|
| `--http-retries N` | Extra attempts after the first try (default **0**) |
| `--http-retry-backoff-ms MS` | Base backoff (default **100**; exponential + full jitter, capped at 2s) |

Applies to HTTP(S) origins on `push` / `pull` / `verify` / `doctor` / `cat` /
`extract` (and shared HTTP args for `mount`). Local `--store` / `file://`
ignore the flags.

## Examples

```bash
# 503 then 200 — succeeds with budget
chunkforge push --store ./store --dest http://127.0.0.1:8766 \
  --http-retries 3 --http-retry-backoff-ms 0 release.cfdir

# 401 — permanent; summary shows failed_permanent≥1, not retried
chunkforge push --store ./store --dest http://127.0.0.1:8766 \
  --http-retries 5 release.cfdir
```

## Demo

```bash
bash scripts/demo_http_retry.sh
# put_stub --fail-transient → --http-retries 0 fails; ≥2/3 succeeds;
# push summary contains retries=
```
