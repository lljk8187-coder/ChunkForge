//! ChunkForge CLI: make / archive / extract / cat / verify / mount / doctor / gc / push / pull / diff / filter / ls / store (+ chunk-id debug).

mod bytesize;
mod parallel;
mod progress;

use anyhow::{Context, Result, bail};
use chunkforge_chunk::{ChunkId, ChunkInfo, ChunkParams, chunk_bytes};
use chunkforge_index::{
    DIR_FORMAT_VERSION_V1, DIR_MAGIC_PREFIX, DiffReport, DirArchive, DirEntry, DirEntryKind,
    FLAG_CHUNKS_COMPRESSED_IN_STORE, Index, IndexEntry, MAGIC_PREFIX, PathFilter, SeedDecision,
    UnchangedVerdict, decide_seed_for_entry_ex, decide_seed_trust_mtime,
    diff_dir_archives_with_progress, entry_length, filter_dir_archive, hash_reader,
    judge_extract_unchanged_opts, load_exclude_file, load_path_file, seed_file_map,
    validate_archive_path,
};
use chunkforge_remote::{
    FileUrlSource, HttpChunkSink, HttpChunkSource, RetryPolicy, SigV4Config, SigV4Signer,
    SummaryFailureBucket, classify_sink_error, classify_source_error, parse_store_location,
};
use chunkforge_store::{
    CacheSource, ChunkSink, ChunkSource, Compression, Error as StoreError, FallbackSource,
    PutOutcome, Store,
};
use clap::{Parser, Subcommand, ValueEnum};
use progress::ProgressReporter;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(
    name = "chunkforge",
    version,
    about = "Content-defined chunking + BLAKE3 CAS (make / archive / extract / cat / verify / mount / doctor / gc / push / pull / diff / filter / ls / store)",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Chunk a file, write chunks into a local store (dedup), and write a .cfidx
    ///
    /// Default **`--format text`** (≡ 1.4.0): summary on stderr
    /// (`make: wrote … (BYTES bytes, N chunk(s); new=X, reused=Y)`).
    /// **`--format json`**: one JSON object on stdout (`ok`, `bytes`, `chunks`,
    /// `new`, `reused`); no duplicate text summary; exit codes are
    /// format-independent. Optional **`--seed <PRIOR.cfidx>`** reuses the prior
    /// chunk table when size + content BLAKE3 match (skip FastCDC; still write
    /// new `-o`); missing prior chunks in store → clear non-zero (no silent
    /// invent). Optional **`--seed-trust-mtime`** (requires `--seed`): size +
    /// mtime match (input file vs prior `.cfidx` **file** mtime — `.cfidx` has
    /// no embedded mtime) → Reuse without content hash. Omit `--seed` ≡ 1.13
    /// make. With **`--dry-run`**, plan-only (no store create/put, no `.cfidx`
    /// write); with seed, reports reuse/rechunk without writing. Still **no**
    /// path 四件套. **≠** pack / **≠** recompress / **≠** path.
    Make {
        /// Local CAS store directory (created if missing; not written in `--dry-run`)
        #[arg(long)]
        store: PathBuf,
        /// Output .cfidx path (not written in `--dry-run`)
        #[arg(short = 'o', long = "output")]
        output: PathBuf,
        /// Input file to chunk
        input: PathBuf,
        /// Override FastCDC sizes as min:avg:max (bytes; all even, min≤avg≤max)
        #[arg(long = "chunk-size", value_name = "MIN:AVG:MAX")]
        chunk_size: Option<String>,
        /// Output format: `text` (default ≡ 1.4.0 stderr summary) or `json`
        /// (one object on stdout; no duplicate stderr summary)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
        /// On-disk chunk compression for a **new** store only (`none`|`zstd`;
        /// case-insensitive). Omit ≡ create with `none` (≡ 1.6); existing stores
        /// open by `meta.toml` (omit → no mismatch check; explicit value that
        /// differs from meta → clear non-zero error). Disk zstd is **not** HTTP
        /// wire compression / Content-Encoding. Default none ≡ 1.6. Ignored for
        /// create under `--dry-run` (dry-run never creates a store).
        #[arg(
            long = "compression",
            value_name = "none|zstd",
            value_parser = parse_cli_compression
        )]
        compression: Option<Compression>,
        /// Emit `progress: op=make done=N/TOTAL` on stderr.
        /// Granularity: single input file → TOTAL=1, one tick when the file is
        /// fully chunked and indexed (default off ≡ 1.6.0). Orthogonal to
        /// `--format json`.
        #[arg(long = "progress")]
        progress: bool,
        /// Max concurrent store puts **after** FastCDC finishes (default 1 =
        /// serial ≡ 1.8). Speeds post-chunk store put / on-disk encoding
        /// (zstd) only; FastCDC cut-points remain serial. Ignored on seed
        /// **Reuse** (no FastCDC / no put).
        #[arg(long, default_value_t = 1, value_name = "N")]
        jobs: u32,
        /// Plan-only: read input (+ FastCDC on rechunk path) + count
        /// chunks/bytes. With `--seed`, plan reuse/rechunk without writing. If
        /// the store exists, open for `has()` accounting → `would_write` /
        /// `would_reuse`. If missing, do **not** create; treat unique chunks as
        /// would_write (seed Reuse with missing chunks → clear non-zero). Does
        /// **not** put chunks or write `.cfidx`. **≠** pack / **≠** recompress /
        /// **≠** path.
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// Reuse an unchanged blob's chunk table from a prior `.cfidx` via
        /// content fingerprint (BLAKE3) with size fast-reject; changed content
        /// → rechunk (FastCDC + put). On Reuse: skip FastCDC, copy prior chunk
        /// table into new `-o`; prior chunks must be `has()` in store or clear
        /// non-zero (do not silently invent). Omit ≡ 1.13 make. **≠** pack /
        /// **≠** recompress / **≠** path.
        #[arg(long = "seed", value_name = "PRIOR.cfidx")]
        seed: Option<PathBuf>,
        /// With `--seed`: if size and mtime both match (input file mtime vs
        /// prior `.cfidx` **file** mtime — `.cfidx` has no embedded mtime),
        /// reuse without content BLAKE3. Default off (≡ 1.13 content path).
        /// WARNING: forged or incorrectly preserved mtimes can miss content
        /// changes — prefer content fingerprint unless you accept that risk.
        #[arg(long = "seed-trust-mtime", requires = "seed")]
        seed_trust_mtime: bool,
    },
    /// Archive a directory tree into a local store + `.cfdir` listing
    ///
    /// Recurses regular files (FastCDC + BLAKE3 per file). Chunks are written
    /// into `--store` with content-addressed dedup; the output `.cfdir` records
    /// relative paths and per-file chunk tables. Default **`--symlinks skip`**
    /// (≡ 1.11.0): symlinks are skipped with a stderr warning and not followed,
    /// **before** `--path`/`--path-from`/`--exclude` filtering. With
    /// **`--symlinks record`**, symlink paths are archive candidates that go
    /// through PathFilter (orthogonal to the path 四件套); recorded targets are
    /// stored as-is (not followed / not canonicalized); ≥1 Symlink ⇒ listing
    /// `format_version=2`. Fifos, sockets, and device nodes are always skipped
    /// with a stderr warning. Empty directories are **omitted by default** (≡
    /// **1.14.0**; extract can recreate parents from file paths). Opt-in
    /// **`--empty-dirs`**: record truly empty leaf directories as
    /// [`DirEntryKind::Dir`] (mode from metadata; relative path; same PathFilter
    /// as files — an empty dir is a candidate when the flag is set). Ancestor
    /// dirs of files remain implied by file paths; empty-dirs means dirs that
    /// would otherwise be dropped. **≠** prune **≠** write mount. Omit flag ≡
    /// 1.14. Optional repeatable `--path` / `--path-from` / `--exclude` /
    /// `--exclude-from` restrict which candidates are chunked / listed
    /// (default: full tree ≡ 1.9.0). Default **`--format text`** (≡ 1.2.0):
    /// summary on stderr. **`--format json`**: one JSON object on stdout; no
    /// duplicate text summary; exit codes are format-independent. `make`
    /// single-file semantics are unchanged.
    Archive {
        /// Local CAS store directory (created if missing; not written in `--dry-run`)
        #[arg(long)]
        store: PathBuf,
        /// Output `.cfdir` path (not written in `--dry-run`)
        #[arg(short = 'o', long = "output")]
        output: PathBuf,
        /// Source directory to recurse
        src_dir: PathBuf,
        /// Override FastCDC sizes as min:avg:max (bytes; all even, min≤avg≤max)
        #[arg(long = "chunk-size", value_name = "MIN:AVG:MAX")]
        chunk_size: Option<String>,
        /// Compute stats only; do not write store chunks or the `.cfdir`
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// Reuse unchanged files' chunk tables from a prior `.cfdir` via content
        /// fingerprint (BLAKE3); changed or missing-chunk files are rechunked
        #[arg(long = "seed", value_name = "PRIOR.cfdir")]
        seed: Option<PathBuf>,
        /// With `--seed`: if size and mtime_secs both match the prior entry,
        /// reuse without content BLAKE3. Default off (≡ 0.6.0 content path).
        /// WARNING: forged or incorrectly preserved mtimes can miss content
        /// changes — prefer content fingerprint unless you accept that risk.
        #[arg(long = "seed-trust-mtime", requires = "seed")]
        seed_trust_mtime: bool,
        /// Max concurrent per-file chunking (default 1 = serial). Seed map is
        /// read-only; store puts stay atomic / race-safe.
        #[arg(long, default_value_t = 1, value_name = "N")]
        jobs: u32,
        /// Include only archive paths under this prefix (repeatable; OR).
        /// With any `--path`, a candidate must match at least one before
        /// excludes apply. Omit all `--path` ⇒ include-all (≡ 1.2.0).
        #[arg(long = "path", value_name = "P", action = clap::ArgAction::Append)]
        paths: Vec<String>,
        /// Exclude archive paths matching this pattern (repeatable): exact,
        /// trailing-`/` directory prefix, or single edge `*` (`*.o`, `temp*`).
        /// Illegal middle `*` / `**` → clear error exit. Applied after type
        /// skip (symlink/special) and after `--path` includes.
        #[arg(long = "exclude", value_name = "PAT", action = clap::ArgAction::Append)]
        excludes: Vec<String>,

        /// Read exclude patterns from a UTF-8 file (repeatable). One pattern
        /// per line (same rules as `--exclude`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--exclude` into one `PathFilter`.
        /// Unreadable file or illegal pattern → clear non-zero error.
        #[arg(long = "exclude-from", value_name = "FILE", action = clap::ArgAction::Append)]
        exclude_from: Vec<PathBuf>,

        /// Read include path prefixes from a UTF-8 file (repeatable). One prefix
        /// per line (same rules as `--path`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--path` (OR) into one `PathFilter`.
        /// May combine with `--exclude` / `--exclude-from`. Unreadable file or
        /// bad UTF-8 → clear non-zero error. Omit all path flags ⇒ include-all
        /// (≡ 1.9.0 full tree).
        #[arg(long = "path-from", value_name = "FILE", action = clap::ArgAction::Append)]
        path_from: Vec<PathBuf>,
        /// Output format: `text` (default ≡ 1.2.0 stderr summary) or `json`
        /// (one object on stdout; no duplicate stderr summary)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
        /// On-disk chunk compression for a **new** store only (`none`|`zstd`;
        /// case-insensitive). Omit ≡ create with `none` (≡ 1.6); existing stores
        /// open by `meta.toml` (omit → no mismatch check; explicit value that
        /// differs from meta → clear non-zero error). Disk zstd is **not** HTTP
        /// wire compression / Content-Encoding. Default none ≡ 1.6. Ignored for
        /// create under `--dry-run` (dry-run never creates a store).
        #[arg(
            long = "compression",
            value_name = "none|zstd",
            value_parser = parse_cli_compression
        )]
        compression: Option<Compression>,
        /// Emit `progress: op=archive done=N/TOTAL` on stderr per filtered file
        /// (default off ≡ 1.6.0). Orthogonal to `--format json` and `--jobs`.
        #[arg(long = "progress")]
        progress: bool,
        /// Symlink handling: `skip` (default ≡ 1.11 skip+warn; not recorded /
        /// not followed) or `record` (write Symlink into the listing; not
        /// followed; absolute or empty target → clear non-zero). Orthogonal to
        /// `--path` / `--exclude` / seed / compression / progress / jobs.
        /// Special files still skip+warn.
        #[arg(long = "symlinks", value_enum, default_value_t = SymlinkPolicy::Skip)]
        symlinks: SymlinkPolicy,
        /// Record truly empty leaf directories as `DirEntryKind::Dir` (mode from
        /// metadata; relative path). Default **off** ≡ **1.14.0** omit empty
        /// dirs (parents of files remain implied by file paths). Empty-dir
        /// paths are PathFilter candidates like files (include then exclude).
        /// **≠** prune **≠** write mount. Orthogonal to `--symlinks` / seed /
        /// compression / progress / jobs / `--format`.
        #[arg(long = "empty-dirs")]
        empty_dirs: bool,
    },
    /// Materialize a directory tree from a `.cfdir` + chunk source
    ///
    /// Reads the **full** `.cfdir` listing and reconstitutes matching regular
    /// files under `-o` from `--store` / `--source` (same origin flags as
    /// `cat` / `verify`). Parent directories are created as needed for written
    /// files. Optional repeatable `--path` / `--path-from` / `--exclude` / `--exclude-from` restrict which
    /// listing entries are materialized (default: full tree ≡ 1.9.0). Path
    /// filtering is **not** prune: filtered-out listing paths and extra
    /// files already under `-o` are left alone — there is no delete/prune mode.
    /// If a destination path already exists, extract fails (non-zero) unless
    /// `--force` is set (overwrites existing regular files; type mismatches
    /// still error). With `--skip-unchanged`, files whose size and content
    /// BLAKE3 already match the listing are left untouched (no chunk fetch /
    /// write), even if `--force` is also set. With `--skip-trust-mtime`
    /// (requires `--skip-unchanged`), size+mtime match skips content BLAKE3.
    /// With `--dry-run`, no target paths are created or modified (output root
    /// included); stderr reports would_skip / would_write / would_dirs /
    /// would_fail / would_symlinks for the **filtered** set and exit is 0 unless the listing
    /// is invalid. Default **`--format text`** (≡ 1.0.0): summaries on stderr.
    /// **`--format json`**: one JSON object on stdout (`ok` / `skipped` /
    /// `wrote` / `dirs`, or dry-run `would_*`); exit codes are format-
    /// independent. Empty matching `Dir` entries create directories; file
    /// modes are restored on Unix when recorded.
    #[command(group(clap::ArgGroup::new("origin").required(true).args(["store", "source"])))]
    Extract {
        /// Local CAS store (Phase 1 compat; synonym for `--source <path>`)
        #[arg(long)]
        store: Option<PathBuf>,
        /// Chunk source: local path, `file://`, or `http(s)://`
        #[arg(long, value_name = "PATH|URL")]
        source: Option<String>,
        /// Extra chunk origin tried only on Missing (repeatable; CLI order preserved).
        /// Not a cache and not sync. Zero times ≡ 1.5 single-origin read path.
        #[arg(long = "fallback", value_name = "PATH|URL", action = clap::ArgAction::Append)]
        fallback: Vec<String>,
        /// Optional local cache store (filled on miss; never writes primary)
        #[arg(long, value_name = "DIR")]
        cache: Option<PathBuf>,
        /// Soft fill budget for `--cache` (bytes). Accepts a plain decimal
        /// integer or `<num>[K|M|G|Ki|Mi|Gi]` (1024-base, case-insensitive;
        /// `K`/`Ki`=2^10, `M`/`Mi`=2^20, `G`/`Gi`=2^30). No decimals; suffixes
        /// with `B` (KB/MB/GB) are rejected. Requires `--cache`. Omit ≡ 1.4
        /// unbounded fill. Over budget skips fill (still serves primary);
        /// never evicts / LRU.
        #[arg(
            long = "cache-max-bytes",
            value_name = "SIZE",
            value_parser = bytesize::parse_byte_size
        )]
        cache_max_bytes: Option<u64>,
        /// Emit `cache: hits=H miss_fills=F miss_refused=R` on stderr when the
        /// command finishes (mount: after the FUSE session ends / before exit).
        /// Requires `--cache`. Default **off** (≡ 1.7.0 quiet). Observation
        /// only — **not** LRU / trim / eviction.
        #[arg(long = "cache-stats")]
        cache_stats: bool,
        #[command(flatten)]
        http_tmpl: HttpTemplateArgs,
        /// Max concurrent chunk fetches (default 1 = serial)
        #[arg(long, default_value_t = 1, value_name = "N")]
        jobs: u32,
        /// Input `.cfdir`
        archive: PathBuf,
        /// Output directory (created if missing; without `--force` must not
        /// collide with existing files)
        #[arg(short = 'o', long = "output")]
        output: PathBuf,
        /// Overwrite existing regular files at destination paths. Without this
        /// flag, an existing path fails (≡ 0.6.0). Directory↔file type
        /// mismatches still error (refusing to replace a directory with a file
        /// or vice versa). When combined with `--skip-unchanged`, a content
        /// match still skips (match takes priority over force rewrite).
        #[arg(long = "force")]
        force: bool,
        /// Skip files whose destination already matches listing size + content
        /// BLAKE3 (no chunk fetch / write; mode untouched). Default **off**
        /// (≡ 0.8.0 full rewrite / conflict semantics). Existing but mismatched
        /// files still require `--force` to overwrite.
        #[arg(long = "skip-unchanged")]
        skip_unchanged: bool,
        /// With `--skip-unchanged`: if size and mtime_secs both match the
        /// listing File entry, skip without content BLAKE3. Default off
        /// (≡ 1.0.0 content path). WARNING: forged or incorrectly preserved
        /// mtimes (clock drift, `cp -p`, some network FS) can miss content
        /// changes — prefer content fingerprint unless you accept that risk.
        #[arg(long = "skip-trust-mtime", requires = "skip_unchanged")]
        skip_trust_mtime: bool,
        /// Plan only: create/modify **no** paths under `-o` (including the
        /// output root). Never fetches chunks. Without `--skip-unchanged`,
        /// does not open `--store`/`--source` and counts every listing file as
        /// `would_write` (existing conflicts without `--force` → `would_fail`).
        /// With `--skip-unchanged`, only reads local dests for size+BLAKE3
        /// judgment. Text format stderr:
        /// `extract: dry-run: would_skip=… would_write=… would_dirs=… would_fail=… would_symlinks=…`.
        /// Symlink would-writes also increment `would_write` (≡ 1.12); additive
        /// `would_symlinks` counts those cases (always present in JSON, incl. 0).
        /// Exit **0** when the listing is valid (even if `would_fail>0`).
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// Output format: `text` (default ≡ 1.0.0 stderr summary) or `json`
        /// (one object on stdout; no duplicate stderr summary)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
        /// Include only listing paths under this prefix (repeatable; OR).
        /// With any `--path`, a candidate must match at least one before
        /// excludes apply. Omit all `--path` ⇒ include-all (≡ 1.2.0 full tree).
        /// Does **not** delete filtered-out or extra dest paths (not prune).
        #[arg(long = "path", value_name = "P", action = clap::ArgAction::Append)]
        paths: Vec<String>,
        /// Exclude listing paths matching this pattern (repeatable): exact,
        /// trailing-`/` directory prefix, or single edge `*` (`*.o`, `temp*`).
        /// Illegal middle `*` / `**` → clear error exit. Orthogonal to
        /// `--force` / `--skip-*` / `--dry-run` / `--format` / `--jobs`.
        #[arg(long = "exclude", value_name = "PAT", action = clap::ArgAction::Append)]
        excludes: Vec<String>,

        /// Read exclude patterns from a UTF-8 file (repeatable). One pattern
        /// per line (same rules as `--exclude`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--exclude` into one `PathFilter`.
        /// Unreadable file or illegal pattern → clear non-zero error.
        #[arg(long = "exclude-from", value_name = "FILE", action = clap::ArgAction::Append)]
        exclude_from: Vec<PathBuf>,

        /// Read include path prefixes from a UTF-8 file (repeatable). One prefix
        /// per line (same rules as `--path`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--path` (OR) into one `PathFilter`.
        /// May combine with `--exclude` / `--exclude-from`. Unreadable file or
        /// bad UTF-8 → clear non-zero error. Omit all path flags ⇒ include-all
        /// (≡ 1.9.0 full tree). Does **not** delete filtered-out paths (not prune).
        #[arg(long = "path-from", value_name = "FILE", action = clap::ArgAction::Append)]
        path_from: Vec<PathBuf>,
        /// Emit `progress: op=extract done=N/TOTAL` on stderr per filtered File
        /// (PathFilter after; includes skip-unchanged judgments; default off ≡
        /// 1.6.0). Orthogonal to `--format json` and `--jobs`.
        #[arg(long = "progress")]
        progress: bool,
    },
    /// Reassemble a blob from a `.cfidx`, or one File from a `.cfdir` via `--path`
    ///
    /// Always writes the reassembled payload to `-o` (product unchanged).
    /// **`.cfidx`**: ≡ 1.14 — reassembles the whole blob; **do not** pass `--path`
    /// (`.cfidx` + `--path` → clear non-zero). JSON field names for `.cfidx`
    /// stay `ok` / `bytes` (+ optional `cache_*`).
    /// **`.cfdir`**: **requires** `--path <rel>` exact-matching one **File**
    /// entry (normalized vs listing path); Symlink / Dir / missing → clear
    /// non-zero. Reuses the same chunk fetch pipeline (jobs / cache / fallback /
    /// progress / format). **≠** extract whole tree **≠** prune **≠** multi-file
    /// batch (single `-o` only).
    /// Default **`--format text`** (≡ 1.4.0): almost no stderr summary on
    /// success. **`--format json`**: one JSON object on stdout (`ok`, `bytes`);
    /// no text dual-write; exit codes are format-independent. Orthogonal to
    /// `--cache` / `--cache-max-bytes` / `--jobs` / `--progress` / `--fallback` /
    /// `--cache-stats`.
    #[command(group(clap::ArgGroup::new("origin").required(true).args(["store", "source"])))]
    Cat {
        /// Local CAS store (Phase 1 compat; synonym for `--source <path>`)
        #[arg(long)]
        store: Option<PathBuf>,
        /// Chunk source: local path, `file://`, or `http(s)://`
        #[arg(long, value_name = "PATH|URL")]
        source: Option<String>,
        /// Extra chunk origin tried only on Missing (repeatable; CLI order preserved).
        /// Not a cache and not sync. Zero times ≡ 1.5 single-origin read path.
        #[arg(long = "fallback", value_name = "PATH|URL", action = clap::ArgAction::Append)]
        fallback: Vec<String>,
        /// Optional local cache store (filled on miss; never writes primary)
        #[arg(long, value_name = "DIR")]
        cache: Option<PathBuf>,
        /// Soft fill budget for `--cache` (bytes). Accepts a plain decimal
        /// integer or `<num>[K|M|G|Ki|Mi|Gi]` (1024-base, case-insensitive;
        /// `K`/`Ki`=2^10, `M`/`Mi`=2^20, `G`/`Gi`=2^30). No decimals; suffixes
        /// with `B` (KB/MB/GB) are rejected. Requires `--cache`. Omit ≡ 1.4
        /// unbounded fill. Over budget skips fill (still serves primary);
        /// never evicts / LRU.
        #[arg(
            long = "cache-max-bytes",
            value_name = "SIZE",
            value_parser = bytesize::parse_byte_size
        )]
        cache_max_bytes: Option<u64>,
        /// Emit `cache: hits=H miss_fills=F miss_refused=R` on stderr when the
        /// command finishes (mount: after the FUSE session ends / before exit).
        /// Requires `--cache`. Default **off** (≡ 1.7.0 quiet). Observation
        /// only — **not** LRU / trim / eviction.
        #[arg(long = "cache-stats")]
        cache_stats: bool,
        #[command(flatten)]
        http_tmpl: HttpTemplateArgs,
        /// Max concurrent chunk fetches (default 1 = serial / 0.3.0 behaviour)
        #[arg(long, default_value_t = 1, value_name = "N")]
        jobs: u32,
        /// Output format: `text` (default ≡ 1.4.0; almost silent on success)
        /// or `json` (one object on stdout; still writes `-o` payload)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
        /// Exact `.cfdir` File path to reassemble (required for `.cfdir`).
        /// Normalized (trim; strip leading `./`) then exact-matched against one
        /// listing File entry. With `.cfidx` → clear non-zero. Symlink / Dir /
        /// missing → clear non-zero. Single path only (**≠** multi-file /
        /// **≠** extract tree / **≠** prune).
        #[arg(long = "path", value_name = "REL")]
        path: Option<String>,
        /// Input `.cfidx` or `.cfdir`
        index: PathBuf,
        /// Output file path
        #[arg(short = 'o', long = "output")]
        output: PathBuf,
        /// Emit `progress: op=cat done=N/TOTAL` on stderr per listing chunk
        /// (TOTAL = entry count when known; default off ≡ 1.7.0). Orthogonal to
        /// `--format json`, `--jobs`, `--cache`, `--fallback`, and `--cache-stats`.
        #[arg(long = "progress")]
        progress: bool,
    },
    /// Verify `.cfidx` / `.cfdir` integrity, chunk presence/hashes, and blob_blake3
    ///
    /// Magic-dispatches: `.cfidx` → single-blob verify (unchanged); `.cfdir` →
    /// tree verify (structure + per-file `blob_blake3` + missing chunks fail with id).
    /// Optional repeatable `--path` / `--path-from` / `--exclude` / `--exclude-from`
    /// restrict which **File** entries in a `.cfdir` are verified (Dir entries never
    /// contribute chunks). Omit all path/exclude flags ⇒ full tree (≡ 1.9.0).
    /// `.cfidx` + any path/exclude flag (including `--path-from` / `--exclude-from`)
    /// → clear non-zero error. JSON field names unchanged (`files` / `chunks` may
    /// shrink under a filter). Orthogonal to `--progress` / `--jobs` / `--cache` /
    /// `--fallback` / `--cache-stats` / `--format`. Default **`--format text`**
    /// (≡ 0.9.0): summary on stderr. **`--format json`**: one JSON object on
    /// stdout (`ok` / `kind` / size fields); exit code is format-independent
    /// (ok → 0, failure → non-zero).
    #[command(group(clap::ArgGroup::new("origin").required(true).args(["store", "source"])))]
    Verify {
        /// Local CAS store (Phase 1 compat; synonym for `--source <path>`)
        #[arg(long)]
        store: Option<PathBuf>,
        /// Chunk source: local path, `file://`, or `http(s)://`
        #[arg(long, value_name = "PATH|URL")]
        source: Option<String>,
        /// Extra chunk origin tried only on Missing (repeatable; CLI order preserved).
        /// Not a cache and not sync. Zero times ≡ 1.5 single-origin read path.
        #[arg(long = "fallback", value_name = "PATH|URL", action = clap::ArgAction::Append)]
        fallback: Vec<String>,
        /// Optional local cache store (filled on miss; never writes primary)
        #[arg(long, value_name = "DIR")]
        cache: Option<PathBuf>,
        /// Soft fill budget for `--cache` (bytes). Accepts a plain decimal
        /// integer or `<num>[K|M|G|Ki|Mi|Gi]` (1024-base, case-insensitive;
        /// `K`/`Ki`=2^10, `M`/`Mi`=2^20, `G`/`Gi`=2^30). No decimals; suffixes
        /// with `B` (KB/MB/GB) are rejected. Requires `--cache`. Omit ≡ 1.4
        /// unbounded fill. Over budget skips fill (still serves primary);
        /// never evicts / LRU.
        #[arg(
            long = "cache-max-bytes",
            value_name = "SIZE",
            value_parser = bytesize::parse_byte_size
        )]
        cache_max_bytes: Option<u64>,
        /// Emit `cache: hits=H miss_fills=F miss_refused=R` on stderr when the
        /// command finishes (mount: after the FUSE session ends / before exit).
        /// Requires `--cache`. Default **off** (≡ 1.7.0 quiet). Observation
        /// only — **not** LRU / trim / eviction.
        #[arg(long = "cache-stats")]
        cache_stats: bool,
        #[command(flatten)]
        http_tmpl: HttpTemplateArgs,
        /// Max concurrent chunk fetches (default 1 = serial / 0.3.0 behaviour)
        #[arg(long, default_value_t = 1, value_name = "N")]
        jobs: u32,
        /// Output format: `text` (default ≡ 0.9.0 stderr summary) or `json`
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
        /// Emit `progress: op=verify done=N/TOTAL` on stderr per listing chunk
        /// (TOTAL = referenced chunk count for `.cfidx` / `.cfdir`, after path
        /// filter). Default **off** (≡ 1.7.0 quiet). Orthogonal to `--format
        /// json` / `--jobs` / `--cache` / `--fallback` / `--cache-stats` / path.
        #[arg(long = "progress")]
        progress: bool,
        /// Include only `.cfdir` File paths under this prefix (repeatable; OR).
        /// With any `--path`, a candidate must match at least one before
        /// excludes apply. Omit all `--path` ⇒ include-all (≡ 1.9.0 full set).
        /// Dir entries never contribute chunks. With `.cfidx` → clear non-zero
        /// error.
        #[arg(long = "path", value_name = "P", action = clap::ArgAction::Append)]
        paths: Vec<String>,
        /// Exclude `.cfdir` File paths matching this pattern (repeatable): exact,
        /// trailing `/` directory prefix, or single edge `*` (`*.o`, `temp*`).
        /// Illegal middle `*` / `**` → clear error. Applied after `--path`.
        /// With `.cfidx` → clear non-zero error.
        #[arg(long = "exclude", value_name = "PAT", action = clap::ArgAction::Append)]
        excludes: Vec<String>,

        /// Read exclude patterns from a UTF-8 file (repeatable). One pattern
        /// per line (same rules as `--exclude`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--exclude` into one `PathFilter`.
        /// Unreadable file or illegal pattern → clear non-zero error.
        #[arg(long = "exclude-from", value_name = "FILE", action = clap::ArgAction::Append)]
        exclude_from: Vec<PathBuf>,

        /// Read include path prefixes from a UTF-8 file (repeatable). One prefix
        /// per line (same rules as `--path`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--path` (OR) into one `PathFilter`.
        /// May combine with `--exclude` / `--exclude-from`. Unreadable file or
        /// bad UTF-8 → clear non-zero error. With `.cfidx` → clear non-zero error.
        #[arg(long = "path-from", value_name = "FILE", action = clap::ArgAction::Append)]
        path_from: Vec<PathBuf>,
        /// Input `.cfidx` or `.cfdir`
        index: PathBuf,
    },
    /// Debug: chunk + hash only; print offset/len/id (no store write)
    ///
    /// Default **`--format text`**: one `offset\tlength\tid` line per chunk on
    /// stdout. **`--format json`**: one JSON object on stdout
    /// (`ok` / `chunks`[{`offset`,`length`,`id`}]); no text dual-write. Exit
    /// codes are format-independent. No store write.
    #[command(name = "chunk-id")]
    ChunkId {
        /// Input file
        input: PathBuf,
        /// Override FastCDC sizes as min:avg:max (bytes; all even, min≤avg≤max)
        #[arg(long = "chunk-size", value_name = "MIN:AVG:MAX")]
        chunk_size: Option<String>,
        /// Output format (default text; json = one object on stdout)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
    },
    /// Mount a `.cfidx` (single file) or `.cfdir` (directory tree) read-only (Linux + fuse3)
    ///
    /// Optional repeatable `--path` / `--path-from` / `--exclude` / `--exclude-from`
    /// restrict which `.cfdir` **File+Symlink** paths appear in the FUSE tree
    /// (filtered File∪Symlink + ancestor Dirs). Default: no flags ⇒ full tree
    /// (≡ 1.10.0). `.cfidx` + any path/exclude flag (including `--path-from` /
    /// `--exclude-from`) → clear non-zero error. Still read-only; orthogonal to
    /// `--fallback` / `--cache*` / prefetch / SigV4. Not write-mount / prune /
    /// gc `--path` / sync.
    #[command(group(clap::ArgGroup::new("origin").required(true).args(["store", "source"])))]
    Mount {
        /// Local CAS store (Phase 1 compat; synonym for `--source <path>`)
        #[arg(long)]
        store: Option<PathBuf>,
        /// Chunk source: local path, `file://`, or `http(s)://`
        #[arg(long, value_name = "PATH|URL")]
        source: Option<String>,
        /// Extra chunk origin tried only on Missing (repeatable; CLI order preserved).
        /// Not a cache and not sync. Zero times ≡ 1.5 single-origin read path.
        #[arg(long = "fallback", value_name = "PATH|URL", action = clap::ArgAction::Append)]
        fallback: Vec<String>,
        /// Optional local cache store (filled on miss; never writes primary)
        #[arg(long, value_name = "DIR")]
        cache: Option<PathBuf>,
        /// Soft fill budget for `--cache` (bytes). Accepts a plain decimal
        /// integer or `<num>[K|M|G|Ki|Mi|Gi]` (1024-base, case-insensitive;
        /// `K`/`Ki`=2^10, `M`/`Mi`=2^20, `G`/`Gi`=2^30). No decimals; suffixes
        /// with `B` (KB/MB/GB) are rejected. Requires `--cache`. Omit ≡ 1.4
        /// unbounded fill. Over budget skips fill (still serves primary);
        /// never evicts / LRU.
        #[arg(
            long = "cache-max-bytes",
            value_name = "SIZE",
            value_parser = bytesize::parse_byte_size
        )]
        cache_max_bytes: Option<u64>,
        /// Emit `cache: hits=H miss_fills=F miss_refused=R` on stderr when the
        /// command finishes (mount: after the FUSE session ends / before exit).
        /// Requires `--cache`. Default **off** (≡ 1.7.0 quiet). Observation
        /// only — **not** LRU / trim / eviction.
        #[arg(long = "cache-stats")]
        cache_stats: bool,
        #[command(flatten)]
        http_tmpl: HttpTemplateArgs,
        /// Override the virtual file name for `.cfidx` mounts (default: stem without `.cfidx`; ignored for `.cfdir`)
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Disable sequential chunk prefetch (default: prefetch on; ≡ 0.9.0 on-demand get).
        /// Takes priority over `--prefetch-chunks`.
        #[arg(long = "no-prefetch")]
        no_prefetch: bool,
        /// Prefetch depth: how many subsequent chunks to warm (default 1 ≡ 1.0.0;
        /// hard cap ≤2; mount only). Ignored when `--no-prefetch` is set.
        #[arg(
            long = "prefetch-chunks",
            value_name = "N",
            default_value_t = 1,
            value_parser = clap::value_parser!(u32).range(1..=2)
        )]
        prefetch_chunks: u32,
        /// Include only `.cfdir` File+Symlink paths under this prefix (repeatable; OR).
        /// With any `--path`, a candidate must match at least one before
        /// excludes apply. Omit all `--path` ⇒ include-all (≡ 1.10.0 full tree).
        /// Ancestor Dir entries retained for kept File/Symlink leaves. With
        /// `.cfidx` → clear non-zero error.
        #[arg(long = "path", value_name = "P", action = clap::ArgAction::Append)]
        paths: Vec<String>,
        /// Exclude `.cfdir` File+Symlink paths matching this pattern (repeatable):
        /// exact, trailing `/` directory prefix, or single edge `*` (`*.o`,
        /// `temp*`). Illegal middle `*` / `**` → clear error. Applied after
        /// `--path`. With `.cfidx` → clear non-zero error.
        #[arg(long = "exclude", value_name = "PAT", action = clap::ArgAction::Append)]
        excludes: Vec<String>,
        /// Read exclude patterns from a UTF-8 file (repeatable). One pattern
        /// per line (same rules as `--exclude`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--exclude` into one `PathFilter`.
        /// Unreadable file or illegal pattern → clear non-zero error.
        /// With `.cfidx` → clear non-zero error.
        #[arg(long = "exclude-from", value_name = "FILE", action = clap::ArgAction::Append)]
        exclude_from: Vec<PathBuf>,
        /// Read include path prefixes from a UTF-8 file (repeatable). One prefix
        /// per line (same rules as `--path`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--path` (OR) into one `PathFilter`.
        /// May combine with `--exclude` / `--exclude-from`. Unreadable file or
        /// bad UTF-8 → clear non-zero error. With `.cfidx` → clear non-zero error.
        #[arg(long = "path-from", value_name = "FILE", action = clap::ArgAction::Append)]
        path_from: Vec<PathBuf>,
        /// Input `.cfidx` or `.cfdir`
        index: PathBuf,
        /// Empty directory to mount onto
        mountpoint: PathBuf,
    },
    /// Check indexes and chunk presence (missing ids → non-zero exit)
    ///
    /// Optional repeatable `--path` / `--path-from` / `--exclude` / `--exclude-from`
    /// restrict which **File** entries in a `.cfdir` contribute chunk ids (Dir
    /// entries never contribute). Omit all path/exclude flags ⇒ full reference
    /// set (≡ 1.9.0). `.cfidx` + any path/exclude flag (including `--path-from` /
    /// `--exclude-from`) → clear non-zero error. JSON field names unchanged
    /// (`checked` / `missing` may shrink under a filter). Orthogonal to `--deep`
    /// / `--fallback` / `--cache*` / `--jobs` / `--progress` / `--format`.
    /// Default **`--format text`** (≡ 0.9.0): ok summary on stderr; missing ids
    /// one-per-line on stdout then non-zero. **`--format json`**: one JSON object
    /// on stdout (`ok` / `listings` / `checked` / `missing` / `deep`); missing ids
    /// live in the JSON only (not also printed as bare lines). Exit code is
    /// format-independent.
    #[command(group(clap::ArgGroup::new("origin").required(true).args(["store", "source"])))]
    Doctor {
        /// Local CAS store (Phase 1 compat; synonym for `--source <path>`)
        #[arg(long)]
        store: Option<PathBuf>,
        /// Chunk source: local path, `file://`, or `http(s)://`
        #[arg(long, value_name = "PATH|URL")]
        source: Option<String>,
        /// Extra chunk origin tried only on Missing (repeatable; CLI order preserved).
        /// Not a cache and not sync. Zero times ≡ 1.5 single-origin read path.
        #[arg(long = "fallback", value_name = "PATH|URL", action = clap::ArgAction::Append)]
        fallback: Vec<String>,
        /// Optional local cache store (filled on miss; never writes primary)
        #[arg(long, value_name = "DIR")]
        cache: Option<PathBuf>,
        /// Soft fill budget for `--cache` (bytes). Accepts a plain decimal
        /// integer or `<num>[K|M|G|Ki|Mi|Gi]` (1024-base, case-insensitive;
        /// `K`/`Ki`=2^10, `M`/`Mi`=2^20, `G`/`Gi`=2^30). No decimals; suffixes
        /// with `B` (KB/MB/GB) are rejected. Requires `--cache`. Omit ≡ 1.4
        /// unbounded fill. Over budget skips fill (still serves primary);
        /// never evicts / LRU.
        #[arg(
            long = "cache-max-bytes",
            value_name = "SIZE",
            value_parser = bytesize::parse_byte_size
        )]
        cache_max_bytes: Option<u64>,
        /// Emit `cache: hits=H miss_fills=F miss_refused=R` on stderr when the
        /// command finishes (mount: after the FUSE session ends / before exit).
        /// Requires `--cache`. Default **off** (≡ 1.7.0 quiet). Observation
        /// only — **not** LRU / trim / eviction.
        #[arg(long = "cache-stats")]
        cache_stats: bool,
        #[command(flatten)]
        http_tmpl: HttpTemplateArgs,
        /// Max concurrent presence checks (default 1 = serial / 0.3.0 behaviour)
        #[arg(long, default_value_t = 1, value_name = "N")]
        jobs: u32,
        /// Use `get` (discard body) instead of `has` for presence checks
        #[arg(long)]
        deep: bool,
        /// Skip the optional one-shot HTTP base connectivity probe
        #[arg(long = "no-probe")]
        no_probe: bool,
        /// Output format: `text` (default ≡ 0.9.0) or `json`
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
        /// Emit progress: op=doctor done=N/TOTAL on stderr per checked chunk
        /// (TOTAL = referenced chunk ids after path filter). Default off ≡ 1.7.0.
        /// Orthogonal to `--format json` / `--jobs` / `--cache` / `--fallback` /
        /// `--cache-stats` / path.
        #[arg(long = "progress")]
        progress: bool,
        /// Include only `.cfdir` File paths under this prefix (repeatable; OR).
        /// With any `--path`, a candidate must match at least one before
        /// excludes apply. Omit all `--path` ⇒ include-all (≡ 1.9.0 full set).
        /// Dir entries never contribute chunks. With `.cfidx` → clear non-zero
        /// error.
        #[arg(long = "path", value_name = "P", action = clap::ArgAction::Append)]
        paths: Vec<String>,
        /// Exclude `.cfdir` File paths matching this pattern (repeatable): exact,
        /// trailing `/` directory prefix, or single edge `*` (`*.o`, `temp*`).
        /// Illegal middle `*` / `**` → clear error. Applied after `--path`.
        /// With `.cfidx` → clear non-zero error.
        #[arg(long = "exclude", value_name = "PAT", action = clap::ArgAction::Append)]
        excludes: Vec<String>,

        /// Read exclude patterns from a UTF-8 file (repeatable). One pattern
        /// per line (same rules as `--exclude`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--exclude` into one `PathFilter`.
        /// Unreadable file or illegal pattern → clear non-zero error.
        #[arg(long = "exclude-from", value_name = "FILE", action = clap::ArgAction::Append)]
        exclude_from: Vec<PathBuf>,

        /// Read include path prefixes from a UTF-8 file (repeatable). One prefix
        /// per line (same rules as `--path`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--path` (OR) into one `PathFilter`.
        /// May combine with `--exclude` / `--exclude-from`. Unreadable file or
        /// bad UTF-8 → clear non-zero error. With `.cfidx` → clear non-zero error.
        #[arg(long = "path-from", value_name = "FILE", action = clap::ArgAction::Append)]
        path_from: Vec<PathBuf>,
        /// One or more `.cfidx` / `.cfdir` listings to check
        #[arg(required = true, num_args = 1..)]
        indexes: Vec<PathBuf>,
    },
    /// List (or delete) unreferenced loose chunks in a local store
    ///
    /// Default **`--format text`** (≡ 1.1.0): unreferenced `.cnk` paths on
    /// stdout (ordered) + summary on stderr. **`--format json`**: one JSON
    /// object on stdout (`ok` / `dry_run` / `applied` / `listings` /
    /// `referenced` / `unreferenced` / `deleted`); no path listing and no
    /// duplicate stderr summary; exit codes are format-independent.
    /// `--jobs` is orthogonal to `--format`.
    Gc {
        /// Local CAS store directory
        #[arg(long)]
        store: PathBuf,
        /// Actually delete unreferenced `.cnk` files (default is dry-run)
        #[arg(long)]
        apply: bool,
        /// Max concurrent deletes on `--apply` (default 1 = serial ≡ 1.1.0).
        /// Dry-run path listing stays ordered/serial; `--jobs` primarily
        /// speeds `--apply` (symmetric to `store scrub --jobs`).
        #[arg(long, default_value_t = 1, value_name = "N")]
        jobs: u32,
        /// Output format: `text` (default ≡ 1.1.0 path list + stderr summary)
        /// or `json` (one object on stdout; no duplicate stderr summary)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
        /// Emit `progress: op=gc done=N/TOTAL` on stderr while `--apply` deletes
        /// (default off ≡ 1.1.0). Orthogonal to `--format json`.
        #[arg(long = "progress")]
        progress: bool,
        /// One or more `.cfidx` / `.cfdir` listings whose chunk ids are retained
        #[arg(required = true, num_args = 1..)]
        indexes: Vec<PathBuf>,
    },
    /// Upload missing chunks referenced by `.cfidx` / `.cfdir` listings to `--dest`
    ///
    /// Reads plaintext chunks from the local `--store`, probes the destination
    /// with `has`, and puts only missing ids. Does **not** upload `.cfidx` /
    /// `.cfdir` listing files themselves (chunks only). **`--dest`** may be
    /// `http(s)://`, a **local store path**, or **`file://`** (single dest;
    /// **≠** read-side fallback / multi-dest). Local/`file://` opens an existing
    /// Store or creates one (omit/`--compression` ≡ create **none** ≡ 1.10;
    /// explicit `zstd` only for **new** local/`file://` dest) via `Store` as
    /// `ChunkSink`. **`http(s)://` dest + any `--compression` (including
    /// explicit `none`) → clear non-zero.** Still **single** dest (**≠**
    /// read-side fallback / multi-dest; **≠** `store recompress` / default zstd /
    /// HTTP wire compression). HTTP template flags
    /// (`--url-template` / `--prefix` / `--header` / `--aws-sigv4` /
    /// `--http-retries`) apply only to `http(s)://` dest — with a local dest
    /// they are a clear non-zero error. For HTTP dest, templates match
    /// read-side layout so a successful push is readable with
    /// `verify --source`. `--http-retries N` (default 0 ≡ 0.7.0) retries
    /// transient HTTP failures (extra attempts after the first try); see also
    /// `--http-retry-backoff-ms`.
    /// Optional repeatable `--path` / `--path-from` / `--exclude` / `--exclude-from` restrict which
    /// **File** entries in a `.cfdir` contribute chunk ids (Dir entries never
    /// contribute; listing itself is **not** uploaded). Omit all path/exclude
    /// flags ⇒ full reference set (≡ 1.9.0). `.cfidx` + any path/exclude flag
    /// (including `--path-from` / `--exclude-from`) → clear non-zero error.
    /// Orthogonal to `--dry-run` / `--format` / `--jobs` / `--progress` /
    /// retries / SigV4 / `--verify`. With `--verify`, after a successful upload
    /// the same `--dest` is treated as a `ChunkSource` and each listing is
    /// verified (skip verify on `--dry-run` or when push already failed).
    /// Default **`--format text`** (≡ 1.0.0): summary on stderr. **`--format
    /// json`**: one JSON object on stdout (`ok` / `skipped` / `uploaded` /
    /// `failed` / `failed_transient` / `failed_permanent` / `retries` /
    /// `unique_chunks` / `listings` / `dry_run`); field names unchanged —
    /// `unique_chunks` is the **filtered** unique id count; no duplicate stderr
    /// summary; exit codes are format-independent.
    Push {
        /// Local CAS store providing plaintext chunks
        #[arg(long)]
        store: PathBuf,
        /// Destination: `http(s)://` base URL, local CAS path, or `file://`
        /// (single dest; local/`file://` ⇒ Store as ChunkSink; create uses
        /// `--compression` / omit ≡ none ≡ 1.10)
        #[arg(long, value_name = "URL|PATH")]
        dest: String,
        #[command(flatten)]
        http_tmpl: HttpTemplateArgs,
        /// Max concurrent has/PUT workers (default 1 = serial); also used for post-push `--verify` fetches
        #[arg(long, default_value_t = 1, value_name = "N")]
        jobs: u32,
        /// Probe and count only; do not issue PUT
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// After a successful push, verify each listing against `--dest` (same templates)
        #[arg(long = "verify")]
        verify: bool,
        /// Output format: `text` (default ≡ 1.0.0 stderr summary) or `json`
        /// (one object on stdout; no duplicate stderr summary)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
        /// Emit `progress: op=push done=N/TOTAL` on stderr per chunk
        /// (default off ≡ 1.1.0). Orthogonal to `--format json`.
        #[arg(long = "progress")]
        progress: bool,
        /// Include only `.cfdir` File paths under this prefix (repeatable; OR).
        /// With any `--path`, a candidate must match at least one before
        /// excludes apply. Omit all `--path` ⇒ include-all (≡ 1.3.0 full set).
        /// Dir entries never contribute chunks; does not upload listings.
        /// With `.cfidx` → clear non-zero error.
        #[arg(long = "path", value_name = "P", action = clap::ArgAction::Append)]
        paths: Vec<String>,
        /// Exclude `.cfdir` File paths matching this pattern (repeatable): exact,
        /// trailing `/` directory prefix, or single edge `*` (`*.o`, `temp*`).
        /// Illegal middle `*` / `**` → clear error. Applied after `--path`.
        /// With `.cfidx` → clear non-zero error.
        #[arg(long = "exclude", value_name = "PAT", action = clap::ArgAction::Append)]
        excludes: Vec<String>,

        /// Read exclude patterns from a UTF-8 file (repeatable). One pattern
        /// per line (same rules as `--exclude`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--exclude` into one `PathFilter`.
        /// Unreadable file or illegal pattern → clear non-zero error.
        #[arg(long = "exclude-from", value_name = "FILE", action = clap::ArgAction::Append)]
        exclude_from: Vec<PathBuf>,

        /// Read include path prefixes from a UTF-8 file (repeatable). One prefix
        /// per line (same rules as `--path`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--path` (OR) into one `PathFilter`.
        /// May combine with `--exclude` / `--exclude-from`. Unreadable file or
        /// bad UTF-8 → clear non-zero error. With `.cfidx` → clear non-zero error.
        #[arg(long = "path-from", value_name = "FILE", action = clap::ArgAction::Append)]
        path_from: Vec<PathBuf>,
        /// On-disk chunk compression for a **new** local/`file://` `--dest` only
        /// (`none`|`zstd`; case-insensitive). Same create semantics as
        /// `pull`/`make`/`archive`/`store create`: omit ≡ create with `none`
        /// (≡ 1.10); existing dest store opens by `meta.toml` (omit → no
        /// mismatch check; explicit value that differs from meta → clear
        /// non-zero error). **`http(s)://` dest + any `--compression`
        /// (including explicit `none`) → clear non-zero.** Disk zstd is **not**
        /// HTTP wire compression. **≠** `store recompress` / default zstd /
        /// read-side fallback / multi-dest. Orthogonal to `--verify` / `--jobs` /
        /// `--progress` / path scope.
        #[arg(
            long = "compression",
            value_name = "none|zstd",
            value_parser = parse_cli_compression
        )]
        compression: Option<Compression>,
        /// One or more `.cfidx` / `.cfdir` listings whose chunk ids are uploaded
        #[arg(required = true, num_args = 1..)]
        indexes: Vec<PathBuf>,
    },
    /// Fill a local CAS `--store` with missing chunks from `--source`
    ///
    /// Merges chunk ids referenced by one or more `.cfidx` / `.cfdir` listings.
    /// For each id: if already present in `--store`, skip; otherwise `source.get`
    /// then `store.put` (plaintext into the local CAS). Does **not** extract a
    /// file tree, delete extras (`gc`), or download the listing itself.
    /// Optional repeatable `--path` / `--path-from` / `--exclude` / `--exclude-from` restrict which
    /// **File** entries in a `.cfdir` contribute chunk ids (Dir entries never
    /// contribute; listing file unchanged / not downloaded). Omit all path/exclude
    /// flags ⇒ full reference set (≡ 1.9.0). Orthogonal to `--dry-run` / `--format` /
    /// `--jobs` /
    /// `--progress` / retries / SigV4 / `--verify`. With `--verify`, after a
    /// successful fetch the local `--store` is treated as a `ChunkSource` and
    /// each listing is verified (skip verify on `--dry-run` or when pull
    /// already failed). `--source` accepts a local path,
    /// `file://`, or `http(s)://` (same templates as `verify` / `cat`).
    /// `--http-retries N` (default 0) applies to HTTP sources only. Symmetric
    /// to `push` (store→dest) but source→store. Default **`--format text`**
    /// (≡ 1.0.0): summary on stderr. **`--format json`**: one JSON object on
    /// stdout (`ok` / `skipped` / `fetched` / `failed` / `failed_transient` /
    /// `failed_permanent` / `retries` / `unique_chunks` / `listings` /
    /// `dry_run`); field names unchanged — `unique_chunks` is the **filtered**
    /// unique id count; no duplicate stderr summary; exit codes are
    /// format-independent.
    Pull {
        /// Local CAS store to fill (created if missing; not written in `--dry-run`)
        #[arg(long)]
        store: PathBuf,
        /// Chunk source: local path, `file://`, or `http(s)://`
        #[arg(long, value_name = "PATH|URL")]
        source: String,
        /// Extra chunk origin tried only on Missing (repeatable; CLI order preserved).
        /// Not a cache and not sync. Zero times ≡ 1.5 single-origin read path.
        #[arg(long = "fallback", value_name = "PATH|URL", action = clap::ArgAction::Append)]
        fallback: Vec<String>,
        /// Optional local cache store (filled on miss; never writes primary)
        #[arg(long, value_name = "DIR")]
        cache: Option<PathBuf>,
        /// Soft fill budget for `--cache` (bytes). Accepts a plain decimal
        /// integer or `<num>[K|M|G|Ki|Mi|Gi]` (1024-base, case-insensitive;
        /// `K`/`Ki`=2^10, `M`/`Mi`=2^20, `G`/`Gi`=2^30). No decimals; suffixes
        /// with `B` (KB/MB/GB) are rejected. Requires `--cache`. Omit ≡ 1.4
        /// unbounded fill. Over budget skips fill (still serves primary);
        /// never evicts / LRU.
        #[arg(
            long = "cache-max-bytes",
            value_name = "SIZE",
            value_parser = bytesize::parse_byte_size
        )]
        cache_max_bytes: Option<u64>,
        /// Emit `cache: hits=H miss_fills=F miss_refused=R` on stderr when the
        /// command finishes (mount: after the FUSE session ends / before exit).
        /// Requires `--cache`. Default **off** (≡ 1.7.0 quiet). Observation
        /// only — **not** LRU / trim / eviction.
        #[arg(long = "cache-stats")]
        cache_stats: bool,
        #[command(flatten)]
        http_tmpl: HttpTemplateArgs,
        /// Max concurrent has/get/put workers (default 1 = serial); also used for post-pull `--verify` fetches
        #[arg(long, default_value_t = 1, value_name = "N")]
        jobs: u32,
        /// Probe and count only; do not write the local store
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// After a successful pull, verify each listing against local `--store`
        #[arg(long = "verify")]
        verify: bool,
        /// Output format: `text` (default ≡ 1.0.0 stderr summary) or `json`
        /// (one object on stdout; no duplicate stderr summary)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
        /// On-disk chunk compression for a **new** local `--store` only (`none`|`zstd`;
        /// case-insensitive). Same semantics as `make`/`archive`/`store create`:
        /// omit ≡ create with `none` (≡ 1.8); existing stores open by `meta.toml`
        /// (omit → no mismatch check; explicit value that differs from meta → clear
        /// non-zero error). **Dry-run never creates.** Disk zstd is **not** HTTP wire
        /// compression. Orthogonal to `--verify` / `--fallback` / `--cache*` / `--jobs`
        /// / `--progress` / path scope.
        #[arg(
            long = "compression",
            value_name = "none|zstd",
            value_parser = parse_cli_compression
        )]
        compression: Option<Compression>,
        /// Emit `progress: op=pull done=N/TOTAL` on stderr per chunk
        /// (default off ≡ 1.1.0). Orthogonal to `--format json`.
        #[arg(long = "progress")]
        progress: bool,
        /// Include only `.cfdir` File paths under this prefix (repeatable; OR).
        /// With any `--path`, a candidate must match at least one before
        /// excludes apply. Omit all `--path` ⇒ include-all (≡ 1.2.0 full set).
        /// Dir entries never contribute chunks; does not download/alter listings.
        #[arg(long = "path", value_name = "P", action = clap::ArgAction::Append)]
        paths: Vec<String>,
        /// Exclude `.cfdir` File paths matching this pattern (repeatable): exact,
        /// trailing `/` directory prefix, or single edge `*` (`*.o`, `temp*`).
        /// Illegal middle `*` / `**` → clear error. Applied after `--path`.
        #[arg(long = "exclude", value_name = "PAT", action = clap::ArgAction::Append)]
        excludes: Vec<String>,

        /// Read exclude patterns from a UTF-8 file (repeatable). One pattern
        /// per line (same rules as `--exclude`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--exclude` into one `PathFilter`.
        /// Unreadable file or illegal pattern → clear non-zero error.
        #[arg(long = "exclude-from", value_name = "FILE", action = clap::ArgAction::Append)]
        exclude_from: Vec<PathBuf>,

        /// Read include path prefixes from a UTF-8 file (repeatable). One prefix
        /// per line (same rules as `--path`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--path` (OR) into one `PathFilter`.
        /// May combine with `--exclude` / `--exclude-from`. Unreadable file or
        /// bad UTF-8 → clear non-zero error. Omit all path flags ⇒ include-all
        /// (≡ 1.9.0 full set).
        #[arg(long = "path-from", value_name = "FILE", action = clap::ArgAction::Append)]
        path_from: Vec<PathBuf>,
        /// One or more `.cfidx` / `.cfdir` listings whose chunk ids are fetched
        #[arg(required = true, num_args = 1..)]
        indexes: Vec<PathBuf>,
    },
    /// Compare `.cfdir` listings, or a source tree against a listing (`--tree`)
    ///
    /// **Listing↔listing:** two `.cfdir` args (`.cfidx` / bad magic → error).
    /// Symlink target/mode are already compared by the library; `--symlinks`
    /// **requires** `--tree` (clap error without it) and does not change
    /// listing↔listing behaviour.
    /// **Tree↔listing:** `--tree <src-dir> <listing.cfdir>` — builds an ephemeral
    /// in-memory `DirArchive` from `src-dir` (left) and compares it to the
    /// listing (right). Read-only: does **not** write store or `.cfdir`.
    /// Default **`--symlinks skip`** (≡ 1.12.0): live symlinks are skip+warn and
    /// not followed / not recorded. **`--symlinks record`**: push
    /// [`DirEntryKind::Symlink`] into the ephemeral tree (target as-is, mode from
    /// `symlink_metadata`; **not** followed; absolute or empty target → clear
    /// non-zero). Special files still skip+warn. PathFilter applies after the
    /// ephemeral build (symlink paths participate when allowed).
    ///
    /// Reports path-level **added** / **removed** / **changed** (content) /
    /// **meta_changed** (same blake3, mode/mtime differ). Default **`--format
    /// text`** (≡ 0.7.0): path lists (non-empty categories) plus a stable
    /// summary line on **stdout**:
    /// `diff: added=… removed=… changed=… meta_changed=… chunks_shared=… chunks_only_left=… chunks_only_right=…`
    /// **`--format json`**: one JSON object with the same path arrays and chunk
    /// stats fields (full arrays; `--max-paths` applies to text listings only).
    /// Optional repeatable `--path` / `--path-from` / `--exclude` / `--exclude-from` narrow both
    /// sides' File/Dir/Symlink entry sets via [`PathFilter`] **before** compare (default
    /// no flags ≡ 1.9 full-listing diff). JSON field names unchanged (arrays may
    /// be shorter). **Not** sync / prune. Exit codes are format-independent:
    /// **0** when identical, **1** when any path or chunk-set difference; usage /
    /// decode errors use the usual non-zero clap/anyhow path. See `docs/diff.md`.
    Diff {
        /// Output format: `text` (default ≡ 0.7.0 path lists + `diff:` summary) or `json`
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
        /// Max paths to print per category in text format (default: unlimited; ignored by json)
        #[arg(long = "max-paths", value_name = "N")]
        max_paths: Option<usize>,
        /// Source directory to compare as left (ephemeral DirArchive; no store/.cfdir writes)
        #[arg(long = "tree", value_name = "SRC_DIR")]
        tree: Option<PathBuf>,
        /// Include only paths under this prefix (repeatable; OR). With any `--path`,
        /// a candidate must match at least one before excludes apply. Omit all
        /// `--path` ⇒ include-all (≡ 1.5 full listing). Applied to both sides
        /// before compare. **Not** sync / prune.
        #[arg(long = "path", value_name = "P", action = clap::ArgAction::Append)]
        paths: Vec<String>,
        /// Exclude paths matching this pattern (repeatable): exact, trailing `/`
        /// directory prefix, or single edge `*` (`*.o`, `temp*`). Illegal middle
        /// `*` / `**` → clear error. Applied after `--path` on both sides.
        #[arg(long = "exclude", value_name = "PAT", action = clap::ArgAction::Append)]
        excludes: Vec<String>,
        /// Read exclude patterns from a UTF-8 file (repeatable). One pattern
        /// per line (same rules as `--exclude`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--exclude` into one `PathFilter`.
        /// Unreadable file or illegal pattern → clear non-zero error.
        #[arg(long = "exclude-from", value_name = "FILE", action = clap::ArgAction::Append)]
        exclude_from: Vec<PathBuf>,

        /// Read include path prefixes from a UTF-8 file (repeatable). One prefix
        /// per line (same rules as `--path`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--path` (OR) into one `PathFilter`.
        /// May combine with `--exclude` / `--exclude-from`. Unreadable file or
        /// bad UTF-8 → clear non-zero error. Applied to both sides before compare.
        #[arg(long = "path-from", value_name = "FILE", action = clap::ArgAction::Append)]
        path_from: Vec<PathBuf>,
        /// Emit `progress: op=diff done=N/TOTAL` on stderr per filtered **File
        /// or Symlink** path in the union of both sides (TOTAL = |left∪right|
        /// File+Symlink paths after `--path`/`--path-from`/`--exclude`/
        /// `--exclude-from`; Dir-only ignored). Default **off** (≡ 1.8.0 quiet).
        /// Orthogonal to `--format json` (progress→stderr, JSON→stdout) and
        /// path filters.
        #[arg(long = "progress")]
        progress: bool,
        /// Symlink handling for `--tree` only: `skip` (default ≡ 1.12 tree
        /// skip+warn; not recorded / not followed) or `record` (ephemeral
        /// Symlink entries; not followed; absolute or empty target → clear
        /// non-zero). Requires `--tree` (listing↔listing already compares
        /// Symlink in-lib; flag rejected without `--tree`). Special files
        /// still skip+warn. **Not** sync / follow / write-mount.
        #[arg(
            long = "symlinks",
            value_enum,
            default_value_t = SymlinkPolicy::Skip,
            requires = "tree"
        )]
        symlinks: SymlinkPolicy,
        /// Left `.cfdir` (listing↔listing), or the listing `.cfdir` when `--tree` is set
        left: PathBuf,
        /// Right `.cfdir` (listing↔listing only). Must be omitted with `--tree`.
        right: Option<PathBuf>,
    },

    /// Persist a path-scoped subset of an existing `.cfdir` listing
    ///
    /// Reads an input `.cfdir`, applies the same path 四件套 as `archive` /
    /// `extract` / `mount` / `diff` (`--path` / `--path-from` / `--exclude` /
    /// `--exclude-from`) via library [`filter_dir_archive`], then
    /// [`DirArchive::encode`]s the result to `-o`. Empty path flags ≡
    /// **identity** (re-encode / normalize; same File+Symlink leaf set). Keeps
    /// matching **File** and **Symlink** leaves plus ancestor **Dir** entries.
    ///
    /// **Does not** open a store, rechunk, touch a source tree, prune a dest
    /// tree, or rewrite the input listing in place. **≠** prune / **≠**
    /// `gc --path` / **≠** sync / **≠** write mount / **≠** pack / **≠**
    /// `archive --path` (no source-tree walk — input must already be a
    /// `.cfdir`). `.cfidx` / wrong magic → clear non-zero error.
    ///
    /// Without `--force`, if `-o` already exists → clear non-zero (no silent
    /// overwrite). With `--force`, atomic replace (temp sibling + rename).
    /// **`--dry-run`**: compute filtered listing + counts; **do not** write
    /// `-o` (still takes `-o` as the planned path, like archive/make). Default
    /// **`--format text`**: stderr summary. **`--format json`**: one JSON
    /// object on stdout (`ok` / `dry_run` / `input` / `output` / `files` /
    /// `dirs` / `symlinks` / `excluded`); no text dual-write; exit codes are
    /// format-independent.
    Filter {
        /// Output `.cfdir` path (planned path under `--dry-run`; without
        /// `--force` must not already exist when writing)
        #[arg(short = 'o', long = "output")]
        output: PathBuf,
        /// Include only listing paths under this prefix (repeatable; OR).
        /// With any `--path`, a candidate must match at least one before
        /// excludes apply. Omit all `--path` ⇒ include-all (identity when
        /// no exclude flags either).
        #[arg(long = "path", value_name = "P", action = clap::ArgAction::Append)]
        paths: Vec<String>,
        /// Exclude listing paths matching this pattern (repeatable): exact,
        /// trailing-`/` directory prefix, or single edge `*` (`*.o`, `temp*`).
        /// Illegal middle `*` / `**` → clear error exit. Applied after `--path`.
        #[arg(long = "exclude", value_name = "PAT", action = clap::ArgAction::Append)]
        excludes: Vec<String>,
        /// Read exclude patterns from a UTF-8 file (repeatable). One pattern
        /// per line (same rules as `--exclude`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--exclude` into one `PathFilter`.
        /// Unreadable file or illegal pattern → clear non-zero error.
        #[arg(long = "exclude-from", value_name = "FILE", action = clap::ArgAction::Append)]
        exclude_from: Vec<PathBuf>,
        /// Read include path prefixes from a UTF-8 file (repeatable). One prefix
        /// per line (same rules as `--path`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--path` (OR) into one `PathFilter`.
        /// May combine with `--exclude` / `--exclude-from`. Unreadable file or
        /// bad UTF-8 → clear non-zero error. Omit all path flags ⇒ identity.
        #[arg(long = "path-from", value_name = "FILE", action = clap::ArgAction::Append)]
        path_from: Vec<PathBuf>,
        /// Plan-only: compute filtered listing + counts; do **not** write `-o`
        /// (still takes `-o` as the planned path). Orthogonal to `--format`.
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// Allow overwrite of an existing `-o` via atomic replace. Without
        /// `--force`, existing `-o` → clear non-zero (≡ M1 refuse). No-op
        /// under `--dry-run` (never writes).
        #[arg(long = "force")]
        force: bool,
        /// Output format: `text` (default ≡ other ops stderr summary) or `json`
        /// (one object on stdout; exit codes format-independent)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
        /// Input `.cfdir` (`.cfidx` / wrong magic → clear non-zero)
        input: PathBuf,
    },

    /// List paths in a `.cfidx` / `.cfdir` listing (inventory only)
    ///
    /// Magic-dispatches like `mount` / `verify`: `.cfidx` → one logical file path
    /// (stem without `.cfidx`, same naming as mount); `.cfdir` → every **File** /
    /// **Dir** / **Symlink** entry present in the listing. Entries are printed in
    /// path lexicographic order. **Does not** open a store, fetch chunks, mount,
    /// extract, verify hashes, or write a new listing.
    ///
    /// Default **`--format text`**: one line per entry on stdout.
    /// Columns (tab-separated): `file\tpath\tsize` (optional 4th column of
    /// comma-joined chunk hex when `--chunks`); `dir\tpath`;
    /// `symlink\tpath\ttarget`. **`--format json`**: one object on stdout —
    /// `{ "ok": true, "entries": [ { "kind": "file"|"dir"|"symlink", "path": "…",
    /// "size"?: n, "target"?: "…", "chunks"?: ["hex…"] } ] }` (prep for ops-json;
    /// `size` on File; `target` on Symlink; `chunks` only when `--chunks` and
    /// kind=file). Exit codes are format-independent.
    ///
    /// **`--chunks`** (default **off**): when set, File entries include the chunk
    /// id list (lowercase hex); Symlink/Dir omit `chunks`. Still **no** store open
    /// (ids come from the listing decode only).
    ///
    /// Optional path 四件套 (`--path` / `--path-from` / `--exclude` /
    /// `--exclude-from`) scopes a `.cfdir` via the same [`PathFilter`] /
    /// [`filter_dir_archive`] rules as mount/filter (empty ≡ full listing;
    /// filter-same include/exclude/from semantics).
    /// `.cfidx` + any path/exclude flag → clear non-zero error (same contract as
    /// mount/verify/filter-on-cfidx). **≠** mount / **≠** extract / **≠** verify /
    /// **≠** pack / **≠** filter (read-only inventory; does not persist a subset).
    Ls {
        /// Include only `.cfdir` paths under this prefix (repeatable; OR).
        /// With any `--path`, a candidate must match at least one before
        /// excludes apply. Omit all `--path` ⇒ include-all (full listing).
        /// With `.cfidx` → clear non-zero error.
        #[arg(long = "path", value_name = "P", action = clap::ArgAction::Append)]
        paths: Vec<String>,
        /// Exclude `.cfdir` paths matching this pattern (repeatable): exact,
        /// trailing-`/` directory prefix, or single edge `*` (`*.o`, `temp*`).
        /// Illegal middle `*` / `**` → clear error. Applied after `--path`.
        /// With `.cfidx` → clear non-zero error.
        #[arg(long = "exclude", value_name = "PAT", action = clap::ArgAction::Append)]
        excludes: Vec<String>,
        /// Read exclude patterns from a UTF-8 file (repeatable). One pattern
        /// per line (same rules as `--exclude`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--exclude` into one `PathFilter`.
        /// Unreadable file or illegal pattern → clear non-zero error.
        /// With `.cfidx` → clear non-zero error.
        #[arg(long = "exclude-from", value_name = "FILE", action = clap::ArgAction::Append)]
        exclude_from: Vec<PathBuf>,
        /// Read include path prefixes from a UTF-8 file (repeatable). One prefix
        /// per line (same rules as `--path`); blank lines and `#` comments
        /// skipped; trim. Merged with every `--path` (OR) into one `PathFilter`.
        /// May combine with `--exclude` / `--exclude-from`. Unreadable file or
        /// bad UTF-8 → clear non-zero error. With `.cfidx` → clear non-zero error.
        #[arg(long = "path-from", value_name = "FILE", action = clap::ArgAction::Append)]
        path_from: Vec<PathBuf>,
        /// Output format: `text` (default; tab columns) or `json` (`ok`/`entries`)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
        /// Include File chunk id lists (hex). Default **off**. Symlink/Dir omit.
        /// Still no store open — ids come from listing decode only.
        #[arg(long = "chunks")]
        chunks: bool,
        /// Input `.cfidx` or `.cfdir`
        listing: PathBuf,
    },

    /// Query the local store
    Store {
        #[command(subcommand)]
        command: StoreCommands,
    },
}

#[derive(Debug, Subcommand)]
enum StoreCommands {
    /// Create an empty local CAS store (`Store::create`)
    ///
    /// Writes `meta.toml` + `chunks/` under `--store`. If `meta.toml` already
    /// exists → clear non-zero error (does **not** overwrite). Omit
    /// `--compression` ≡ `none` (≡ 1.8 create default). **Not** recompress /
    /// trim / change compression on an existing store. On-disk zstd is **not**
    /// HTTP wire compression / Content-Encoding.
    /// Default **`--format text`**: one stderr summary line.
    /// **`--format json`**: one JSON object on stdout
    /// (`ok` / `store` / `compression`); no duplicate text summary. Exit codes
    /// are format-independent.
    Create {
        /// Local CAS store directory to create
        #[arg(long)]
        store: PathBuf,
        /// On-disk chunk compression for this **new** store only (`none`|`zstd`;
        /// case-insensitive). Omit ≡ `none` (≡ 1.8). Disk zstd is **not** HTTP
        /// wire compression / Content-Encoding. Does not alter an existing store.
        #[arg(
            long = "compression",
            value_name = "none|zstd",
            value_parser = parse_cli_compression
        )]
        compression: Option<Compression>,
        /// Output format (default text; json = one object on stdout)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
    },
    /// Check whether a chunk id exists in the store
    ///
    /// Default **`--format text`**: `present\t<id>` on stdout (exit 0) or
    /// non-zero with `missing\t<id>`. **`--format json`**: one JSON object on
    /// stdout (`ok` / `present` / `id`); missing → `ok=false`, `present=false`,
    /// exit non-zero. Exit codes are format-independent.
    Has {
        /// Local CAS store directory
        #[arg(long)]
        store: PathBuf,
        /// Chunk id as 64 lowercase hex characters
        hex_id: String,
        /// Output format (default text; json = one object on stdout)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
    },
    /// Fetch one chunk's plaintext bytes from a local store (`Store::get` /
    /// `get_verify`)
    ///
    /// Writes plaintext to **`-o`** (required). Default path uses
    /// `Store::get_verify(id, false)` — decode on-disk encoding, **no** BLAKE3
    /// re-hash (trust disk). Optional **`--verify`** → `get_verify(id, true)`
    /// (re-hash). Default **`--format text`**: one stderr summary
    /// `store get: ok id=<hex> bytes=N`. **`--format json`**: one JSON object
    /// on stdout with field names **`ok` / `id` / `bytes`** (ops-json full doc
    /// may land in a later milestone; pin these names). Missing chunk / bad hex
    /// id → clear non-zero. **Not** `store scrub` / **not** `cat` / **not**
    /// extract / **not** recompress / **not** remove / **not** trim / **not**
    /// multi-id batch / **not** HTTP source.
    Get {
        /// Local CAS store directory
        #[arg(long)]
        store: PathBuf,
        /// Chunk id as 64 lowercase hex characters (same as `store has`)
        hex_id: String,
        /// Output file for plaintext bytes (required)
        #[arg(short = 'o', long = "output", value_name = "FILE")]
        output: PathBuf,
        /// Re-hash plaintext against the chunk id (`get_verify(..., true)`).
        /// Default off ≡ trust on-disk encoding (`get_verify(..., false)`).
        #[arg(long = "verify")]
        verify: bool,
        /// Output format (default text; json = `{ok,id,bytes}` on stdout)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
    },
    /// Rehash every loose chunk in a local store (bitrot / integrity scrub)
    ///
    /// Default (no `--listing`) ≡ 1.4.0 full-store traversal. With
    /// `--listing <index>`: only rehash chunk ids referenced by that
    /// `.cfidx` / `.cfdir` (local store only — **not** remote scrub).
    Scrub {
        /// Local CAS store directory
        #[arg(long)]
        store: PathBuf,
        /// Optional `.cfidx` / `.cfdir`: scrub only its referenced chunk ids
        /// (local store). Omit ≡ full-store list (≡ 1.4.0). Not remote scrub.
        #[arg(long = "listing", value_name = "INDEX")]
        listing: Option<PathBuf>,
        /// Parallel verify workers (default 1 = serial)
        #[arg(long, default_value_t = 1, value_name = "N")]
        jobs: u32,
        /// Output format (default text ≡ 1.1.0 scrub lines; json = one object on stdout)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
        /// Emit `progress: op=scrub done=N/TOTAL` on stderr per chunk
        /// (default off ≡ 1.1.0). Orthogonal to `--format json`.
        #[arg(long = "progress")]
        progress: bool,
    },
    /// Report local CAS chunk count and on-disk bytes (alias: `du`)
    ///
    /// Read-only observation via `Store::stats` / `stats_with_decode` — never
    /// deletes, trims, or applies LRU. Default **`--format text`**: one stdout
    /// summary line `store stats: chunks=N bytes_on_disk=M [bytes_plaintext=P]
    /// compression=none|zstd` (`bytes_plaintext` printed when known: always for
    /// `compression=none`, or for zstd only with `--decode`).
    /// **`--format json`**: one JSON object on stdout (`ok` / `chunks` /
    /// `bytes_on_disk` / `bytes_plaintext` / `compression`); `bytes_plaintext`
    /// is a number or `null`. No text dual-write. Exit codes are
    /// format-independent (success → 0).
    #[command(visible_alias = "du")]
    Stats {
        /// Local CAS store directory
        #[arg(long)]
        store: PathBuf,
        /// Output format (default text; json = one object on stdout)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
        /// Opt-in: for zstd stores, decode every chunk (`get`) and sum plaintext
        /// lengths into `bytes_plaintext`. For `compression=none` this is a
        /// no-op (`bytes_plaintext` already equals `bytes_on_disk`). Default off
        /// so stats stay cheap (no full-store decode).
        #[arg(long = "decode")]
        decode: bool,
    },
    /// List every loose chunk id in a local CAS store (`Store::list_chunk_ids`)
    ///
    /// Read-only enumeration — **not** GC / scrub / trim / LRU. Default
    /// **`--format text`**: one lowercase hex id per line, **stably sorted**.
    /// Empty store → no lines (exit 0). **`--format json`**: one JSON object on
    /// stdout (`ok` / `chunks` / `ids`); `ids` is a sorted string array; no text
    /// dual-write. Exit codes are format-independent (success → 0). Same
    /// `--store` convention as other `store` subcommands.
    List {
        /// Local CAS store directory
        #[arg(long)]
        store: PathBuf,
        /// Output format (default text = one hex id per line; json = one object)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
    },
}

/// Shared `--format text|json` for `diff` / `verify` / `doctor` / `extract` /
/// `push` / `pull` / `gc` / `store scrub` / `store stats` / `store create` /
/// `store list` / `store get` / `archive` / `make` / `cat` (Phase 8–12 + Phase 13 M2 + Phase 14 M2 +
/// Phase 15 M3/M4 + Phase 19 M1 + Phase 21 M6 + Phase26-M2).
/// Default `text` preserves prior behaviour (`diff` ≡ 0.7.0; `verify`/`doctor` ≡ 0.9.0;
/// `extract` / `push` / `pull` ≡ 1.0.0; `gc` / `store scrub` ≡ 1.1.0; `archive` ≡ 1.2.0;
/// `store stats` ≡ text summary; `make` ≡ 1.4.0 stderr summary; `cat` ≡ 1.4.0 almost silent;
/// `store list` ≡ sorted hex ids one-per-line; `store get` ≡ stderr summary + `-o` bytes).
/// How `archive` / `diff --tree` treat symbolic links (Phase22-M2 / Phase23-M1).
///
/// Default [`Skip`] ≡ 1.11/1.12 skip+warn (not recorded / not followed).
/// [`Record`] writes [`DirEntryKind::Symlink`] entries (targets as-is; not
/// followed); absolute or empty targets are rejected. Shared by archive and
/// `diff --tree` (listing↔listing needs no flag — library already compares).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
enum SymlinkPolicy {
    /// Skip + warn (≡ 1.11/1.12); do not record / do not follow
    #[default]
    Skip,
    /// Record Symlink into listing / ephemeral tree (not followed; absolute target → error)
    Record,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
enum CliFormat {
    /// Human / prior-stable text (stderr or stdout summaries as documented per command)
    #[default]
    Text,
    /// Single JSON object on stdout; exit codes unchanged vs text
    Json,
}

/// Optional HTTP URL / header templates and retry knobs for `cat` / `verify` /
/// `mount` / `doctor` / `push` / `pull` / `extract`.
///
/// Template flags (`--url-template` / `--prefix` / `--header`) and `--aws-sigv4`
/// apply isomorphically to every `http(s)://` origin in a read chain (primary
/// and each HTTP `--fallback`) and to an `http(s)://` `push --dest`. On read
/// chains they are no-ops for non-HTTP origins (error only when those flags are
/// set and the chain has no `http(s)://` source at all). On `push`, the same
/// flags (plus non-zero `--http-retries`) with a local/`file://` `--dest` are a
/// clear non-zero error. Omitting them preserves the Phase 2 default layout
/// (`{base}/chunks/<2hex>/<62hex>.cnk`).
///
/// `--http-retries` / `--http-retry-backoff-ms` apply only to HTTP(S) origins;
/// local `--store` / `file://` paths ignore them (no-op).
#[derive(Debug, Clone, clap::Args)]
struct HttpTemplateArgs {
    /// URL template expanded per chunk id (HTTP sources only; default `{base}/{path}`)
    #[arg(long = "url-template", value_name = "TMPL")]
    url_template: Option<String>,
    /// Key prefix for `{prefix}` in templates (HTTP only; normalized to `foo/` or empty)
    #[arg(long, value_name = "PREFIX")]
    prefix: Option<String>,
    /// Extra request header as `Name: value-template` (repeatable; HTTP only)
    #[arg(long = "header", value_name = "NAME: VALUE", action = clap::ArgAction::Append)]
    headers: Vec<String>,
    /// Extra HTTP attempts after the first try for transient failures (408/429/5xx,
    /// timeouts). Default **0** ≡ 0.7.0 single attempt. HTTP(S) only; ignored for
    /// local `--store` / `file://`.
    #[arg(long = "http-retries", default_value_t = 0, value_name = "N")]
    http_retries: u32,
    /// Base backoff in milliseconds for HTTP retries (exponential + jitter; capped
    /// at 2s). Default **100**. HTTP(S) only; ignored for local origins. Set `0`
    /// for tests / demos that want retries without sleep.
    #[arg(
        long = "http-retry-backoff-ms",
        default_value_t = 100,
        value_name = "MS"
    )]
    http_retry_backoff_ms: u64,
    /// Sign HTTP GET/HEAD/PUT with AWS SigV4 (AWS4-HMAC-SHA256). Default **off**
    /// ≡ 0.7.0 (no SigV4 headers). Credentials from env
    /// (`AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY`); if those are missing,
    /// fall back to the shared credentials file (`~/.aws/credentials` or
    /// `AWS_SHARED_CREDENTIALS_FILE`, default profile / `AWS_PROFILE`). Optional
    /// `AWS_SESSION_TOKEN` / `AWS_REGION` (default `us-east-1` with a warning).
    /// No IMDS/SSO/`aws-sdk-*`. Conflicts with `--header Authorization:…`.
    /// See `docs/sigv4.md`.
    #[arg(long = "aws-sigv4", default_value_t = false)]
    aws_sigv4: bool,
}

impl Default for HttpTemplateArgs {
    fn default() -> Self {
        Self {
            url_template: None,
            prefix: None,
            headers: Vec::new(),
            http_retries: 0,
            http_retry_backoff_ms: 100,
            aws_sigv4: false,
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Make {
            store,
            output,
            input,
            chunk_size,
            format,
            compression,
            progress,
            jobs,
            dry_run,
            seed,
            seed_trust_mtime,
        } => {
            let jobs = parse_jobs(jobs)?;
            cmd_make(
                &store,
                &output,
                &input,
                chunk_size.as_deref(),
                format,
                compression,
                progress,
                jobs,
                dry_run,
                seed.as_deref().map(|p| (p, seed_trust_mtime)),
            )
        }
        Commands::Archive {
            store,
            output,
            src_dir,
            chunk_size,
            dry_run,
            seed,
            seed_trust_mtime,
            jobs,
            paths,
            excludes,
            exclude_from,
            path_from,
            format,
            compression,
            progress,
            symlinks,
            empty_dirs,
        } => {
            let jobs = parse_jobs(jobs)?;
            let paths = merged_paths(&paths, &path_from)?;
            let excludes = merged_excludes(&excludes, &exclude_from)?;
            cmd_archive(
                &store,
                &output,
                &src_dir,
                chunk_size.as_deref(),
                dry_run,
                seed.as_deref().map(|p| (p, seed_trust_mtime)),
                jobs,
                &paths,
                &excludes,
                format,
                compression,
                progress,
                symlinks,
                empty_dirs,
            )
        }
        Commands::Extract {
            store,
            source,
            fallback,
            cache,
            cache_max_bytes,
            cache_stats,
            http_tmpl,
            jobs,
            archive,
            output,
            force,
            skip_unchanged,
            skip_trust_mtime,
            dry_run,
            format,
            paths,
            excludes,
            exclude_from,
            path_from,
            progress,
        } => {
            let jobs = parse_jobs(jobs)?;
            let paths = merged_paths(&paths, &path_from)?;
            let excludes = merged_excludes(&excludes, &exclude_from)?;
            let path_filter = PathFilter::new(paths.iter().cloned(), excludes.iter().cloned())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            require_cache_for_stats(cache.as_deref(), cache_stats)?;
            // Dry-run never opens store/source (no chunk get; G2 / §3.2).
            if dry_run {
                let result = cmd_extract(
                    None,
                    &archive,
                    &output,
                    jobs,
                    force,
                    skip_unchanged,
                    skip_trust_mtime,
                    true,
                    format,
                    &path_filter,
                    progress,
                    None,
                );
                maybe_emit_cache_stats(&None, cache_stats);
                result
            } else {
                let (src, stats) = open_chunk_source(
                    store.as_deref(),
                    source.as_deref(),
                    cache.as_deref(),
                    cache_max_bytes,
                    &http_tmpl,
                    &fallback,
                )?;
                let result = cmd_extract(
                    Some(src.as_ref()),
                    &archive,
                    &output,
                    jobs,
                    force,
                    skip_unchanged,
                    skip_trust_mtime,
                    false,
                    format,
                    &path_filter,
                    progress,
                    stats.as_ref(),
                );
                maybe_emit_cache_stats(&stats, cache_stats);
                result
            }
        }
        Commands::Cat {
            store,
            source,
            fallback,
            cache,
            cache_max_bytes,
            cache_stats,
            http_tmpl,
            jobs,
            format,
            path,
            index,
            output,
            progress,
        } => {
            let jobs = parse_jobs(jobs)?;
            require_cache_for_stats(cache.as_deref(), cache_stats)?;
            let (src, stats) = open_chunk_source(
                store.as_deref(),
                source.as_deref(),
                cache.as_deref(),
                cache_max_bytes,
                &http_tmpl,
                &fallback,
            )?;
            let result = cmd_cat(
                src.as_ref(),
                &index,
                &output,
                jobs,
                format,
                path.as_deref(),
                progress,
                stats.as_ref(),
            );
            maybe_emit_cache_stats(&stats, cache_stats);
            result
        }
        Commands::Verify {
            store,
            source,
            fallback,
            cache,
            cache_max_bytes,
            cache_stats,
            http_tmpl,
            jobs,
            format,
            progress,
            paths,
            excludes,
            exclude_from,
            path_from,
            index,
        } => {
            let jobs = parse_jobs(jobs)?;
            require_cache_for_stats(cache.as_deref(), cache_stats)?;
            let paths = merged_paths(&paths, &path_from)?;
            let excludes = merged_excludes(&excludes, &exclude_from)?;
            let path_filter = PathFilter::new(paths.iter().cloned(), excludes.iter().cloned())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let (src, stats) = open_chunk_source(
                store.as_deref(),
                source.as_deref(),
                cache.as_deref(),
                cache_max_bytes,
                &http_tmpl,
                &fallback,
            )?;
            let result = cmd_verify(
                src.as_ref(),
                &index,
                jobs,
                format,
                progress,
                stats.as_ref(),
                &path_filter,
            );
            maybe_emit_cache_stats(&stats, cache_stats);
            result
        }
        Commands::ChunkId {
            input,
            chunk_size,
            format,
        } => cmd_chunk_id(&input, chunk_size.as_deref(), format),
        Commands::Mount {
            store,
            source,
            fallback,
            cache,
            cache_max_bytes,
            cache_stats,
            http_tmpl,
            name,
            no_prefetch,
            prefetch_chunks,
            paths,
            excludes,
            exclude_from,
            path_from,
            index,
            mountpoint,
        } => {
            ensure_mount_supported()?;
            require_cache_for_stats(cache.as_deref(), cache_stats)?;
            let paths = merged_paths(&paths, &path_from)?;
            let excludes = merged_excludes(&excludes, &exclude_from)?;
            let path_filter = PathFilter::new(paths.iter().cloned(), excludes.iter().cloned())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let (src, stats) = open_chunk_source(
                store.as_deref(),
                source.as_deref(),
                cache.as_deref(),
                cache_max_bytes,
                &http_tmpl,
                &fallback,
            )?;
            let result = cmd_mount(
                src,
                &index,
                &mountpoint,
                name.as_deref(),
                !no_prefetch,
                prefetch_chunks as usize,
                &path_filter,
            );
            // After FUSE session ends (unmount), emit once.
            maybe_emit_cache_stats(&stats, cache_stats);
            result
        }
        Commands::Doctor {
            store,
            source,
            fallback,
            cache,
            cache_max_bytes,
            cache_stats,
            http_tmpl,
            jobs,
            deep,
            no_probe,
            format,
            progress,
            paths,
            excludes,
            exclude_from,
            path_from,
            indexes,
        } => {
            let jobs = parse_jobs(jobs)?;
            require_cache_for_stats(cache.as_deref(), cache_stats)?;
            let paths = merged_paths(&paths, &path_from)?;
            let excludes = merged_excludes(&excludes, &exclude_from)?;
            let path_filter = PathFilter::new(paths.iter().cloned(), excludes.iter().cloned())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let (src, stats) = open_chunk_source(
                store.as_deref(),
                source.as_deref(),
                cache.as_deref(),
                cache_max_bytes,
                &http_tmpl,
                &fallback,
            )?;
            let origin_spec = match (store.as_deref(), source.as_deref()) {
                (Some(path), None) => path.to_string_lossy().into_owned(),
                (None, Some(s)) => s.to_string(),
                _ => unreachable!("clap origin group requires exactly one of --store/--source"),
            };
            let result = cmd_doctor(
                src.as_ref(),
                &origin_spec,
                &indexes,
                deep,
                no_probe,
                jobs,
                http_tmpl.http_retries,
                format,
                progress,
                stats.as_ref(),
                &path_filter,
            );
            maybe_emit_cache_stats(&stats, cache_stats);
            result
        }
        Commands::Gc {
            store,
            apply,
            jobs,
            format,
            progress,
            indexes,
        } => {
            let jobs = parse_jobs(jobs)?;
            cmd_gc(&store, &indexes, apply, jobs, format, progress)
        }
        Commands::Push {
            store,
            dest,
            http_tmpl,
            jobs,
            dry_run,
            verify,
            format,
            progress,
            paths,
            excludes,
            exclude_from,
            path_from,
            compression,
            indexes,
        } => {
            let jobs = parse_jobs(jobs)?;
            let paths = merged_paths(&paths, &path_from)?;
            let excludes = merged_excludes(&excludes, &exclude_from)?;
            let path_filter = PathFilter::new(paths.iter().cloned(), excludes.iter().cloned())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            cmd_push(
                &store,
                &dest,
                &http_tmpl,
                dry_run,
                verify,
                &indexes,
                jobs,
                format,
                progress,
                &path_filter,
                compression,
            )
        }
        Commands::Pull {
            store,
            source,
            fallback,
            cache,
            cache_max_bytes,
            cache_stats,
            http_tmpl,
            jobs,
            dry_run,
            verify,
            format,
            compression,
            progress,
            paths,
            excludes,
            exclude_from,
            path_from,
            indexes,
        } => {
            let jobs = parse_jobs(jobs)?;
            let paths = merged_paths(&paths, &path_from)?;
            let excludes = merged_excludes(&excludes, &exclude_from)?;
            let path_filter = PathFilter::new(paths.iter().cloned(), excludes.iter().cloned())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            require_cache_for_stats(cache.as_deref(), cache_stats)?;
            cmd_pull(
                &store,
                &source,
                &fallback,
                cache.as_deref(),
                cache_max_bytes,
                cache_stats,
                &http_tmpl,
                dry_run,
                verify,
                &indexes,
                jobs,
                format,
                progress,
                &path_filter,
                compression,
            )
        }
        Commands::Diff {
            format,
            max_paths,
            tree,
            paths,
            excludes,
            exclude_from,
            path_from,
            progress,
            symlinks,
            left,
            right,
        } => {
            let paths = merged_paths(&paths, &path_from)?;
            let excludes = merged_excludes(&excludes, &exclude_from)?;
            let path_filter = PathFilter::new(paths.iter().cloned(), excludes.iter().cloned())
                .map_err(|e| anyhow::anyhow!("path filter: {e}"))?;
            cmd_diff_dispatch(
                tree.as_deref(),
                &left,
                right.as_deref(),
                max_paths,
                format,
                &path_filter,
                progress,
                symlinks,
            )
        }

        Commands::Filter {
            output,
            paths,
            excludes,
            exclude_from,
            path_from,
            dry_run,
            force,
            format,
            input,
        } => {
            let paths = merged_paths(&paths, &path_from)?;
            let excludes = merged_excludes(&excludes, &exclude_from)?;
            let path_filter = PathFilter::new(paths.iter().cloned(), excludes.iter().cloned())
                .map_err(|e| anyhow::anyhow!("path filter: {e}"))?;
            cmd_filter(&input, &output, &path_filter, dry_run, force, format)
        }

        Commands::Ls {
            paths,
            excludes,
            exclude_from,
            path_from,
            format,
            chunks,
            listing,
        } => {
            let paths = merged_paths(&paths, &path_from)?;
            let excludes = merged_excludes(&excludes, &exclude_from)?;
            let path_filter = PathFilter::new(paths.iter().cloned(), excludes.iter().cloned())
                .map_err(|e| anyhow::anyhow!("path filter: {e}"))?;
            cmd_ls(&listing, &path_filter, format, chunks)
        }

        Commands::Store {
            command:
                StoreCommands::Create {
                    store,
                    compression,
                    format,
                },
        } => cmd_store_create(&store, compression, format),
        Commands::Store {
            command:
                StoreCommands::Has {
                    store,
                    hex_id,
                    format,
                },
        } => cmd_store_has(&store, &hex_id, format),
        Commands::Store {
            command:
                StoreCommands::Get {
                    store,
                    hex_id,
                    output,
                    verify,
                    format,
                },
        } => cmd_store_get(&store, &hex_id, &output, verify, format),
        Commands::Store {
            command:
                StoreCommands::Scrub {
                    store,
                    listing,
                    jobs,
                    format,
                    progress,
                },
        } => {
            let jobs = parse_jobs(jobs)?;
            cmd_store_scrub(&store, listing.as_deref(), jobs, format, progress)
        }
        Commands::Store {
            command:
                StoreCommands::Stats {
                    store,
                    format,
                    decode,
                },
        } => cmd_store_stats(&store, format, decode),
        Commands::Store {
            command: StoreCommands::List { store, format },
        } => cmd_store_list(&store, format),
    }
}

/// CLI `--exclude` patterns ∪ every `--exclude-from` file (in flag order).
fn merged_excludes(cli: &[String], files: &[PathBuf]) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(cli.len());
    out.extend(cli.iter().cloned());
    for path in files {
        let more = load_exclude_file(path).map_err(|e| anyhow::anyhow!("{e}"))?;
        out.extend(more);
    }
    Ok(out)
}

/// CLI `--path` prefixes ∪ every `--path-from` file (in flag order).
/// Missing file / bad UTF-8 → non-zero via [`chunkforge_index::Error::PathFile`].
fn merged_paths(cli: &[String], files: &[PathBuf]) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(cli.len());
    out.extend(cli.iter().cloned());
    for path in files {
        let more = load_path_file(path).map_err(|e| anyhow::anyhow!("{e}"))?;
        out.extend(more);
    }
    Ok(out)
}

fn parse_jobs(jobs: u32) -> Result<usize> {
    if jobs == 0 {
        bail!("--jobs must be >= 1 (got 0); default 1 is serial / 0.3.0 behaviour");
    }
    Ok(jobs as usize)
}

fn parse_chunk_size(spec: Option<&str>) -> Result<ChunkParams> {
    let Some(spec) = spec else {
        return Ok(ChunkParams::default());
    };
    let parts: Vec<&str> = spec.split(':').collect();
    if parts.len() != 3 {
        bail!(
            "invalid --chunk-size {spec:?}: expected min:avg:max (three colon-separated integers)"
        );
    }
    let parse_u64 = |s: &str, label: &str| -> Result<u64> {
        s.trim()
            .parse::<u64>()
            .with_context(|| format!("invalid {label} in --chunk-size {spec:?}"))
    };
    let min = parse_u64(parts[0], "min")?;
    let avg = parse_u64(parts[1], "avg")?;
    let max = parse_u64(parts[2], "max")?;
    ChunkParams::new(min, avg, max).map_err(|e| anyhow::anyhow!("{e}"))
}

/// Parse `--compression none|zstd` (case-insensitive). Used as a clap
/// `value_parser`; clap wraps the field in `Option` so omit → `None` and
/// explicit `none` → `Some(Compression::None)` (distinguishable for mismatch).
fn parse_cli_compression(s: &str) -> std::result::Result<Compression, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "none" => Ok(Compression::None),
        "zstd" => Ok(Compression::Zstd),
        other => Err(format!("unknown compression {other:?}: expected none|zstd")),
    }
}

/// Open an existing store or create one at `root`.
///
/// `compression`:
/// - `None` — flag omitted: create with [`Compression::None`] (≡ 1.6); open existing
///   as recorded in `meta.toml` (no mismatch check).
/// - `Some(c)` — explicit request: create with `c`; open existing only if
///   `store.compression() == c`, else a clear non-zero error.
///
/// Cache stores should pass `None` (or `Some(Compression::None)` on create); there
/// is no `--cache-compression`. Mid-life recompress is out of scope.
fn open_or_create_store(root: &Path, compression: Option<Compression>) -> Result<Store> {
    let meta = root.join("meta.toml");
    if meta.is_file() {
        let store =
            Store::open(root).with_context(|| format!("open store at {}", root.display()))?;
        if let Some(requested) = compression {
            let actual = store.compression();
            if requested != actual {
                bail!(
                    "compression mismatch for store at {}: requested {}, store has {}",
                    root.display(),
                    requested,
                    actual
                );
            }
        }
        Ok(store)
    } else {
        let chosen = compression.unwrap_or(Compression::None);
        Store::create(root, chosen).with_context(|| format!("create store at {}", root.display()))
    }
}

/// Process-local handle to [`CacheSource`] observation counters (Phase18-M2).
///
/// Kept as an [`Arc`] twin of the boxed [`ChunkSource`] so counters remain
/// readable after the source is moved into command / FUSE code. Observation
/// only — **not** an LRU / trim / eviction control.
#[derive(Clone)]
struct CacheStatsRef {
    inner: Arc<CacheSource<Box<dyn ChunkSource>>>,
}

impl CacheStatsRef {
    fn hits(&self) -> u64 {
        self.inner.hits()
    }

    fn miss_fills(&self) -> u64 {
        self.inner.miss_fills()
    }

    fn miss_refused(&self) -> u64 {
        self.inner.miss_refused()
    }

    fn emit_stderr(&self) {
        eprintln!(
            "cache: hits={} miss_fills={} miss_refused={}",
            self.hits(),
            self.miss_fills(),
            self.miss_refused()
        );
    }
}

/// Bail when `--cache-stats` is set without `--cache` (same style as
/// `--cache-max-bytes requires --cache`).
fn require_cache_for_stats(cache: Option<&Path>, cache_stats: bool) -> Result<()> {
    if cache_stats && cache.is_none() {
        bail!("--cache-stats requires --cache <DIR>");
    }
    Ok(())
}

/// Additive ops-json fields from [`CacheStatsRef`] (Phase18-M3 / G3).
///
/// When `stats` is `Some` (`--cache` opened a [`CacheSource`]), insert
/// `cache_hits` / `cache_miss_fills` / `cache_miss_refused`. When `None`
/// (no `--cache`), **omit** the keys so the object shape stays the 1.7
/// baseline. Orthogonal to `--cache-stats` (stderr) and `--progress`.
/// Observation only — **≠** LRU / trim / eviction.
fn apply_cache_ops_json(obj: &mut serde_json::Value, stats: Option<&CacheStatsRef>) {
    let Some(s) = stats else {
        return;
    };
    let map = obj
        .as_object_mut()
        .expect("ops-json summary must be a JSON object");
    map.insert("cache_hits".to_string(), serde_json::json!(s.hits()));
    map.insert(
        "cache_miss_fills".to_string(),
        serde_json::json!(s.miss_fills()),
    );
    map.insert(
        "cache_miss_refused".to_string(),
        serde_json::json!(s.miss_refused()),
    );
}

/// Emit the one-line cache summary when `--cache-stats` was requested.
fn maybe_emit_cache_stats(stats: &Option<CacheStatsRef>, cache_stats: bool) {
    if cache_stats {
        if let Some(s) = stats {
            s.emit_stderr();
        } else {
            // Flag pair validated, but no CacheSource was constructed (e.g.
            // extract `--dry-run` never opens the origin). Report zeros.
            eprintln!("cache: hits=0 miss_fills=0 miss_refused=0");
        }
    }
}

/// RAII helper: emit cache-stats on drop (success or failure exit paths).
struct CacheStatsEmit<'a> {
    stats: &'a Option<CacheStatsRef>,
    enabled: bool,
}

impl Drop for CacheStatsEmit<'_> {
    fn drop(&mut self) {
        maybe_emit_cache_stats(self.stats, self.enabled);
    }
}

/// Resolve `--store` / `--source` / repeatable `--fallback` / `--cache` (+ optional
/// HTTP templates) into a boxed [`ChunkSource`] plus an optional [`CacheStatsRef`].
///
/// `--store PATH` is a Phase 1 synonym for `--source PATH` (local only).
/// Primary origin is still exactly one of `--store` / `--source` (mutually exclusive).
/// Each `--fallback` is opened in CLI order and chained behind the primary via
/// [`FallbackSource`] (Missing-only failover). Zero fallbacks ≡ 1.5 single-origin.
/// With `--cache`, an outer [`CacheSource`] wraps the **whole** fallback chain
/// (fill on miss; never write primary / fallbacks). The cache is wrapped in
/// [`Arc`] so the returned [`CacheStatsRef`] can read `hits` / `miss_fills` /
/// `miss_refused` after the boxed source is moved into command code (Phase18-M2).
/// `--cache-max-bytes SIZE` sets a soft fill budget (requires `--cache`; plain int or K/M/G/Ki/Mi/Gi); omit ≡ 1.4 unbounded.
/// `--url-template` / `--prefix` / `--header` / `--aws-sigv4` apply isomorphically to
/// every `http(s)://` origin in the chain (primary and HTTP fallbacks). They are
/// no-ops for non-HTTP origins; an error is raised only when those flags are set
/// and the chain has **no** `http(s)://` source at all.
fn open_chunk_source(
    store: Option<&Path>,
    source: Option<&str>,
    cache: Option<&Path>,
    cache_max_bytes: Option<u64>,
    http_tmpl: &HttpTemplateArgs,
    fallbacks: &[String],
) -> Result<(Box<dyn ChunkSource>, Option<CacheStatsRef>)> {
    if cache_max_bytes.is_some() && cache.is_none() {
        bail!("--cache-max-bytes requires --cache <DIR>");
    }

    let spec = match (store, source) {
        (Some(path), None) => path.to_string_lossy().into_owned(),
        (None, Some(s)) => s.to_string(),
        (Some(_), Some(_)) => bail!("use either --store or --source, not both"),
        (None, None) => bail!("missing chunk origin: pass --store <path> or --source <PATH|URL>"),
    };

    ensure_http_template_has_http_origin(&spec, fallbacks, http_tmpl)?;

    let primary = open_primary_source(&spec, http_tmpl)?;
    let chain: Box<dyn ChunkSource> = if fallbacks.is_empty() {
        primary
    } else {
        let mut sources = Vec::with_capacity(1 + fallbacks.len());
        sources.push(primary);
        for fb in fallbacks {
            sources.push(open_primary_source(fb, http_tmpl)?);
        }
        Box::new(FallbackSource::new(sources).map_err(|e| anyhow::anyhow!("{e}"))?)
    };

    match cache {
        None => Ok((chain, None)),
        Some(cache_path) => {
            let cache_store = open_or_create_store(cache_path, None)?;
            // `None` ≡ CacheSource::new (1.4 unbounded); Some(N) soft-refuses fill.
            // Outer Cache wraps the whole Fallback chain (Phase16-M2).
            // Arc keeps counter getters reachable after boxing (Phase18-M2).
            let cached = Arc::new(CacheSource::with_max_bytes(
                chain,
                cache_store,
                cache_max_bytes,
            ));
            let stats = CacheStatsRef {
                inner: Arc::clone(&cached),
            };
            Ok((Box::new(cached), Some(stats)))
        }
    }
}

fn is_http_spec(spec: &str) -> bool {
    let trimmed = spec.trim();
    trimmed.starts_with("http://") || trimmed.starts_with("https://")
}

/// Bail when HTTP-only template / SigV4 flags are set but neither primary nor
/// any `--fallback` is `http(s)://`. Retries alone are ignored for local origins.
fn ensure_http_template_has_http_origin(
    primary: &str,
    fallbacks: &[String],
    http_tmpl: &HttpTemplateArgs,
) -> Result<()> {
    if !http_template_flags_set(http_tmpl) {
        return Ok(());
    }
    if is_http_spec(primary) || fallbacks.iter().any(|s| is_http_spec(s)) {
        return Ok(());
    }
    bail!(
        "--url-template / --prefix / --header / --aws-sigv4 require at least one http(s):// source (primary or --fallback); got only non-HTTP origins"
    );
}

fn http_template_flags_set(http_tmpl: &HttpTemplateArgs) -> bool {
    http_tmpl.url_template.is_some()
        || http_tmpl.prefix.is_some()
        || !http_tmpl.headers.is_empty()
        || http_tmpl.aws_sigv4
}

/// Build a [`RetryPolicy`] from CLI HTTP retry flags.
///
/// `N` on `--http-retries` is **extra** attempts (0 → one try ≡ 0.7.0).
/// Base backoff comes from `--http-retry-backoff-ms` (default 100ms); max backoff
/// stays at the library default (2s).
fn retry_policy_from_http_args(http_tmpl: &HttpTemplateArgs) -> RetryPolicy {
    RetryPolicy {
        max_retries: http_tmpl.http_retries,
        base_backoff: Duration::from_millis(http_tmpl.http_retry_backoff_ms),
        max_backoff: Duration::from_secs(2),
    }
}

/// Build an optional [`SigV4Signer`] from `--aws-sigv4` + env credentials.
///
/// Returns `None` when the flag is off. Errors clearly when the flag is on but
/// credentials are missing, or when an `Authorization` header is also set.
fn sigv4_signer_from_http_args(http_tmpl: &HttpTemplateArgs) -> Result<Option<SigV4Signer>> {
    if !http_tmpl.aws_sigv4 {
        return Ok(None);
    }
    for raw in &http_tmpl.headers {
        let (name, _) = parse_header_flag(raw)?;
        if name.eq_ignore_ascii_case("Authorization") {
            bail!(
                "--aws-sigv4 conflicts with --header Authorization:…;                  omit the Authorization header or disable --aws-sigv4"
            );
        }
    }
    let (config, region_defaulted) = SigV4Config::from_env().map_err(|e| anyhow::anyhow!("{e}"))?;
    if region_defaulted {
        eprintln!(
            "warning: AWS_REGION unset; defaulting SigV4 region to {}              (set AWS_REGION to silence this warning)",
            config.region
        );
    }
    Ok(Some(SigV4Signer::new(config)))
}

/// Parse `--header 'Name: value-template'` into (name, value_template).
fn parse_header_flag(raw: &str) -> Result<(String, String)> {
    let Some((name, value)) = raw.split_once(':') else {
        bail!("invalid --header {raw:?}: expected 'Name: value-template' (colon-separated)");
    };
    let name = name.trim();
    let value = value.trim();
    if name.is_empty() {
        bail!("invalid --header {raw:?}: header name must not be empty");
    }
    Ok((name.to_string(), value.to_string()))
}

fn open_primary_source(spec: &str, http_tmpl: &HttpTemplateArgs) -> Result<Box<dyn ChunkSource>> {
    let trimmed = spec.trim();
    let is_http = is_http_spec(trimmed);

    // Template / SigV4 flags are no-ops for a single non-HTTP origin so that
    // `local primary + HTTP --fallback + --url-template` works. Chain-level
    // validation lives in `ensure_http_template_has_http_origin`.

    if is_http {
        let mut builder = HttpChunkSource::builder(trimmed)
            .timeout(Some(Duration::from_secs(30)))
            .retry_policy(retry_policy_from_http_args(http_tmpl));
        if let Some(ref tmpl) = http_tmpl.url_template {
            builder = builder.url_template(tmpl.clone());
        }
        if let Some(ref prefix) = http_tmpl.prefix {
            builder = builder.prefix(prefix.clone());
        }
        for raw in &http_tmpl.headers {
            let (name, value_tmpl) = parse_header_flag(raw)?;
            builder = builder.header(name, value_tmpl);
        }
        if let Some(signer) = sigv4_signer_from_http_args(http_tmpl)? {
            builder = builder.aws_sigv4(signer);
        }
        let src = builder.build().context("build HTTP chunk source")?;
        return Ok(Box::new(src));
    }

    // Local path or file:// — FileUrlSource / Store::open (must already exist).
    let src = FileUrlSource::open(trimmed)
        .with_context(|| format!("open chunk source {trimmed:?} (local path or file:// URL)"))?;
    Ok(Box::new(src))
}

#[allow(clippy::too_many_arguments)]
fn cmd_make(
    store_path: &Path,
    output: &Path,
    input: &Path,
    chunk_size: Option<&str>,
    format: CliFormat,
    compression: Option<Compression>,
    progress: bool,
    jobs: usize,
    dry_run: bool,
    seed: Option<(&Path, bool)>,
) -> Result<()> {
    let params = parse_chunk_size(chunk_size)?;

    let input_meta =
        fs::metadata(input).with_context(|| format!("stat input {}", input.display()))?;
    if !input_meta.is_file() {
        bail!(
            "make input {} is not a regular file (or is unreadable)",
            input.display()
        );
    }
    let source_size = input_meta.len();
    let source_mtime_secs = file_mtime_secs(&input_meta);

    // Load prior `.cfidx` for --seed (fail non-zero on bad magic / decode / .cfdir).
    let (prior_index, prior_mtime_secs, seed_trust_mtime, seeding) = match seed {
        Some((seed_path, trust)) => {
            let prior = load_seed_cfidx(seed_path)?;
            let prior_meta = fs::metadata(seed_path)
                .with_context(|| format!("stat seed {}", seed_path.display()))?;
            (Some(prior), file_mtime_secs(&prior_meta), trust, true)
        }
        None => (None, 0u64, false, false),
    };

    // Dry-run: open existing store for has() accounting only; do not create / put / write .cfidx.
    // `--compression` is ignored for create under dry-run (never creates).
    let store = if dry_run {
        let meta = store_path.join("meta.toml");
        if meta.is_file() {
            Some(
                Store::open(store_path)
                    .with_context(|| format!("open store at {}", store_path.display()))?,
            )
        } else {
            None
        }
    } else {
        Some(open_or_create_store(store_path, compression)?)
    };

    // Progress granularity (Phase17-M4 / G4): make always processes exactly one
    // input file → TOTAL=1 and a single tick after the file is fully chunked,
    // stored, and indexed. Dry-run / seed-Reuse ticks after accounting (no put
    // on dry-run; no FastCDC on Reuse).
    let prog = ProgressReporter::new(progress, "make", Some(1));

    // --- Seed reuse path (size fast-reject + BLAKE3 / optional trust-mtime) ---
    if let Some(ref prior) = prior_index {
        let mut f =
            File::open(input).with_context(|| format!("open {} for seed hash", input.display()))?;
        let decision = decide_seed_trust_mtime(
            prior.total_size,
            &prior.blob_blake3,
            prior_mtime_secs,
            source_size,
            source_mtime_secs,
            &mut f,
            seed_trust_mtime,
        )
        .map_err(|e| anyhow::anyhow!("seed decide for {}: {e}", input.display()))?;

        if decision == SeedDecision::Reuse {
            // Prior chunks must be present in store — clear non-zero if missing
            // (do not silently invent / do not fall through to inventing).
            let missing: Vec<String> = match store.as_ref() {
                Some(s) => prior
                    .entries
                    .iter()
                    .filter(|e| !s.has(&e.chunk_id))
                    .map(|e| e.chunk_id.to_string())
                    .collect(),
                None => prior
                    .entries
                    .iter()
                    .map(|e| e.chunk_id.to_string())
                    .collect(),
            };
            if !missing.is_empty() {
                bail!(
                    "make: seed reuse: missing {} chunk{} in store for {} \
                     (refusing to invent; first missing: {}). Restore the store \
                     or omit --seed to rechunk.",
                    missing.len(),
                    if missing.len() == 1 { "" } else { "s" },
                    input.display(),
                    missing.first().map(|s| s.as_str()).unwrap_or("?"),
                );
            }

            let chunk_n = prior.entries.len();
            prog.tick();

            if dry_run {
                match format {
                    CliFormat::Text => {
                        eprintln!(
                            "make: dry-run: {} bytes, {} chunk{} (would_write=0, would_reuse={}; \
                             seed_reused=true); no store/.cfidx written (would write {})",
                            source_size,
                            chunk_n,
                            if chunk_n == 1 { "" } else { "s" },
                            chunk_n,
                            output.display()
                        );
                    }
                    CliFormat::Json => {
                        let obj = serde_json::json!({
                            "ok": true,
                            "dry_run": true,
                            "bytes": source_size,
                            "chunks": chunk_n,
                            "would_write": 0usize,
                            "would_reuse": chunk_n,
                            "seed_reused": true,
                        });
                        println!("{obj}");
                    }
                }
                return Ok(());
            }

            let store = store.expect("non-dry-run make always opens or creates the store");
            let mut flags = prior.flags;
            if !matches!(store.compression(), Compression::None) {
                flags |= FLAG_CHUNKS_COMPRESSED_IN_STORE;
            } else {
                flags &= !FLAG_CHUNKS_COMPRESSED_IN_STORE;
            }

            let index = if chunk_n == 0 {
                let mut empty = Index::empty(prior.params);
                empty.flags = flags;
                empty
            } else {
                Index::new(
                    flags,
                    prior.params,
                    prior.total_size,
                    prior.blob_blake3,
                    prior.entries.clone(),
                )
                .map_err(|e| anyhow::anyhow!("build index from seed: {e}"))?
            };

            if let Some(parent) = output.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("create parent dir {}", parent.display()))?;
                }
            }
            let mut file = File::create(output)
                .with_context(|| format!("create index {}", output.display()))?;
            index
                .write_to(&mut file)
                .map_err(|e| anyhow::anyhow!("write index: {e}"))?;
            file.sync_all()
                .with_context(|| format!("fsync index {}", output.display()))?;

            match format {
                CliFormat::Text => {
                    eprintln!(
                        "make: wrote {} ({} bytes, {} chunk{}; new=0, reused={}; \
                         seed_reused=true) → store {}",
                        output.display(),
                        source_size,
                        chunk_n,
                        if chunk_n == 1 { "" } else { "s" },
                        chunk_n,
                        store_path.display()
                    );
                }
                CliFormat::Json => {
                    let obj = serde_json::json!({
                        "ok": true,
                        "bytes": source_size,
                        "chunks": chunk_n,
                        "new": 0usize,
                        "reused": chunk_n,
                        "seed_reused": true,
                    });
                    println!("{obj}");
                }
            }
            return Ok(());
        }
        // SeedDecision::Rechunk → fall through to FastCDC path below.
    }

    // FastCDC cut-points stay serial (StreamCDC / single-file). `--jobs` only
    // parallelizes post-chunk store put / on-disk encoding (zstd).
    let data = fs::read(input).with_context(|| format!("read input {}", input.display()))?;
    let chunks = chunk_bytes(&data, &params);
    let blob_blake3 = ChunkId::hash(&data);

    if dry_run {
        let mut would_write = 0usize;
        let mut would_reuse = 0usize;
        let mut seen: HashSet<ChunkId> = HashSet::new();
        for c in &chunks {
            let exists_in_store = store.as_ref().is_some_and(|s| s.has(&c.id));
            if exists_in_store || seen.contains(&c.id) {
                would_reuse += 1;
            } else {
                seen.insert(c.id);
                would_write += 1;
            }
        }
        prog.tick();
        match format {
            CliFormat::Text => {
                if seeding {
                    eprintln!(
                        "make: dry-run: {} bytes, {} chunk{} (would_write={}, would_reuse={}; \
                         seed_reused=false); no store/.cfidx written (would write {})",
                        data.len(),
                        chunks.len(),
                        if chunks.len() == 1 { "" } else { "s" },
                        would_write,
                        would_reuse,
                        output.display()
                    );
                } else {
                    eprintln!(
                        "make: dry-run: {} bytes, {} chunk{} (would_write={}, would_reuse={}); \
                         no store/.cfidx written (would write {})",
                        data.len(),
                        chunks.len(),
                        if chunks.len() == 1 { "" } else { "s" },
                        would_write,
                        would_reuse,
                        output.display()
                    );
                }
            }
            CliFormat::Json => {
                // Mirror archive dry-run: would_* instead of new/reused (avoid
                // misleading written-field names on plan-only).
                let mut obj = serde_json::json!({
                    "ok": true,
                    "dry_run": true,
                    "bytes": data.len() as u64,
                    "chunks": chunks.len(),
                    "would_write": would_write,
                    "would_reuse": would_reuse,
                });
                if seeding {
                    obj.as_object_mut()
                        .expect("object")
                        .insert("seed_reused".into(), serde_json::json!(false));
                }
                println!("{obj}");
            }
        }
        return Ok(());
    }

    let store = store.expect("non-dry-run make always opens or creates the store");

    // Store puts are atomic / race-safe (`Store::put_with_id`). map_indexed
    // preserves chunk order into `entries` (jobs=1 ≡ prior serial loop).
    let put_results = parallel::map_indexed(
        &chunks,
        jobs,
        |_idx, c| -> Result<(PutOutcome, IndexEntry)> {
            let start = c.offset as usize;
            let end = (c.offset + c.length) as usize;
            let slice = data
                .get(start..end)
                .with_context(|| format!("chunk range {start}..{end} out of bounds"))?;
            let outcome = store
                .put_with_id(&c.id, slice)
                .with_context(|| format!("put chunk {}", c.id))?;
            Ok((
                outcome,
                IndexEntry {
                    end_offset: c.offset + c.length,
                    chunk_id: c.id,
                },
            ))
        },
    );

    let mut entries = Vec::with_capacity(chunks.len());
    let mut new_chunks = 0usize;
    let mut reused_chunks = 0usize;
    for r in put_results {
        let (outcome, entry) = r?;
        match outcome {
            PutOutcome::Written => new_chunks += 1,
            PutOutcome::SkippedExists => reused_chunks += 1,
        }
        entries.push(entry);
    }

    let mut flags = 0u16;
    if !matches!(store.compression(), Compression::None) {
        flags |= FLAG_CHUNKS_COMPRESSED_IN_STORE;
    }

    let index = if chunks.is_empty() {
        let mut empty = Index::empty(params);
        empty.flags = flags;
        empty
    } else {
        Index::new(flags, params, data.len() as u64, blob_blake3, entries)
            .map_err(|e| anyhow::anyhow!("build index: {e}"))?
    };

    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create parent dir {}", parent.display()))?;
        }
    }
    let mut file =
        File::create(output).with_context(|| format!("create index {}", output.display()))?;
    index
        .write_to(&mut file)
        .map_err(|e| anyhow::anyhow!("write index: {e}"))?;
    file.sync_all()
        .with_context(|| format!("fsync index {}", output.display()))?;

    prog.tick();

    match format {
        CliFormat::Text => {
            if seeding {
                eprintln!(
                    "make: wrote {} ({} bytes, {} chunk{}; new={}, reused={}; \
                     seed_reused=false) → store {}",
                    output.display(),
                    data.len(),
                    chunks.len(),
                    if chunks.len() == 1 { "" } else { "s" },
                    new_chunks,
                    reused_chunks,
                    store_path.display()
                );
            } else {
                eprintln!(
                    "make: wrote {} ({} bytes, {} chunk{}; new={}, reused={}) → store {}",
                    output.display(),
                    data.len(),
                    chunks.len(),
                    if chunks.len() == 1 { "" } else { "s" },
                    new_chunks,
                    reused_chunks,
                    store_path.display()
                );
            }
        }
        CliFormat::Json => {
            let mut obj = serde_json::json!({
                "ok": true,
                "bytes": data.len() as u64,
                "chunks": chunks.len(),
                "new": new_chunks,
                "reused": reused_chunks,
            });
            if seeding {
                obj.as_object_mut()
                    .expect("object")
                    .insert("seed_reused".into(), serde_json::json!(false));
            }
            println!("{obj}");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_archive(
    store_path: &Path,
    output: &Path,
    src_dir: &Path,
    chunk_size: Option<&str>,
    dry_run: bool,
    seed: Option<(&Path, bool)>,
    jobs: usize,
    paths: &[String],
    excludes: &[String],
    format: CliFormat,
    compression: Option<Compression>,
    progress: bool,
    symlinks: SymlinkPolicy,
    empty_dirs: bool,
) -> Result<()> {
    let params = parse_chunk_size(chunk_size)?;

    if !src_dir.is_dir() {
        bail!(
            "archive source {} is not a directory (or is unreadable)",
            src_dir.display()
        );
    }

    let path_filter = PathFilter::new(paths.iter().cloned(), excludes.iter().cloned())
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let seed_trust_mtime = seed.map(|(_, t)| t).unwrap_or(false);

    // Load prior `.cfdir` for --seed (fail non-zero on bad magic / decode / .cfidx).
    let prior_arch = match seed {
        Some((seed_path, _)) => Some(load_seed_cfdir(seed_path)?),
        None => None,
    };
    let seed_map: Option<HashMap<&str, &DirEntry>> =
        prior_arch.as_ref().map(|arch| seed_file_map(arch));
    let seeding = seed_map.is_some();

    // Dry-run: open existing store for has() accounting only; do not create / put / write .cfdir.
    let store = if dry_run {
        let meta = store_path.join("meta.toml");
        if meta.is_file() {
            Some(
                Store::open(store_path)
                    .with_context(|| format!("open store at {}", store_path.display()))?,
            )
        } else {
            None
        }
    } else {
        Some(open_or_create_store(store_path, compression)?)
    };

    let mut flags = 0u16;
    if let Some(ref store) = store {
        if !matches!(store.compression(), Compression::None) {
            flags |= FLAG_CHUNKS_COMPRESSED_IN_STORE;
        }
    }

    // Collect candidates (sorted) so .cfdir output is deterministic.
    // Order: discover (type skip for special; symlink skip-or-record per policy;
    // optional empty leaf dirs when `--empty-dirs`), then PathFilter.
    // Symlink-to-dir is never followed.
    let mut file_paths: Vec<PathBuf> = Vec::new();
    let mut symlink_candidates: Vec<ArchiveSymlinkCandidate> = Vec::new();
    let mut empty_dir_candidates: Vec<ArchiveEmptyDirCandidate> = Vec::new();
    let mut skipped_symlinks = 0usize;
    let mut skipped_special = 0usize;
    collect_archive_files(
        src_dir,
        src_dir,
        &mut file_paths,
        &mut symlink_candidates,
        &mut empty_dir_candidates,
        &mut skipped_symlinks,
        &mut skipped_special,
        symlinks,
        empty_dirs,
    )?;
    file_paths.sort();
    symlink_candidates.sort_by(|a, b| a.full.cmp(&b.full));
    empty_dir_candidates.sort_by(|a, b| a.full.cmp(&b.full));

    let mut excluded = 0usize;
    let mut kept: Vec<PathBuf> = Vec::with_capacity(file_paths.len());
    for full in file_paths {
        let rel = relative_archive_path(src_dir, &full)?;
        validate_archive_path(&rel)
            .map_err(|e| anyhow::anyhow!("invalid archive path {rel:?}: {e}"))?;
        if path_filter.allows(&rel) {
            kept.push(full);
        } else {
            excluded += 1;
        }
    }
    let file_paths = kept;

    let mut kept_symlinks: Vec<ArchiveSymlinkCandidate> =
        Vec::with_capacity(symlink_candidates.len());
    for cand in symlink_candidates {
        let rel = relative_archive_path(src_dir, &cand.full)?;
        validate_archive_path(&rel)
            .map_err(|e| anyhow::anyhow!("invalid archive path {rel:?}: {e}"))?;
        if path_filter.allows(&rel) {
            kept_symlinks.push(cand);
        } else {
            excluded += 1;
        }
    }

    // Empty leaf dirs: same PathFilter as files (candidate only when --empty-dirs).
    let mut kept_empty_dirs: Vec<ArchiveEmptyDirCandidate> =
        Vec::with_capacity(empty_dir_candidates.len());
    for cand in empty_dir_candidates {
        let rel = relative_archive_path(src_dir, &cand.full)?;
        validate_archive_path(&rel)
            .map_err(|e| anyhow::anyhow!("invalid archive path {rel:?}: {e}"))?;
        if path_filter.allows(&rel) {
            kept_empty_dirs.push(cand);
        } else {
            excluded += 1;
        }
    }

    // Absolute / empty targets: reject only kept (path-filtered) symlink candidates.
    for cand in &kept_symlinks {
        let rel = relative_archive_path(src_dir, &cand.full)?;
        if cand.target.is_empty() {
            bail!("archive: symlink {rel} has empty target (refusing to record)");
        }
        if Path::new(&cand.target).is_absolute() {
            bail!(
                "archive: symlink {rel} has absolute target {:?} (refusing; use a relative target)",
                cand.target
            );
        }
    }

    if format == CliFormat::Text {
        match symlinks {
            SymlinkPolicy::Skip if skipped_symlinks > 0 || skipped_special > 0 => {
                eprintln!(
                    "archive: symlink policy = skip+warn (not recorded / not followed); \
                     skipped {skipped_symlinks} symlink{}, {skipped_special} special (fifo/socket/device)",
                    if skipped_symlinks == 1 { "" } else { "s" },
                );
            }
            SymlinkPolicy::Record => {
                if !kept_symlinks.is_empty() {
                    eprintln!(
                        "archive: symlink policy = record (not followed); recorded {} symlink{}",
                        kept_symlinks.len(),
                        if kept_symlinks.len() == 1 { "" } else { "s" },
                    );
                }
                if skipped_special > 0 {
                    eprintln!("archive: skipped {skipped_special} special (fifo/socket/device)");
                }
            }
            SymlinkPolicy::Skip => {}
        }
    }

    // Dry-run cross-file dedup set (Mutex so --jobs > 1 stays correct).
    let dry_seen: Mutex<HashSet<ChunkId>> = Mutex::new(HashSet::new());

    let seed_ctx = seed_map.as_ref().map(|m| (m, seed_trust_mtime));
    // Progress is per filtered File (PathFilter after type-skip); TOTAL known.
    let prog = ProgressReporter::new(progress, "archive", Some(file_paths.len()));
    let outcomes = parallel::map_indexed(&file_paths, jobs, |_idx, full| {
        let outcome = archive_one_file(
            src_dir,
            full,
            &params,
            store.as_ref(),
            seed_ctx,
            dry_run,
            &dry_seen,
        );
        prog.tick();
        outcome
    });

    let mut entries: Vec<DirEntry> = Vec::with_capacity(file_paths.len());
    let mut total_chunks = 0usize;
    let mut new_chunks = 0usize;
    let mut reused_chunks = 0usize;
    let mut seed_reused_files = 0usize;
    let mut rechunked_files = 0usize;
    let mut seed_missing_chunks = 0usize;

    for outcome in outcomes {
        let outcome = outcome?;
        total_chunks += outcome.chunk_count;
        new_chunks += outcome.new_chunks;
        reused_chunks += outcome.reused_chunks;
        if outcome.seed_reused {
            seed_reused_files += 1;
        }
        if outcome.rechunked {
            rechunked_files += 1;
        }
        if outcome.seed_missing {
            seed_missing_chunks += 1;
        }
        if let Some(entry) = outcome.entry {
            entries.push(entry);
        }
    }

    // Append recorded Symlink entries (0 chunks; never seed-reused).
    for cand in &kept_symlinks {
        let rel = relative_archive_path(src_dir, &cand.full)?;
        entries.push(DirEntry {
            path: rel,
            kind: DirEntryKind::Symlink {
                mode: cand.mode,
                target: cand.target.clone(),
            },
        });
    }
    // Append empty leaf Dir entries when `--empty-dirs` (0 chunks; omit ≡ 1.14).
    for cand in &kept_empty_dirs {
        let rel = relative_archive_path(src_dir, &cand.full)?;
        entries.push(DirEntry {
            path: rel,
            kind: DirEntryKind::Dir { mode: cand.mode },
        });
    }
    // Deterministic listing order: File / Dir / Symlink interleaved by path.
    entries.sort_by(|a, b| a.path.cmp(&b.path));

    let recorded_symlinks = kept_symlinks.len();
    let file_count = entries
        .iter()
        .filter(|e| matches!(e.kind, DirEntryKind::File { .. }))
        .count();
    // Default omit empty dirs ⇒ dirs usually 0 (≡ 1.14); `--empty-dirs` can be >0.
    let dir_count = entries
        .iter()
        .filter(|e| matches!(e.kind, DirEntryKind::Dir { .. }))
        .count();

    if dry_run {
        match format {
            CliFormat::Text => {
                if seeding {
                    eprintln!(
                        "archive: dry-run: {} file{}, {} chunk{} (would_write={}, would_reuse={}; \
                         would_seed_reuse={}, would_rechunk={}{}; excluded={}); \
                         no store/.cfdir written (would write {})",
                        file_count,
                        if file_count == 1 { "" } else { "s" },
                        total_chunks,
                        if total_chunks == 1 { "" } else { "s" },
                        new_chunks,
                        reused_chunks,
                        seed_reused_files,
                        rechunked_files,
                        if seed_missing_chunks > 0 {
                            format!(", seed_missing_chunks={seed_missing_chunks}")
                        } else {
                            String::new()
                        },
                        excluded,
                        output.display()
                    );
                } else {
                    eprintln!(
                        "archive: dry-run: {} file{}, {} chunk{} (would_write={}, would_reuse={}; \
                         excluded={}); no store/.cfdir written (would write {})",
                        file_count,
                        if file_count == 1 { "" } else { "s" },
                        total_chunks,
                        if total_chunks == 1 { "" } else { "s" },
                        new_chunks,
                        reused_chunks,
                        excluded,
                        output.display()
                    );
                }
            }
            CliFormat::Json => {
                let obj = serde_json::json!({
                    "ok": true,
                    "dry_run": true,
                    "files": file_count,
                    "dirs": dir_count,
                    "chunks": total_chunks,
                    "would_write": new_chunks,
                    "would_reuse": reused_chunks,
                    "seed_reused_files": seed_reused_files,
                    "rechunked_files": rechunked_files,
                    "skipped_symlinks": skipped_symlinks,
                    "skipped_special": skipped_special,
                    "recorded_symlinks": recorded_symlinks,
                    "excluded": excluded,
                });
                println!("{obj}");
            }
        }
        return Ok(());
    }

    let archive =
        DirArchive::new(flags, entries).map_err(|e| anyhow::anyhow!("build .cfdir: {e}"))?;

    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create parent dir {}", parent.display()))?;
        }
    }
    let mut file =
        File::create(output).with_context(|| format!("create archive {}", output.display()))?;
    archive
        .write_to(&mut file)
        .map_err(|e| anyhow::anyhow!("write .cfdir: {e}"))?;
    file.sync_all()
        .with_context(|| format!("fsync archive {}", output.display()))?;

    match format {
        CliFormat::Text => {
            if seeding {
                eprintln!(
                    "archive: wrote {} ({} file{}, {} chunk{}; new={}, reused={}; \
                     seed_reused_files={}, rechunked_files={}{}; excluded={}) → store {}",
                    output.display(),
                    file_count,
                    if file_count == 1 { "" } else { "s" },
                    total_chunks,
                    if total_chunks == 1 { "" } else { "s" },
                    new_chunks,
                    reused_chunks,
                    seed_reused_files,
                    rechunked_files,
                    if seed_missing_chunks > 0 {
                        format!(", seed_missing_chunks={seed_missing_chunks}")
                    } else {
                        String::new()
                    },
                    excluded,
                    store_path.display()
                );
            } else {
                eprintln!(
                    "archive: wrote {} ({} file{}, {} chunk{}; new={}, reused={}; excluded={}) → store {}",
                    output.display(),
                    file_count,
                    if file_count == 1 { "" } else { "s" },
                    total_chunks,
                    if total_chunks == 1 { "" } else { "s" },
                    new_chunks,
                    reused_chunks,
                    excluded,
                    store_path.display()
                );
            }
        }
        CliFormat::Json => {
            let obj = serde_json::json!({
                "ok": true,
                "dry_run": false,
                "files": file_count,
                "dirs": dir_count,
                "chunks": total_chunks,
                "written": new_chunks,
                "reused": reused_chunks,
                "seed_reused_files": seed_reused_files,
                "rechunked_files": rechunked_files,
                "skipped_symlinks": skipped_symlinks,
                "skipped_special": skipped_special,
                "recorded_symlinks": recorded_symlinks,
                "excluded": excluded,
            });
            println!("{obj}");
        }
    }
    Ok(())
}

/// Per-file result from [`archive_one_file`] (order-preserving aggregation).
struct ArchiveFileOutcome {
    entry: Option<DirEntry>,
    chunk_count: usize,
    new_chunks: usize,
    reused_chunks: usize,
    seed_reused: bool,
    rechunked: bool,
    seed_missing: bool,
}

/// Process one source file for `archive` (seed reuse or FastCDC + put).
///
/// Seed map is read-only. Store puts are atomic / race-safe (`Store::put_with_id`).
/// `dry_seen` tracks would-write ids across files under `--dry-run`.
fn archive_one_file(
    src_dir: &Path,
    full: &Path,
    params: &ChunkParams,
    store: Option<&Store>,
    seed: Option<(&HashMap<&str, &DirEntry>, bool)>,
    dry_run: bool,
    dry_seen: &Mutex<HashSet<ChunkId>>,
) -> Result<ArchiveFileOutcome> {
    let rel = relative_archive_path(src_dir, full)?;
    validate_archive_path(&rel)
        .map_err(|e| anyhow::anyhow!("invalid archive path {rel:?}: {e}"))?;

    let meta = fs::symlink_metadata(full).with_context(|| format!("stat {}", full.display()))?;
    if meta.file_type().is_symlink() {
        eprintln!(
            "archive: skip symlink {} (policy: skip+warn; not recorded)",
            full.display()
        );
        return Ok(ArchiveFileOutcome {
            entry: None,
            chunk_count: 0,
            new_chunks: 0,
            reused_chunks: 0,
            seed_reused: false,
            rechunked: false,
            seed_missing: false,
        });
    }
    if !meta.is_file() {
        eprintln!(
            "archive: skip special file {} (fifo/socket/device)",
            full.display()
        );
        return Ok(ArchiveFileOutcome {
            entry: None,
            chunk_count: 0,
            new_chunks: 0,
            reused_chunks: 0,
            seed_reused: false,
            rechunked: false,
            seed_missing: false,
        });
    }

    let source_size = meta.len();
    let source_mtime_secs = file_mtime_secs(&meta);

    // --- Seed reuse path -------------------------------------------------
    if let Some((map, seed_trust_mtime)) = seed {
        if let Some(prior_entry) = map.get(rel.as_str()) {
            let mut f = File::open(full)
                .with_context(|| format!("open {} for seed hash", full.display()))?;
            let decision = decide_seed_for_entry_ex(
                prior_entry,
                source_size,
                source_mtime_secs,
                &mut f,
                seed_trust_mtime,
            )
            .map_err(|e| anyhow::anyhow!("seed decide for {rel}: {e}"))?;
            if decision == SeedDecision::Reuse {
                let prior_chunks = match &prior_entry.kind {
                    DirEntryKind::File { chunks, .. } => chunks.as_slice(),
                    DirEntryKind::Dir { .. } | DirEntryKind::Symlink { .. } => {
                        unreachable!("seed map is files only")
                    }
                };
                let all_present = match store {
                    Some(s) => prior_chunks.iter().all(|e| s.has(&e.chunk_id)),
                    // Dry-run with no store: cannot verify → force rechunk.
                    None => false,
                };
                if all_present {
                    let chunk_n = prior_chunks.len();
                    return Ok(ArchiveFileOutcome {
                        entry: Some(DirEntry {
                            path: rel,
                            kind: prior_entry.kind.clone(),
                        }),
                        chunk_count: chunk_n,
                        new_chunks: 0,
                        reused_chunks: chunk_n,
                        seed_reused: true,
                        rechunked: false,
                        seed_missing: false,
                    });
                }
                let missing: Vec<_> = match store {
                    Some(s) => prior_chunks
                        .iter()
                        .filter(|e| !s.has(&e.chunk_id))
                        .map(|e| e.chunk_id.to_string())
                        .collect(),
                    None => prior_chunks
                        .iter()
                        .map(|e| e.chunk_id.to_string())
                        .collect(),
                };
                eprintln!(
                    "archive: seed: missing {} chunk{} in store for {rel} \
                     (rechunking; first missing: {})",
                    missing.len(),
                    if missing.len() == 1 { "" } else { "s" },
                    missing.first().map(|s| s.as_str()).unwrap_or("?"),
                );
                // Fall through to rechunk; mark seed_missing below.
                let outcome =
                    archive_rechunk_file(full, &rel, &meta, params, store, dry_run, dry_seen)?;
                return Ok(ArchiveFileOutcome {
                    seed_missing: true,
                    ..outcome
                });
            }
        }
    }

    archive_rechunk_file(full, &rel, &meta, params, store, dry_run, dry_seen)
}

fn archive_rechunk_file(
    full: &Path,
    rel: &str,
    meta: &std::fs::Metadata,
    params: &ChunkParams,
    store: Option<&Store>,
    dry_run: bool,
    dry_seen: &Mutex<HashSet<ChunkId>>,
) -> Result<ArchiveFileOutcome> {
    let data = fs::read(full).with_context(|| format!("read {}", full.display()))?;
    let mode = file_mode_u32(meta);
    let mtime_secs = file_mtime_secs(meta);
    let blob_blake3 = ChunkId::hash(&data);
    let chunks = chunk_bytes(&data, params);

    let mut index_entries = Vec::with_capacity(chunks.len());
    let mut new_chunks = 0usize;
    let mut reused_chunks = 0usize;
    for c in &chunks {
        let start = c.offset as usize;
        let end = (c.offset + c.length) as usize;
        let slice = data.get(start..end).with_context(|| {
            format!(
                "chunk range {start}..{end} out of bounds in {}",
                full.display()
            )
        })?;
        if dry_run {
            let exists_in_store = store.is_some_and(|s| s.has(&c.id));
            let mut seen = dry_seen.lock().expect("dry_seen lock");
            if exists_in_store || seen.contains(&c.id) {
                reused_chunks += 1;
            } else {
                seen.insert(c.id);
                new_chunks += 1;
            }
        } else {
            let store = store.expect("store open when not dry-run");
            let outcome = store
                .put_with_id(&c.id, slice)
                .with_context(|| format!("put chunk {} (file {rel})", c.id))?;
            match outcome {
                PutOutcome::Written => new_chunks += 1,
                PutOutcome::SkippedExists => reused_chunks += 1,
            }
        }
        index_entries.push(IndexEntry {
            end_offset: c.offset + c.length,
            chunk_id: c.id,
        });
    }

    Ok(ArchiveFileOutcome {
        entry: Some(DirEntry {
            path: rel.to_string(),
            kind: DirEntryKind::File {
                mode,
                size: data.len() as u64,
                mtime_secs,
                blob_blake3,
                chunks: index_entries,
            },
        }),
        chunk_count: chunks.len(),
        new_chunks,
        reused_chunks,
        seed_reused: false,
        rechunked: true,
        seed_missing: false,
    })
}

/// Load a prior `.cfidx` for `make --seed` (rejects `.cfdir` / bad magic / decode).
fn load_seed_cfidx(path: &Path) -> Result<Index> {
    match peek_listing_kind(path)? {
        ListingKind::Index => {}
        ListingKind::DirArchive => bail!(
            "make --seed requires a .cfidx prior (got .cfdir at {})",
            path.display()
        ),
    }
    let idx = load_index(path)?;
    idx.validate()
        .map_err(|e| anyhow::anyhow!("seed index structure {}: {e}", path.display()))?;
    Ok(idx)
}

/// Load a prior `.cfdir` for `--seed` (rejects `.cfidx` / bad magic / decode errors).
fn load_seed_cfdir(path: &Path) -> Result<DirArchive> {
    match peek_listing_kind(path)? {
        ListingKind::DirArchive => {}
        ListingKind::Index => bail!(
            "--seed requires a .cfdir prior (got .cfidx at {})",
            path.display()
        ),
    }
    let arch = load_dir_archive(path)?;
    arch.validate()
        .map_err(|e| anyhow::anyhow!("seed archive structure {}: {e}", path.display()))?;
    Ok(arch)
}

/// One filesystem symlink discovered during archive walk (`--symlinks record`).
struct ArchiveSymlinkCandidate {
    full: PathBuf,
    /// Target string as returned by `read_link` (not canonicalized).
    target: String,
    mode: u32,
}

/// One truly empty leaf directory discovered during archive walk (`--empty-dirs`).
struct ArchiveEmptyDirCandidate {
    full: PathBuf,
    mode: u32,
}

/// Recursively collect regular-file paths (and optionally symlink / empty-dir
/// candidates) under `dir` (relative walk from `root`).
///
/// Special files are always skipped with a per-path stderr warning.
/// With [`SymlinkPolicy::Skip`] (default ≡ 1.11): symlinks are skipped with warn
/// **before** PathFilter. With [`SymlinkPolicy::Record`]: symlinks are collected
/// as candidates (target via `read_link` as-is; mode from `symlink_metadata`)
/// and later pass through PathFilter — still **not** followed (symlink-to-dir
/// is not recursed into).
///
/// With `empty_dirs` (opt-in `--empty-dirs`): a directory whose `read_dir` yields
/// **zero** children is recorded as an empty-leaf candidate (mode from
/// metadata). Non-empty dirs are recursed; ancestor dirs of files are **not**
/// emitted (implied by file paths). Omit `empty_dirs` ≡ 1.14 (no Dir rows).
#[allow(clippy::too_many_arguments)] // walk collectors: root/dir + out vecs + counters + policy
fn collect_archive_files(
    root: &Path,
    dir: &Path,
    out: &mut Vec<PathBuf>,
    symlink_out: &mut Vec<ArchiveSymlinkCandidate>,
    empty_dir_out: &mut Vec<ArchiveEmptyDirCandidate>,
    skipped_symlinks: &mut usize,
    skipped_special: &mut usize,
    policy: SymlinkPolicy,
    empty_dirs: bool,
) -> Result<()> {
    let entries = fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("read_dir entry under {}", dir.display()))?;
        let path = entry.path();
        let ft = entry
            .file_type()
            .with_context(|| format!("file_type {}", path.display()))?;

        if ft.is_symlink() {
            match policy {
                SymlinkPolicy::Skip => {
                    *skipped_symlinks += 1;
                    eprintln!(
                        "archive: skip symlink {} (policy: skip+warn; not recorded / not followed)",
                        display_under_root(root, &path)
                    );
                }
                SymlinkPolicy::Record => {
                    let target_path = fs::read_link(&path)
                        .with_context(|| format!("read_link {}", path.display()))?;
                    let target = target_path.to_string_lossy().into_owned();
                    let meta = fs::symlink_metadata(&path)
                        .with_context(|| format!("symlink_metadata {}", path.display()))?;
                    // Match file mode recording: full unix mode bits from metadata.
                    let mode = file_mode_u32(&meta);
                    symlink_out.push(ArchiveSymlinkCandidate {
                        full: path,
                        target,
                        mode,
                    });
                    // Do not recurse into symlink-to-dir.
                }
            }
            continue;
        }
        if ft.is_dir() {
            // Truly empty leaf: no children at all. Opt-in `--empty-dirs` records
            // it; otherwise omit (≡ 1.14). Non-empty → recurse (never emit Dir for
            // ancestors of files — those paths are implied by File entries).
            let is_empty = {
                let mut rd =
                    fs::read_dir(&path).with_context(|| format!("read_dir {}", path.display()))?;
                rd.next().is_none()
            };
            if is_empty {
                if empty_dirs {
                    let meta = fs::metadata(&path)
                        .with_context(|| format!("metadata {}", path.display()))?;
                    empty_dir_out.push(ArchiveEmptyDirCandidate {
                        full: path,
                        mode: file_mode_u32(&meta),
                    });
                }
                continue;
            }
            collect_archive_files(
                root,
                &path,
                out,
                symlink_out,
                empty_dir_out,
                skipped_symlinks,
                skipped_special,
                policy,
                empty_dirs,
            )?;
            continue;
        }
        if ft.is_file() {
            out.push(path);
            continue;
        }
        // fifo / socket / device / other
        *skipped_special += 1;
        eprintln!(
            "archive: skip special file {} (fifo/socket/device)",
            display_under_root(root, &path)
        );
    }
    Ok(())
}

fn display_under_root(root: &Path, path: &Path) -> String {
    relative_archive_path(root, path).unwrap_or_else(|_| path.display().to_string())
}

/// Build a `/`-separated relative archive path from `root` to `full`.
fn relative_archive_path(root: &Path, full: &Path) -> Result<String> {
    let rel = full.strip_prefix(root).with_context(|| {
        format!(
            "path {} is not under archive root {}",
            full.display(),
            root.display()
        )
    })?;
    let mut parts: Vec<String> = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(s) => {
                let s = s.to_string_lossy();
                if s.contains('/') || s.contains('\\') {
                    bail!("path component contains separator: {s:?}");
                }
                parts.push(s.into_owned());
            }
            Component::CurDir => {}
            other => bail!(
                "unsupported path component {:?} under {}",
                other,
                full.display()
            ),
        }
    }
    if parts.is_empty() {
        bail!("refusing to archive the root directory itself as a file entry");
    }
    Ok(parts.join("/"))
}

fn file_mode_u32(meta: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode()
    }
    #[cfg(not(unix))]
    {
        if meta.permissions().readonly() {
            0o444
        } else {
            0o644
        }
    }
}

fn file_mtime_secs(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn load_index(path: &Path) -> Result<Index> {
    let bytes = fs::read(path).with_context(|| format!("read index {}", path.display()))?;
    Index::decode(&bytes).map_err(|e| anyhow::anyhow!("decode index {}: {e}", path.display()))
}

fn load_dir_archive(path: &Path) -> Result<DirArchive> {
    let bytes = fs::read(path).with_context(|| format!("read archive {}", path.display()))?;
    DirArchive::decode(&bytes)
        .map_err(|e| anyhow::anyhow!("decode archive {}: {e}", path.display()))
}

/// Load a `.cfdir` for `filter` (reject `.cfidx` / bad magic).
fn load_cfdir_for_filter(path: &Path) -> Result<DirArchive> {
    match peek_listing_kind(path)? {
        ListingKind::DirArchive => {}
        ListingKind::Index => bail!(
            "filter expects a `.cfdir` listing; {} looks like a `.cfidx` (single-blob; filter does not apply to `.cfidx`)",
            path.display()
        ),
    }
    let arch = load_dir_archive(path)?;
    arch.validate()
        .map_err(|e| anyhow::anyhow!("archive structure {}: {e}", path.display()))?;
    Ok(arch)
}

/// Persist `filter_dir_archive` of `input` to `output` (atomic; refuse overwrite
/// unless `force`; skip write under `dry_run`).
///
/// Empty `path_filter` ≡ identity (re-encode). Does **not** open a store /
/// rechunk / walk a source tree / prune dest trees. **≠** prune / **≠**
/// `gc --path` / **≠** sync / **≠** write mount / **≠** pack / **≠**
/// `archive --path`.
fn cmd_filter(
    input: &Path,
    output: &Path,
    path_filter: &PathFilter,
    dry_run: bool,
    force: bool,
    format: CliFormat,
) -> Result<()> {
    let input_arch = load_cfdir_for_filter(input)?;
    let out_arch = filter_dir_archive(&input_arch, path_filter);

    let file_count = out_arch
        .entries
        .iter()
        .filter(|e| matches!(e.kind, DirEntryKind::File { .. }))
        .count();
    let dir_count = out_arch
        .entries
        .iter()
        .filter(|e| matches!(e.kind, DirEntryKind::Dir { .. }))
        .count();
    let symlink_count = out_arch
        .entries
        .iter()
        .filter(|e| matches!(e.kind, DirEntryKind::Symlink { .. }))
        .count();
    // Excluded = input File+Symlink leaves that failed PathFilter (Dirs that
    // drop as non-ancestors are not counted — same leaf accounting as archive).
    let input_leaves = input_arch
        .entries
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                DirEntryKind::File { .. } | DirEntryKind::Symlink { .. }
            )
        })
        .count();
    let excluded = input_leaves.saturating_sub(file_count + symlink_count);

    if dry_run {
        match format {
            CliFormat::Text => {
                eprintln!(
                    "filter: dry-run: would write {} (files={}, dirs={}, symlinks={}; excluded={});                      no .cfdir written (from {})",
                    output.display(),
                    file_count,
                    dir_count,
                    symlink_count,
                    excluded,
                    input.display()
                );
            }
            CliFormat::Json => {
                let obj = serde_json::json!({
                    "ok": true,
                    "dry_run": true,
                    "input": input.display().to_string(),
                    "output": output.display().to_string(),
                    "files": file_count,
                    "dirs": dir_count,
                    "symlinks": symlink_count,
                    "excluded": excluded,
                });
                println!("{obj}");
            }
        }
        return Ok(());
    }

    if output.exists() && !force {
        bail!(
            "filter output {} already exists (refusing to overwrite; pass --force or choose another -o)",
            output.display()
        );
    }

    let bytes = out_arch
        .encode()
        .map_err(|e| anyhow::anyhow!("encode filtered .cfdir: {e}"))?;

    write_cfdir_atomic(output, &bytes)?;

    match format {
        CliFormat::Text => {
            eprintln!(
                "filter: wrote {} (files={}, dirs={}, symlinks={}; excluded={}) from {}",
                output.display(),
                file_count,
                dir_count,
                symlink_count,
                excluded,
                input.display()
            );
        }
        CliFormat::Json => {
            let obj = serde_json::json!({
                "ok": true,
                "dry_run": false,
                "input": input.display().to_string(),
                "output": output.display().to_string(),
                "files": file_count,
                "dirs": dir_count,
                "symlinks": symlink_count,
                "excluded": excluded,
            });
            println!("{obj}");
        }
    }
    Ok(())
}

/// Write `.cfdir` bytes via a unique temp sibling + rename (atomic on same FS).
fn write_cfdir_atomic(output: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create parent dir {}", parent.display()))?;
        }
    }
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let stem = output
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("out.cfdir");
    let tmp_name = format!(
        ".{}.{}.{:x}.tmp",
        stem,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let tmp_path = parent.join(tmp_name);

    let write_result = (|| -> Result<()> {
        let mut file = File::create(&tmp_path)
            .with_context(|| format!("create temp {}", tmp_path.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("write temp {}", tmp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("fsync temp {}", tmp_path.display()))?;
        drop(file);
        fs::rename(&tmp_path, output).with_context(|| {
            format!("rename temp {} → {}", tmp_path.display(), output.display())
        })?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    write_result
}

/// Default logical blob name for `.cfidx` inventory (strip trailing `.cfidx`).
/// Same naming as mount's default blob file (without requiring the fuse crate).
fn ls_blob_name(index_path: &Path) -> String {
    let file_name = index_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("blob");
    match file_name.strip_suffix(".cfidx") {
        Some(stem) if !stem.is_empty() => stem.to_string(),
        Some(_) => "blob".to_string(), // bare ".cfidx"
        None if file_name.is_empty() => "blob".to_string(),
        None => file_name.to_string(),
    }
}

/// One inventory row for `ls` (decode only; never opens a store).
///
/// JSON shape (ops-json prep): each object has `kind` (`file`|`dir`|`symlink`)
/// and `path`; File may add `size` and (with `--chunks`) `chunks` (hex array);
/// Symlink adds `target`; Dir has neither size nor chunks.
struct LsRow {
    kind: &'static str,
    path: String,
    size: Option<u64>,
    target: Option<String>,
    /// Chunk hex ids; populated only for File when `--chunks` is set.
    chunks: Option<Vec<String>>,
}

/// Inventory a `.cfidx` / `.cfdir` listing (decode only; **never** opens a store).
///
/// Text lines (tab-separated): `file\tpath\tsize` (+ optional `\thex,hex…` when
/// `--chunks`); `dir\tpath`; `symlink\tpath\ttarget`. JSON:
/// `{ "ok": true, "entries": [ { "kind", "path", "size"?, "target"?, "chunks"? } ] }`.
/// Path flags scope `.cfdir` via [`filter_dir_archive`] (empty ≡ full);
/// `.cfidx` + any path flag → clear non-zero.
/// **≠** mount / **≠** extract / **≠** verify / **≠** pack / **≠** filter.
fn cmd_ls(
    listing: &Path,
    path_filter: &PathFilter,
    format: CliFormat,
    show_chunks: bool,
) -> Result<()> {
    let filter_active = !path_filter.paths().is_empty() || !path_filter.excludes().is_empty();
    let mut rows: Vec<LsRow> = Vec::new();
    match peek_listing_kind(listing)? {
        ListingKind::Index => {
            if filter_active {
                bail!(
                    "--path/--path-from/--exclude applies to `.cfdir` File entries; {} looks like a `.cfidx` (use without path flags for full single-blob inventory)",
                    listing.display()
                );
            }
            let index = load_index(listing)?;
            index
                .validate()
                .map_err(|e| anyhow::anyhow!("index structure {}: {e}", listing.display()))?;
            let name = ls_blob_name(listing);
            let chunks = if show_chunks {
                Some(
                    index
                        .entries
                        .iter()
                        .map(|e| e.chunk_id.to_string())
                        .collect(),
                )
            } else {
                None
            };
            rows.push(LsRow {
                kind: "file",
                path: name,
                size: Some(index.total_size),
                target: None,
                chunks,
            });
        }
        ListingKind::DirArchive => {
            let arch = load_dir_archive(listing)?;
            arch.validate()
                .map_err(|e| anyhow::anyhow!("archive structure {}: {e}", listing.display()))?;
            // Empty PathFilter ≡ full listing; otherwise same keep rules as mount/filter.
            let arch = filter_dir_archive(&arch, path_filter);
            rows.reserve(arch.entries.len());
            for entry in &arch.entries {
                match &entry.kind {
                    DirEntryKind::File { size, chunks, .. } => {
                        let chunk_hexes = if show_chunks {
                            Some(chunks.iter().map(|c| c.chunk_id.to_string()).collect())
                        } else {
                            None
                        };
                        rows.push(LsRow {
                            kind: "file",
                            path: entry.path.clone(),
                            size: Some(*size),
                            target: None,
                            chunks: chunk_hexes,
                        });
                    }
                    DirEntryKind::Dir { .. } => {
                        rows.push(LsRow {
                            kind: "dir",
                            path: entry.path.clone(),
                            size: None,
                            target: None,
                            chunks: None,
                        });
                    }
                    DirEntryKind::Symlink { target, .. } => {
                        rows.push(LsRow {
                            kind: "symlink",
                            path: entry.path.clone(),
                            size: None,
                            target: Some(target.clone()),
                            chunks: None,
                        });
                    }
                }
            }
            rows.sort_by(|a, b| a.path.cmp(&b.path));
        }
    }
    emit_ls_rows(&rows, format)
}

fn emit_ls_rows(rows: &[LsRow], format: CliFormat) -> Result<()> {
    match format {
        CliFormat::Text => {
            let mut out = std::io::stdout().lock();
            for row in rows {
                match row.kind {
                    "file" => {
                        let size = row.size.unwrap_or(0);
                        write!(out, "file\t{}\t{size}", row.path)?;
                        if let Some(chunks) = &row.chunks {
                            write!(out, "\t{}", chunks.join(","))?;
                        }
                        writeln!(out)?;
                    }
                    "dir" => {
                        writeln!(out, "dir\t{}", row.path)?;
                    }
                    "symlink" => {
                        let target = row.target.as_deref().unwrap_or("");
                        writeln!(out, "symlink\t{}\t{target}", row.path)?;
                    }
                    other => bail!("internal: unknown ls kind {other}"),
                }
            }
            Ok(())
        }
        CliFormat::Json => {
            // Nail JSON field names for ops-json M4:
            // { "ok": true, "entries": [ { "kind", "path", "size"?, "target"?, "chunks"? } ] }
            let mut entries = Vec::with_capacity(rows.len());
            for row in rows {
                let mut obj = serde_json::Map::new();
                obj.insert(
                    "kind".into(),
                    serde_json::Value::String(row.kind.to_string()),
                );
                obj.insert("path".into(), serde_json::Value::String(row.path.clone()));
                if let Some(size) = row.size {
                    obj.insert("size".into(), serde_json::json!(size));
                }
                if let Some(target) = &row.target {
                    obj.insert("target".into(), serde_json::Value::String(target.clone()));
                }
                if let Some(chunks) = &row.chunks {
                    obj.insert("chunks".into(), serde_json::json!(chunks));
                }
                entries.push(serde_json::Value::Object(obj));
            }
            let root = serde_json::json!({
                "ok": true,
                "entries": entries,
            });
            println!("{root}");
            Ok(())
        }
    }
}

/// Load a `.cfdir` for `diff` (reject `.cfidx` / bad magic).
fn load_cfdir_for_diff(path: &Path) -> Result<DirArchive> {
    match peek_listing_kind(path)? {
        ListingKind::DirArchive => {}
        ListingKind::Index => bail!(
            "diff expects `.cfdir` listings; {} looks like a `.cfidx` (single-blob; use verify/cmp)",
            path.display()
        ),
    }
    let arch = load_dir_archive(path)?;
    arch.validate()
        .map_err(|e| anyhow::anyhow!("archive structure {}: {e}", path.display()))?;
    Ok(arch)
}

/// Format the stable machine-parseable `diff:` summary line (exact field names).
fn format_diff_summary(report: &DiffReport) -> String {
    format!(
        "diff: added={} removed={} changed={} meta_changed={} chunks_shared={} chunks_only_left={} chunks_only_right={}",
        report.added.len(),
        report.removed.len(),
        report.changed.len(),
        report.meta_changed.len(),
        report.chunks_shared,
        report.chunks_only_left,
        report.chunks_only_right,
    )
}

/// Escape a string as a JSON string literal (including surrounding quotes).
fn json_escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_string_array(paths: &[String]) -> String {
    let mut out = String::from("[");
    for (i, path) in paths.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&json_escape_string(path));
    }
    out.push(']');
    out
}

/// One JSON object aligned with the text `diff:` summary field names.
fn format_diff_json(report: &DiffReport) -> String {
    format!(
        "{{\"added\":{},\"removed\":{},\"changed\":{},\"meta_changed\":{},\"chunks_shared\":{},\"chunks_only_left\":{},\"chunks_only_right\":{}}}",
        json_string_array(&report.added),
        json_string_array(&report.removed),
        json_string_array(&report.changed),
        json_string_array(&report.meta_changed),
        report.chunks_shared,
        report.chunks_only_left,
        report.chunks_only_right,
    )
}

fn print_diff_path_category(label: &str, paths: &[String], max_paths: Option<usize>) {
    if paths.is_empty() {
        return;
    }
    println!("{label}:");
    let limit = max_paths.unwrap_or(usize::MAX);
    let shown = paths.len().min(limit);
    for path in &paths[..shown] {
        println!("  {path}");
    }
    if paths.len() > shown {
        let more = paths.len() - shown;
        println!("  ... and {more} more");
    }
}

fn diff_has_differences(report: &DiffReport) -> bool {
    let has_path_diff = !(report.added.is_empty()
        && report.removed.is_empty()
        && report.changed.is_empty()
        && report.meta_changed.is_empty());
    let has_chunk_diff = report.chunks_only_left > 0 || report.chunks_only_right > 0;
    has_path_diff || has_chunk_diff
}

fn emit_diff_report(
    report: &DiffReport,
    max_paths: Option<usize>,
    format: CliFormat,
) -> Result<()> {
    match format {
        CliFormat::Text => {
            print_diff_path_category("added", &report.added, max_paths);
            print_diff_path_category("removed", &report.removed, max_paths);
            print_diff_path_category("changed", &report.changed, max_paths);
            print_diff_path_category("meta_changed", &report.meta_changed, max_paths);
            // Path lists + summary on stdout (documented in `diff --help` / docs/diff.md).
            println!("{}", format_diff_summary(report));
        }
        CliFormat::Json => {
            // Full path arrays; `--max-paths` does not truncate JSON.
            println!("{}", format_diff_json(report));
        }
    }

    if diff_has_differences(report) {
        // Like diff(1): differences → exit 1 without an "error:" prefix.
        // Exit code is independent of `--format`.
        std::process::exit(1);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_diff_dispatch(
    tree: Option<&Path>,
    left: &Path,
    right: Option<&Path>,
    max_paths: Option<usize>,
    format: CliFormat,
    path_filter: &PathFilter,
    progress: bool,
    symlinks: SymlinkPolicy,
) -> Result<()> {
    match tree {
        Some(src_dir) => {
            if let Some(extra) = right {
                bail!(
                    "diff --tree takes one listing `.cfdir` after the source dir;                      unexpected extra argument {}",
                    extra.display()
                );
            }
            cmd_diff_tree(
                src_dir,
                left,
                max_paths,
                format,
                path_filter,
                progress,
                symlinks,
            )
        }
        None => {
            let Some(right) = right else {
                bail!(
                    "diff without --tree requires two `.cfdir` arguments                      (or use: chunkforge diff --tree <src-dir> <listing.cfdir>)"
                );
            };
            // Bare directory without --tree: do not silently try to decode as .cfdir.
            if left.is_dir() || right.is_dir() {
                bail!(
                    "diff without --tree expects two `.cfdir` files; got a directory.                      Use: chunkforge diff --tree <src-dir> <listing.cfdir>"
                );
            }
            // Listing↔listing: Symlink already compared in-lib; `--symlinks`
            // requires `--tree` (clap), so this path never sees an explicit flag.
            let _ = symlinks;
            cmd_diff_listings(left, right, max_paths, format, path_filter, progress)
        }
    }
}

/// `|left∪right|` File + Symlink paths after filtering (Dir-only excluded;
/// matches [`diff_dir_archives_with_progress`] TOTAL / tick count).
fn diff_file_path_union_len(left: &DirArchive, right: &DirArchive) -> usize {
    let mut paths: HashSet<&str> = left
        .entries
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                DirEntryKind::File { .. } | DirEntryKind::Symlink { .. }
            )
        })
        .map(|e| e.path.as_str())
        .collect();
    for e in &right.entries {
        if matches!(
            e.kind,
            DirEntryKind::File { .. } | DirEntryKind::Symlink { .. }
        ) {
            paths.insert(e.path.as_str());
        }
    }
    paths.len()
}

/// Narrow a listing with [`PathFilter`] before compare (empty filter ≡ identity).
/// Uses library [`filter_dir_archive`]: keeps matching **File** and **Symlink**
/// leaves plus ancestor **Dir** entries (same policy as mount / archive filter).
fn filter_dir_archive_entries(arch: &DirArchive, filter: &PathFilter) -> DirArchive {
    filter_dir_archive(arch, filter)
}

fn cmd_diff_listings(
    left: &Path,
    right: &Path,
    max_paths: Option<usize>,
    format: CliFormat,
    path_filter: &PathFilter,
    progress: bool,
) -> Result<()> {
    let left_arch = filter_dir_archive_entries(&load_cfdir_for_diff(left)?, path_filter);
    let right_arch = filter_dir_archive_entries(&load_cfdir_for_diff(right)?, path_filter);
    let total = diff_file_path_union_len(&left_arch, &right_arch);
    let prog = ProgressReporter::new(progress, "diff", Some(total));
    let report = diff_dir_archives_with_progress(&left_arch, &right_arch, || prog.tick());
    emit_diff_report(&report, max_paths, format)
}

/// Tree↔listing: left = ephemeral DirArchive from `src_dir`, right = listing.
fn cmd_diff_tree(
    src_dir: &Path,
    listing: &Path,
    max_paths: Option<usize>,
    format: CliFormat,
    path_filter: &PathFilter,
    progress: bool,
    symlinks: SymlinkPolicy,
) -> Result<()> {
    if !src_dir.is_dir() {
        bail!(
            "diff --tree source {} is not a directory (or is unreadable)",
            src_dir.display()
        );
    }
    if listing.is_dir() {
        bail!(
            "diff --tree expects one listing `.cfdir` (got directory {}); two trees are not supported",
            listing.display()
        );
    }
    // Build ephemeral tree against the full listing (so in-scope matching paths
    // still copy chunk tables), then narrow both sides with PathFilter.
    let listing_full = load_cfdir_for_diff(listing)?;
    let tree_full = build_ephemeral_tree_archive(src_dir, &listing_full, symlinks)?;
    let listing_arch = filter_dir_archive_entries(&listing_full, path_filter);
    let tree_arch = filter_dir_archive_entries(&tree_full, path_filter);
    // Documented orientation: left=tree, right=listing.
    let total = diff_file_path_union_len(&tree_arch, &listing_arch);
    let prog = ProgressReporter::new(progress, "diff", Some(total));
    let report = diff_dir_archives_with_progress(&tree_arch, &listing_arch, || prog.tick());
    emit_diff_report(&report, max_paths, format)
}

/// Build an in-memory `DirArchive` from a source tree for `diff --tree`.
///
/// Does **not** write store or `.cfdir`. For each regular file: size, mode,
/// mtime_secs, stream BLAKE3 → `blob_blake3`. When the path exists in `listing`
/// **and** `blob_blake3` matches, copy the listing's File entry (chunk table +
/// meta) so identical tree↔listing yields `chunks_shared`≈full and
/// `chunks_only_*=0`. Otherwise use an empty chunk list (path-level diff still
/// works). Empty non-zero-size chunk tables are **not** structurally valid for
/// encode, so this archive is ephemeral and never written.
///
/// Symlink policy (Phase23-M1, mirrors archive):
/// - [`SymlinkPolicy::Skip`] (default ≡ 1.12): skip+warn; not in entries.
/// - [`SymlinkPolicy::Record`]: `read_link` target as-is + `symlink_metadata`
///   mode → [`DirEntryKind::Symlink`] (0 chunks; **not** followed); absolute or
///   empty target → clear non-zero. Special files always skip+warn.
///
/// PathFilter is applied by the caller **after** this build.
fn build_ephemeral_tree_archive(
    src_dir: &Path,
    listing: &DirArchive,
    policy: SymlinkPolicy,
) -> Result<DirArchive> {
    let listing_files = seed_file_map(listing);

    let mut file_paths: Vec<PathBuf> = Vec::new();
    let mut symlink_candidates: Vec<ArchiveSymlinkCandidate> = Vec::new();
    let mut skipped_symlinks = 0usize;
    let mut skipped_special = 0usize;
    collect_diff_tree_files(
        src_dir,
        src_dir,
        &mut file_paths,
        &mut symlink_candidates,
        &mut skipped_symlinks,
        &mut skipped_special,
        policy,
    )?;
    file_paths.sort();

    // Absolute / empty targets: reject under Record before any summary
    // (same spirit as archive; PathFilter still applied by caller afterward).
    for cand in &symlink_candidates {
        let rel = relative_archive_path(src_dir, &cand.full)?;
        validate_archive_path(&rel)
            .map_err(|e| anyhow::anyhow!("invalid archive path {rel:?}: {e}"))?;
        if cand.target.is_empty() {
            bail!("diff: symlink {rel} has empty target (refusing to record)");
        }
        if Path::new(&cand.target).is_absolute() {
            bail!(
                "diff: symlink {rel} has absolute target {:?} (refusing; use a relative target)",
                cand.target
            );
        }
    }

    match policy {
        SymlinkPolicy::Skip if skipped_symlinks > 0 || skipped_special > 0 => {
            eprintln!(
                "diff: symlink policy = skip+warn (not recorded / not followed); \
                 skipped {skipped_symlinks} symlink{}, {skipped_special} special (fifo/socket/device)",
                if skipped_symlinks == 1 { "" } else { "s" },
            );
        }
        SymlinkPolicy::Record => {
            if !symlink_candidates.is_empty() {
                eprintln!(
                    "diff: symlink policy = record (not followed); recorded {} symlink{}",
                    symlink_candidates.len(),
                    if symlink_candidates.len() == 1 {
                        ""
                    } else {
                        "s"
                    },
                );
            }
            if skipped_special > 0 {
                eprintln!("diff: skipped {skipped_special} special (fifo/socket/device)");
            }
        }
        SymlinkPolicy::Skip => {}
    }

    let mut entries: Vec<DirEntry> =
        Vec::with_capacity(file_paths.len() + symlink_candidates.len());
    for full in &file_paths {
        let rel = relative_archive_path(src_dir, full)?;
        validate_archive_path(&rel)
            .map_err(|e| anyhow::anyhow!("invalid archive path {rel:?}: {e}"))?;

        let meta =
            fs::symlink_metadata(full).with_context(|| format!("stat {}", full.display()))?;
        if meta.file_type().is_symlink() {
            // collect should have skipped or recorded; belt-and-suspenders.
            eprintln!(
                "diff: skip symlink {} (unexpected after walk; not recorded)",
                full.display()
            );
            continue;
        }
        if !meta.is_file() {
            eprintln!(
                "diff: skip special file {} (fifo/socket/device)",
                full.display()
            );
            continue;
        }

        let size = meta.len();
        let mode = file_mode_u32(&meta);
        let mtime_secs = file_mtime_secs(&meta);
        let mut file =
            File::open(full).with_context(|| format!("open {} for tree blake3", full.display()))?;
        let blob_blake3 =
            hash_reader(&mut file).map_err(|e| anyhow::anyhow!("hash {}: {e}", full.display()))?;

        let kind = if let Some(prior) = listing_files.get(rel.as_str()) {
            match &prior.kind {
                DirEntryKind::File {
                    blob_blake3: prior_blob,
                    ..
                } if *prior_blob == blob_blake3 => {
                    // Identical content: prefer listing meta + chunk table.
                    prior.kind.clone()
                }
                _ => DirEntryKind::File {
                    mode,
                    size,
                    mtime_secs,
                    blob_blake3,
                    chunks: Vec::new(),
                },
            }
        } else {
            DirEntryKind::File {
                mode,
                size,
                mtime_secs,
                blob_blake3,
                chunks: Vec::new(),
            }
        };

        entries.push(DirEntry { path: rel, kind });
    }

    for cand in &symlink_candidates {
        let rel = relative_archive_path(src_dir, &cand.full)?;
        entries.push(DirEntry {
            path: rel,
            kind: DirEntryKind::Symlink {
                mode: cand.mode,
                target: cand.target.clone(),
            },
        });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));

    // Ephemeral only — may contain empty chunk tables on non-zero size (not
    // encode-valid) and Symlink under Record. Do not call DirArchive::new /
    // encode / write / validate.
    Ok(DirArchive {
        format_version: DIR_FORMAT_VERSION_V1,
        flags: 0,
        entries,
    })
}

/// Walk like [`collect_archive_files`], but warn with a `diff:` prefix.
///
/// With [`SymlinkPolicy::Skip`] (default ≡ 1.12): symlinks are skipped with warn
/// and not followed. With [`SymlinkPolicy::Record`]: collect candidates via
/// `read_link` / `symlink_metadata` — still **not** followed (symlink-to-dir is
/// not recursed into). Special files always skip+warn.
fn collect_diff_tree_files(
    root: &Path,
    dir: &Path,
    out: &mut Vec<PathBuf>,
    symlink_out: &mut Vec<ArchiveSymlinkCandidate>,
    skipped_symlinks: &mut usize,
    skipped_special: &mut usize,
    policy: SymlinkPolicy,
) -> Result<()> {
    let entries = fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("read_dir entry under {}", dir.display()))?;
        let path = entry.path();
        let ft = entry
            .file_type()
            .with_context(|| format!("file_type {}", path.display()))?;

        if ft.is_symlink() {
            match policy {
                SymlinkPolicy::Skip => {
                    *skipped_symlinks += 1;
                    eprintln!(
                        "diff: skip symlink {} (policy: skip+warn; not recorded / not followed)",
                        display_under_root(root, &path)
                    );
                }
                SymlinkPolicy::Record => {
                    let target_path = fs::read_link(&path)
                        .with_context(|| format!("read_link {}", path.display()))?;
                    let target = target_path.to_string_lossy().into_owned();
                    let meta = fs::symlink_metadata(&path)
                        .with_context(|| format!("symlink_metadata {}", path.display()))?;
                    let mode = file_mode_u32(&meta);
                    symlink_out.push(ArchiveSymlinkCandidate {
                        full: path,
                        target,
                        mode,
                    });
                    // Do not recurse into symlink-to-dir.
                }
            }
            continue;
        }
        if ft.is_dir() {
            collect_diff_tree_files(
                root,
                &path,
                out,
                symlink_out,
                skipped_symlinks,
                skipped_special,
                policy,
            )?;
            continue;
        }
        if ft.is_file() {
            out.push(path);
            continue;
        }
        *skipped_special += 1;
        eprintln!(
            "diff: skip special file {} (fifo/socket/device)",
            display_under_root(root, &path)
        );
    }
    Ok(())
}

/// Peek listing magic: `.cfidx` vs `.cfdir`.
enum ListingKind {
    Index,
    DirArchive,
}

fn peek_listing_kind(path: &Path) -> Result<ListingKind> {
    use std::io::Read;
    let mut file = File::open(path).with_context(|| format!("open listing {}", path.display()))?;
    let mut magic = [0u8; 8];
    file.read_exact(&mut magic)
        .with_context(|| format!("read magic from {}", path.display()))?;
    if magic[..7] == MAGIC_PREFIX {
        Ok(ListingKind::Index)
    } else if magic[..7] == DIR_MAGIC_PREFIX {
        Ok(ListingKind::DirArchive)
    } else {
        bail!(
            "unrecognized listing magic in {} (expected CFIDX or CFDIR, got {:02x?})",
            path.display(),
            &magic[..]
        );
    }
}

/// Collect chunk ids referenced by a `.cfidx` or `.cfdir` listing (validated).
fn listing_chunk_ids(path: &Path) -> Result<Vec<ChunkId>> {
    listing_chunk_ids_filtered(
        path,
        &PathFilter::new(Vec::<String>::new(), Vec::<String>::new()).expect("empty PathFilter"),
    )
}

/// Collect chunk ids from a listing, applying [`PathFilter`] to `.cfdir` File
/// entries only (Dir entries never contribute). Empty filter ≡ full reference
/// set (≡ 1.9.0). `--path`/`--path-from`/`--exclude` on a `.cfidx` is an error (no File paths).
fn listing_chunk_ids_filtered(path: &Path, filter: &PathFilter) -> Result<Vec<ChunkId>> {
    let filter_active = !filter.paths().is_empty() || !filter.excludes().is_empty();
    match peek_listing_kind(path)? {
        ListingKind::Index => {
            if filter_active {
                bail!(
                    "--path/--path-from/--exclude applies to `.cfdir` File entries; {} looks like a `.cfidx` (use without path flags for full single-blob reference set)",
                    path.display()
                );
            }
            let index = load_index(path)?;
            index
                .validate()
                .map_err(|e| anyhow::anyhow!("index structure {}: {e}", path.display()))?;
            Ok(index.entries.iter().map(|e| e.chunk_id).collect())
        }
        ListingKind::DirArchive => {
            let arch = load_dir_archive(path)?;
            arch.validate()
                .map_err(|e| anyhow::anyhow!("archive structure {}: {e}", path.display()))?;
            let mut ids = Vec::new();
            for entry in &arch.entries {
                if !filter.allows(&entry.path) {
                    continue;
                }
                match &entry.kind {
                    DirEntryKind::File { chunks, .. } => {
                        ids.extend(chunks.iter().map(|c| c.chunk_id));
                    }
                    DirEntryKind::Dir { .. } | DirEntryKind::Symlink { .. } => {}
                }
            }
            Ok(ids)
        }
    }
}

/// Union of chunk ids across mixed `.cfidx` / `.cfdir` listing args.
fn union_listing_chunk_ids(paths: &[PathBuf]) -> Result<(HashSet<ChunkId>, usize)> {
    union_listing_chunk_ids_filtered(
        paths,
        &PathFilter::new(Vec::<String>::new(), Vec::<String>::new()).expect("empty PathFilter"),
    )
}

/// Union of chunk ids with path filtering (Phase 13 M4 pull).
fn union_listing_chunk_ids_filtered(
    paths: &[PathBuf],
    filter: &PathFilter,
) -> Result<(HashSet<ChunkId>, usize)> {
    let mut referenced = HashSet::new();
    let mut listings_ok = 0usize;
    for path in paths {
        for id in listing_chunk_ids_filtered(path, filter)? {
            referenced.insert(id);
        }
        listings_ok += 1;
    }
    Ok((referenced, listings_ok))
}

/// Join a validated `/`-separated archive path onto `out_root`.
fn join_archive_path(out_root: &Path, rel: &str) -> Result<PathBuf> {
    // Paths are validated by DirArchive; still refuse absolute / escape defensively.
    validate_archive_path(rel).map_err(|e| anyhow::anyhow!("invalid archive path {rel:?}: {e}"))?;
    let mut dest = out_root.to_path_buf();
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            bail!("refusing path segment {seg:?} in {rel:?}");
        }
        dest.push(seg);
    }
    Ok(dest)
}

/// Phase22-M3: materialize one Symlink listing entry under `out_dir`.
///
/// Conflict policy (pinned):
/// - dest missing → create via `std::os::unix::fs::symlink`
/// - dest exists and is a **symlink**: without `--force` → error; with `--force`
///   → remove existing symlink then recreate (same-type overwrite only)
/// - dest exists and is a **directory** → always refuse (even with `--force`);
///   never replace a directory with a symlink
/// - dest exists as regular file / other → refuse regardless of `--force`
///
/// `--force` only overwrites an existing **symlink**. Absolute / empty targets
/// are rejected (belt-and-suspenders vs archive). No prune / `--delete`.
///
/// Mode: Linux does not support `lchmod` on symlinks (permissions are unused /
/// always 0777); we deliberately do **not** call [`apply_file_mode`] here because
/// `fs::set_permissions` would follow the link and mutate the target.
#[allow(clippy::too_many_arguments)]
fn materialize_extract_symlink(
    dest: &Path,
    entry_path: &str,
    target: &str,
    mode: u32,
    force: bool,
    skip_unchanged: bool,
    skipped_count: &mut usize,
    symlink_count: &mut usize,
) -> Result<()> {
    let _ = mode; // recorded in listing; not applied on Linux (see doc above)

    if target.is_empty() {
        bail!("extract: symlink {entry_path} has empty target (refusing)");
    }
    if Path::new(target).is_absolute() {
        bail!(
            "extract: symlink {entry_path} has absolute target {target:?} (refusing; use a relative target)"
        );
    }

    match fs::symlink_metadata(dest) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Fall through to create.
        }
        Err(e) => {
            return Err(e).with_context(|| format!("stat {}", dest.display()));
        }
        Ok(meta) => {
            let ft = meta.file_type();
            if ft.is_dir() {
                bail!(
                    "extract target {} already exists and is a directory (refusing to replace a directory with a symlink; --force does not change type)",
                    dest.display(),
                );
            }
            if ft.is_symlink() {
                let same_target = fs::read_link(dest)
                    .ok()
                    .is_some_and(|p| p.as_os_str() == std::ffi::OsStr::new(target));
                if skip_unchanged && same_target {
                    *skipped_count += 1;
                    return Ok(());
                }
                if !force {
                    bail!(
                        "extract target {} already exists (refusing to overwrite symlink; pass --force)",
                        dest.display()
                    );
                }
                // --force + existing symlink: remove then recreate below.
                fs::remove_file(dest).with_context(|| {
                    format!("remove existing symlink {} for --force", dest.display())
                })?;
            } else {
                // Regular file / other node: refuse even with --force.
                // Phase22: --force only overwrites an existing symlink.
                bail!(
                    "extract target {} already exists and is not a symlink (refusing to replace with a symlink; --force only overwrites an existing symlink)",
                    dest.display(),
                );
            }
        }
    }

    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create parent dir {}", parent.display()))?;
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(target, dest)
            .with_context(|| format!("create symlink {} -> {target:?}", dest.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = (dest, target);
        bail!("extract: symlink materialization requires unix");
    }

    *symlink_count += 1;
    Ok(())
}

fn apply_file_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(mode);
        fs::set_permissions(path, perms)
            .with_context(|| format!("set mode {:#o} on {}", mode, path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}

/// Normalize `cat --path` against `.cfdir` listing paths: trim, strip leading
/// `./`, then [`validate_archive_path`]. Exact string equality vs entry.path.
fn normalize_cat_path(raw: &str) -> Result<String> {
    let mut s = raw.trim();
    while let Some(rest) = s.strip_prefix("./") {
        s = rest;
    }
    if s.is_empty() {
        bail!("cat --path: path must be a non-empty relative archive path");
    }
    validate_archive_path(s).map_err(|e| anyhow::anyhow!("cat --path: {e}"))?;
    Ok(s.to_string())
}

#[allow(clippy::too_many_arguments)]
fn cmd_cat(
    source: &dyn ChunkSource,
    index_path: &Path,
    output: &Path,
    jobs: usize,
    format: CliFormat,
    path: Option<&str>,
    progress: bool,
    cache_stats: Option<&CacheStatsRef>,
) -> Result<()> {
    match peek_listing_kind(index_path)? {
        ListingKind::Index => {
            if path.is_some() {
                bail!(
                    "cat --path applies to `.cfdir` File entries; {} looks like a `.cfidx` (omit --path to reassemble the whole blob ≡ 1.14)",
                    index_path.display()
                );
            }
            let index = load_index(index_path)?;
            // Empty file: still created below; total_size must be 0.
            if index.total_size == 0 && !index.entries.is_empty() {
                bail!("invalid index: total_size 0 with non-empty entries");
            }
            let bytes = index.total_size;
            write_cat_plains(source, &index.entries, output, jobs, progress, "cat")?;
            emit_cat_ops_json(format, bytes, cache_stats);
            Ok(())
        }
        ListingKind::DirArchive => {
            let want = match path {
                Some(p) => normalize_cat_path(p)?,
                None => bail!(
                    "cat on `.cfdir` requires --path <rel> naming exactly one File entry (≠ extract whole tree ≠ prune ≠ multi-file); {}",
                    index_path.display()
                ),
            };
            let arch = load_dir_archive(index_path)?;
            arch.validate()
                .map_err(|e| anyhow::anyhow!("archive structure {}: {e}", index_path.display()))?;
            let matches: Vec<&DirEntry> = arch.entries.iter().filter(|e| e.path == want).collect();
            match matches.as_slice() {
                [] => bail!(
                    "cat --path: no entry matching {want:?} in {}",
                    index_path.display()
                ),
                [entry] => match &entry.kind {
                    DirEntryKind::File { size, chunks, .. } => {
                        let op = format!("cat (file {want})");
                        write_cat_plains(source, chunks, output, jobs, progress, &op)?;
                        // JSON field names for `.cfdir` reuse `.cfidx` `ok`/`bytes`.
                        emit_cat_ops_json(format, *size, cache_stats);
                        Ok(())
                    }
                    DirEntryKind::Symlink { target, .. } => bail!(
                        "cat --path: {want:?} is a symlink (target {target:?}); need a File entry (≠ follow ≠ extract)"
                    ),
                    DirEntryKind::Dir { .. } => bail!(
                        "cat --path: {want:?} is a directory; need a File entry (≠ extract whole tree ≠ prune)"
                    ),
                },
                _ => bail!(
                    "cat --path: multiple entries matching {want:?} in {} (listing corrupt?)",
                    index_path.display()
                ),
            }
        }
    }
}

/// Fetch entry plains and write a single `-o` file (shared by `.cfidx` / `.cfdir --path`).
fn write_cat_plains(
    source: &dyn ChunkSource,
    entries: &[IndexEntry],
    output: &Path,
    jobs: usize,
    progress: bool,
    op: &str,
) -> Result<()> {
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create parent dir {}", parent.display()))?;
        }
    }
    let file =
        File::create(output).with_context(|| format!("create output {}", output.display()))?;
    let mut writer = BufWriter::new(file);

    // Fetch plaintext (optionally concurrent); always write in entry order.
    // Progress is per listing chunk (TOTAL = entry count).
    let prog = ProgressReporter::new(progress, "cat", Some(entries.len()));
    let plains = fetch_entry_plains(source, entries, jobs, op, Some(&prog))?;
    for plain in &plains {
        writer
            .write_all(plain)
            .with_context(|| format!("write output {}", output.display()))?;
    }
    writer
        .flush()
        .with_context(|| format!("flush output {}", output.display()))?;
    writer
        .into_inner()
        .with_context(|| format!("finalize output {}", output.display()))?
        .sync_all()
        .with_context(|| format!("fsync output {}", output.display()))?;
    Ok(())
}

fn emit_cat_ops_json(format: CliFormat, bytes: u64, cache_stats: Option<&CacheStatsRef>) {
    match format {
        CliFormat::Text => {
            // ≡ 1.4.0: almost no stderr summary on success.
        }
        CliFormat::Json => {
            // `.cfidx` JSON field names unchanged (`ok` / `bytes`); `.cfdir --path` reuses them.
            let mut obj = serde_json::json!({
                "ok": true,
                "bytes": bytes,
            });
            apply_cache_ops_json(&mut obj, cache_stats);
            println!("{obj}");
        }
    }
}

/// Fetch every entry's plaintext with bounded concurrency; results in entry order.
///
/// When `jobs == 1`, work is serial and fail-fast on the calling thread (≡ 0.3.0).
/// Errors always include the chunk id.
fn fetch_entry_plains(
    source: &dyn ChunkSource,
    entries: &[IndexEntry],
    jobs: usize,
    op: &str,
    prog: Option<&ProgressReporter>,
) -> Result<Vec<Vec<u8>>> {
    let tasks: Vec<(usize, ChunkId, u64)> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let expected_len = entry_length(entries, i).expect("entry index in range");
            (i, entry.chunk_id, expected_len)
        })
        .collect();

    if jobs <= 1 {
        let mut plains = Vec::with_capacity(tasks.len());
        for (i, chunk_id, expected_len) in &tasks {
            let plain = source.get(chunk_id).with_context(|| {
                format!("{op}: missing or corrupt chunk {chunk_id} (index entry {i})")
            })?;
            if plain.len() as u64 != *expected_len {
                bail!(
                    "{op}: chunk {chunk_id} length mismatch: source has {} bytes, index expects {expected_len}",
                    plain.len()
                );
            }
            plains.push(plain);
            if let Some(p) = prog {
                p.tick();
            }
        }
        return Ok(plains);
    }

    let results = parallel::map_indexed(&tasks, jobs, |_idx, (i, chunk_id, expected_len)| {
        let outcome = (|| {
            let plain = match source.get(chunk_id) {
                Ok(bytes) => bytes,
                Err(e) => {
                    return Err(format!(
                        "{op}: missing or corrupt chunk {chunk_id} (index entry {i}): {e}"
                    ));
                }
            };
            if plain.len() as u64 != *expected_len {
                return Err(format!(
                    "{op}: chunk {chunk_id} length mismatch: source has {} bytes, index expects {expected_len}",
                    plain.len()
                ));
            }
            Ok(plain)
        })();
        if let Some(p) = prog {
            p.tick();
        }
        outcome
    });

    // Prefer lowest entry-index error (stable vs serial fail-fast order).
    let mut plains = Vec::with_capacity(results.len());
    let mut first_err: Option<(usize, String)> = None;
    for (pos, r) in results.into_iter().enumerate() {
        match r {
            Ok(bytes) => {
                if first_err.is_none() {
                    plains.push(bytes);
                }
            }
            Err(msg) => {
                if first_err.is_none() {
                    first_err = Some((pos, msg));
                }
            }
        }
    }
    if let Some((_, msg)) = first_err {
        bail!("{msg}");
    }
    Ok(plains)
}

fn cmd_verify(
    source: &dyn ChunkSource,
    listing_path: &Path,
    jobs: usize,
    format: CliFormat,
    progress: bool,
    cache_stats: Option<&CacheStatsRef>,
    path_filter: &PathFilter,
) -> Result<()> {
    match peek_listing_kind(listing_path)? {
        ListingKind::Index => {
            // `.cfidx` + any path/exclude flag → clear non-zero (same as push/pull).
            let filter_active =
                !path_filter.paths().is_empty() || !path_filter.excludes().is_empty();
            if filter_active {
                bail!(
                    "--path/--path-from/--exclude applies to `.cfdir` File entries; {} looks like a `.cfidx` (use without path flags for full single-blob reference set)",
                    listing_path.display()
                );
            }
            cmd_verify_index(source, listing_path, jobs, format, progress, cache_stats)
        }
        ListingKind::DirArchive => cmd_verify_dir(
            source,
            listing_path,
            jobs,
            format,
            progress,
            cache_stats,
            path_filter,
        ),
    }
}

fn cmd_verify_index(
    source: &dyn ChunkSource,
    index_path: &Path,
    jobs: usize,
    format: CliFormat,
    progress: bool,
    cache_stats: Option<&CacheStatsRef>,
) -> Result<()> {
    let index = load_index(index_path)?;
    index
        .validate()
        .map_err(|e| anyhow::anyhow!("index structure: {e}"))?;

    // Progress is per listing chunk (TOTAL = entry count).
    let prog = ProgressReporter::new(progress, "verify", Some(index.entries.len()));

    let plains = if jobs <= 1 {
        // Serial path ≡ 0.3.0: has then get, fail-fast in entry order.
        let mut plains = Vec::with_capacity(index.entries.len());
        for (i, entry) in index.entries.iter().enumerate() {
            let expected_len = entry_length(&index.entries, i).expect("entry index in range");
            match source.has(&entry.chunk_id) {
                Ok(true) => {}
                Ok(false) => bail!(
                    "verify failed: chunk {} missing from source (entry {i})",
                    entry.chunk_id
                ),
                Err(e) => bail!(
                    "verify failed: chunk {} presence check error (entry {i}): {e}",
                    entry.chunk_id
                ),
            }
            let plain = source.get(&entry.chunk_id).with_context(|| {
                format!(
                    "verify failed: chunk {} unreadable or hash mismatch (entry {i})",
                    entry.chunk_id
                )
            })?;
            if plain.len() as u64 != expected_len {
                bail!(
                    "verify failed: chunk {} length mismatch (got {}, expected {expected_len})",
                    entry.chunk_id,
                    plain.len()
                );
            }
            plains.push(plain);
            prog.tick();
        }
        plains
    } else {
        // Concurrent get (presence implied); length-checked; errors include chunk id.
        fetch_entry_plains(source, &index.entries, jobs, "verify failed", Some(&prog))?
    };

    let mut hasher = blake3::Hasher::new();
    let mut assembled: u64 = 0;
    for plain in &plains {
        hasher.update(plain);
        assembled += plain.len() as u64;
    }

    if assembled != index.total_size {
        bail!(
            "verify failed: assembled size {assembled} != index total_size {}",
            index.total_size
        );
    }

    let blob_id = ChunkId::from_bytes(*hasher.finalize().as_bytes());
    if blob_id != index.blob_blake3 {
        bail!(
            "verify failed: blob_blake3 mismatch (got {blob_id}, index has {})",
            index.blob_blake3
        );
    }

    match format {
        CliFormat::Text => {
            eprintln!(
                "verify: ok ({} bytes, {} chunk{})",
                index.total_size,
                index.chunk_count(),
                if index.chunk_count() == 1 { "" } else { "s" }
            );
        }
        CliFormat::Json => {
            let mut obj = serde_json::json!({
                "ok": true,
                "kind": "cfidx",
                "bytes": index.total_size,
                "chunks": index.chunk_count(),
            });
            apply_cache_ops_json(&mut obj, cache_stats);
            println!("{obj}");
        }
    }
    Ok(())
}

fn cmd_verify_dir(
    source: &dyn ChunkSource,
    archive_path: &Path,
    jobs: usize,
    format: CliFormat,
    progress: bool,
    cache_stats: Option<&CacheStatsRef>,
    path_filter: &PathFilter,
) -> Result<()> {
    let archive = load_dir_archive(archive_path)?;
    archive
        .validate()
        .map_err(|e| anyhow::anyhow!("archive structure: {e}"))?;

    // File entries that pass PathFilter contribute chunks; Symlink is structure-
    // only (0 chunks). Empty filter ≡ full tree (≡ 1.9.0).
    let mut file_count = 0usize;
    let mut symlink_count = 0usize;
    let mut total_chunks = 0usize;
    for entry in &archive.entries {
        if !path_filter.allows(&entry.path) {
            continue;
        }
        match &entry.kind {
            DirEntryKind::File { chunks, .. } => {
                file_count += 1;
                total_chunks += chunks.len();
            }
            DirEntryKind::Symlink { .. } => {
                symlink_count += 1;
            }
            DirEntryKind::Dir { .. } => {}
        }
    }

    // Progress is per referenced chunk across filtered File entries (TOTAL known).
    let prog = ProgressReporter::new(progress, "verify", Some(total_chunks));

    for entry in &archive.entries {
        if !path_filter.allows(&entry.path) {
            continue;
        }
        match &entry.kind {
            DirEntryKind::Dir { .. } => {
                // Structure already validated; nothing to fetch.
            }
            DirEntryKind::Symlink { target, .. } => {
                // Phase22-M3: structure + non-empty target + reject absolute target.
                // Do not fetch chunks; File/Dir behavior unchanged.
                if target.is_empty() {
                    bail!("verify failed (symlink {}): empty target", entry.path);
                }
                if Path::new(target).is_absolute() {
                    bail!(
                        "verify failed (symlink {}): absolute target {:?} (refusing)",
                        entry.path,
                        target
                    );
                }
            }
            DirEntryKind::File {
                size,
                blob_blake3,
                chunks,
                ..
            } => {
                let op = format!("verify failed (file {})", entry.path);
                let plains = if jobs <= 1 {
                    let mut plains = Vec::with_capacity(chunks.len());
                    for (i, chunk_entry) in chunks.iter().enumerate() {
                        let expected_len = entry_length(chunks, i).expect("entry index in range");
                        match source.has(&chunk_entry.chunk_id) {
                            Ok(true) => {}
                            Ok(false) => bail!(
                                "{op}: chunk {} missing from source (entry {i})",
                                chunk_entry.chunk_id
                            ),
                            Err(e) => bail!(
                                "{op}: chunk {} presence check error (entry {i}): {e}",
                                chunk_entry.chunk_id
                            ),
                        }
                        let plain = source.get(&chunk_entry.chunk_id).with_context(|| {
                            format!(
                                "{op}: chunk {} unreadable or hash mismatch (entry {i})",
                                chunk_entry.chunk_id
                            )
                        })?;
                        if plain.len() as u64 != expected_len {
                            bail!(
                                "{op}: chunk {} length mismatch (got {}, expected {expected_len})",
                                chunk_entry.chunk_id,
                                plain.len()
                            );
                        }
                        plains.push(plain);
                        prog.tick();
                    }
                    plains
                } else {
                    fetch_entry_plains(source, chunks, jobs, &op, Some(&prog))?
                };

                let mut hasher = blake3::Hasher::new();
                let mut assembled: u64 = 0;
                for plain in &plains {
                    hasher.update(plain);
                    assembled += plain.len() as u64;
                }
                if assembled != *size {
                    bail!("{op}: assembled size {assembled} != file size {size}");
                }
                let blob_id = ChunkId::from_bytes(*hasher.finalize().as_bytes());
                if blob_id != *blob_blake3 {
                    bail!("{op}: blob_blake3 mismatch (got {blob_id}, archive has {blob_blake3})");
                }
            }
        }
    }

    match format {
        CliFormat::Text => {
            if symlink_count > 0 {
                eprintln!(
                    "verify: ok ({} file{}, {} symlink{}, {} chunk{})",
                    file_count,
                    if file_count == 1 { "" } else { "s" },
                    symlink_count,
                    if symlink_count == 1 { "" } else { "s" },
                    total_chunks,
                    if total_chunks == 1 { "" } else { "s" }
                );
            } else {
                eprintln!(
                    "verify: ok ({} file{}, {} chunk{})",
                    file_count,
                    if file_count == 1 { "" } else { "s" },
                    total_chunks,
                    if total_chunks == 1 { "" } else { "s" }
                );
            }
        }
        CliFormat::Json => {
            let mut obj = serde_json::json!({
                "ok": true,
                "kind": "cfdir",
                "files": file_count,
                "chunks": total_chunks,
                "symlinks": symlink_count,
            });
            apply_cache_ops_json(&mut obj, cache_stats);
            println!("{obj}");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_extract(
    source: Option<&dyn ChunkSource>,
    archive_path: &Path,
    out_dir: &Path,
    jobs: usize,
    force: bool,
    skip_unchanged: bool,
    skip_trust_mtime: bool,
    dry_run: bool,
    format: CliFormat,
    path_filter: &PathFilter,
    progress: bool,
    cache_stats: Option<&CacheStatsRef>,
) -> Result<()> {
    match peek_listing_kind(archive_path)? {
        ListingKind::DirArchive => {}
        ListingKind::Index => bail!(
            "extract expects a `.cfdir` archive; {} looks like a `.cfidx` (use `cat` for single-blob)",
            archive_path.display()
        ),
    }

    let archive = load_dir_archive(archive_path)?;
    archive
        .validate()
        .map_err(|e| anyhow::anyhow!("archive structure: {e}"))?;

    if dry_run {
        return cmd_extract_dry_run(
            &archive,
            out_dir,
            force,
            skip_unchanged,
            skip_trust_mtime,
            format,
            path_filter,
            progress,
            cache_stats,
        );
    }

    let source = source.expect("non-dry-run extract always opens a chunk source");

    if out_dir.exists() {
        if out_dir.is_file() {
            // Never replace the output root with a directory tree, even with --force.
            bail!(
                "extract output {} exists and is a file (refusing to overwrite)",
                out_dir.display()
            );
        }
    } else {
        fs::create_dir_all(out_dir)
            .with_context(|| format!("create output dir {}", out_dir.display()))?;
    }

    let mut file_count = 0usize;
    let mut dir_count = 0usize;
    let mut symlink_count = 0usize;
    let mut skipped_count = 0usize;

    // Progress: per filtered File (PathFilter after; includes skip-unchanged
    // judgments). Dir / Symlink entries are not counted in TOTAL. TOTAL known
    // up front.
    let file_total = archive
        .entries
        .iter()
        .filter(|e| path_filter.allows(&e.path) && matches!(e.kind, DirEntryKind::File { .. }))
        .count();
    let prog = ProgressReporter::new(progress, "extract", Some(file_total));

    // Phase 13 M3 / Phase22-M3: read full listing; materialize matching File +
    // Dir + Symlink entries (parents via create_dir_all). Unmatched paths are
    // skipped (not written, never deleted — non-prune). No --delete.
    for entry in &archive.entries {
        if !path_filter.allows(&entry.path) {
            continue;
        }
        let dest = join_archive_path(out_dir, &entry.path)?;
        match &entry.kind {
            DirEntryKind::Dir { mode } => {
                if dest.exists() {
                    if !dest.is_dir() {
                        bail!(
                            "extract target {} already exists and is not a directory (refusing to replace a file with a directory{})",
                            dest.display(),
                            if force {
                                "; --force does not change type"
                            } else {
                                ""
                            },
                        );
                    }
                } else {
                    fs::create_dir_all(&dest)
                        .with_context(|| format!("create dir {}", dest.display()))?;
                }
                apply_file_mode(&dest, *mode)?;
                dir_count += 1;
            }
            DirEntryKind::Symlink { mode, target } => {
                materialize_extract_symlink(
                    &dest,
                    &entry.path,
                    target,
                    *mode,
                    force,
                    skip_unchanged,
                    &mut skipped_count,
                    &mut symlink_count,
                )?;
            }
            DirEntryKind::File {
                mode,
                size,
                mtime_secs,
                blob_blake3,
                chunks,
            } => {
                // Phase 9 / 11: optional skip when dest already matches listing.
                if skip_unchanged {
                    let verdict = judge_extract_unchanged_opts(
                        &dest,
                        *size,
                        blob_blake3,
                        *mtime_secs,
                        skip_trust_mtime,
                    )
                    .with_context(|| format!("stat/hash {}", dest.display()))?;
                    match verdict {
                        UnchangedVerdict::Unchanged => {
                            // Match takes priority over --force: leave file untouched
                            // (no chunk get, no write, no mode change).
                            skipped_count += 1;
                            prog.tick();
                            continue;
                        }
                        UnchangedVerdict::Missing => {
                            // Fall through to normal write (no --force needed).
                        }
                        UnchangedVerdict::TypeMismatch => {
                            // Align messages with the 0.8.0 path: directories are a
                            // hard type conflict; other non-regular nodes use the
                            // same overwrite gate as an existing file.
                            if dest.is_dir() {
                                bail!(
                                    "extract target {} already exists and is a directory (refusing to replace a directory with a file{})",
                                    dest.display(),
                                    if force {
                                        "; --force does not change type"
                                    } else {
                                        ""
                                    },
                                );
                            }
                            if !force {
                                bail!(
                                    "extract target {} already exists (refusing to overwrite; pass --force)",
                                    dest.display()
                                );
                            }
                            // --force: fall through (File::create may replace the node).
                        }
                        UnchangedVerdict::SizeMismatch | UnchangedVerdict::ContentMismatch => {
                            if !force {
                                bail!(
                                    "extract target {} already exists (refusing to overwrite; pass --force)",
                                    dest.display()
                                );
                            }
                            // --force: fall through to rewrite.
                        }
                    }
                } else if dest.exists() {
                    if dest.is_dir() {
                        bail!(
                            "extract target {} already exists and is a directory (refusing to replace a directory with a file{})",
                            dest.display(),
                            if force {
                                "; --force does not change type"
                            } else {
                                ""
                            },
                        );
                    }
                    if !force {
                        bail!(
                            "extract target {} already exists (refusing to overwrite; pass --force)",
                            dest.display()
                        );
                    }
                    // --force: truncate/overwrite existing regular file via File::create below.
                }
                if let Some(parent) = dest.parent() {
                    if !parent.as_os_str().is_empty() {
                        fs::create_dir_all(parent)
                            .with_context(|| format!("create parent dir {}", parent.display()))?;
                    }
                }

                let op = format!("extract (file {})", entry.path);
                let plains = fetch_entry_plains(source, chunks, jobs, &op, None)?;

                let file = File::create(&dest)
                    .with_context(|| format!("create file {}", dest.display()))?;
                let mut writer = BufWriter::new(file);
                let mut hasher = blake3::Hasher::new();
                let mut assembled: u64 = 0;
                for plain in &plains {
                    writer
                        .write_all(plain)
                        .with_context(|| format!("write {}", dest.display()))?;
                    hasher.update(plain);
                    assembled += plain.len() as u64;
                }
                writer
                    .flush()
                    .with_context(|| format!("flush {}", dest.display()))?;
                writer
                    .into_inner()
                    .with_context(|| format!("finalize {}", dest.display()))?
                    .sync_all()
                    .with_context(|| format!("fsync {}", dest.display()))?;

                if assembled != *size {
                    bail!("{op}: assembled size {assembled} != archive size {size}");
                }
                let blob_id = ChunkId::from_bytes(*hasher.finalize().as_bytes());
                if blob_id != *blob_blake3 {
                    bail!("{op}: blob_blake3 mismatch (got {blob_id}, archive has {blob_blake3})");
                }
                apply_file_mode(&dest, *mode)?;
                file_count += 1;
                prog.tick();
            }
        }
    }

    match format {
        CliFormat::Text => {
            if skip_unchanged {
                // G3 / M2: field names skipped= / wrote= / dirs= (nailed);
                // Phase22-M3 additive symlinks= when any symlink was materialized.
                if symlink_count > 0 {
                    eprintln!(
                        "extract: {} skipped={skipped_count} wrote={file_count} dirs={dir_count} symlinks={symlink_count}",
                        out_dir.display(),
                    );
                } else {
                    eprintln!(
                        "extract: {} skipped={skipped_count} wrote={file_count} dirs={dir_count}",
                        out_dir.display(),
                    );
                }
            } else if symlink_count > 0 {
                eprintln!(
                    "extract: wrote {} ({} file{}, {} dir{}, {} symlink{})",
                    out_dir.display(),
                    file_count,
                    if file_count == 1 { "" } else { "s" },
                    dir_count,
                    if dir_count == 1 { "" } else { "s" },
                    symlink_count,
                    if symlink_count == 1 { "" } else { "s" },
                );
            } else {
                // ≡ 0.8.0 / 1.0.0 summary line when --skip-unchanged is off and no symlinks.
                eprintln!(
                    "extract: wrote {} ({} file{}, {} dir{})",
                    out_dir.display(),
                    file_count,
                    if file_count == 1 { "" } else { "s" },
                    dir_count,
                    if dir_count == 1 { "" } else { "s" }
                );
            }
        }
        CliFormat::Json => {
            // Always emit skipped/wrote/dirs for scripts (skipped=0 when flag off).
            // Phase22-M3: additive wrote_symlinks / symlinks (same count).
            let mut obj = serde_json::json!({
                "ok": true,
                "dry_run": false,
                "skipped": skipped_count,
                "wrote": file_count,
                "dirs": dir_count,
                "wrote_symlinks": symlink_count,
                "symlinks": symlink_count,
            });
            apply_cache_ops_json(&mut obj, cache_stats);
            println!("{obj}");
        }
    }
    Ok(())
}

/// Phase 9 M3 / G2: dry-run extract — no writes, no chunk gets.
///
/// Without `--skip-unchanged`: do not open source/store; classify by local
/// presence only (`would_write` vs `would_fail`). With skip: only read local
/// dests for size+BLAKE3 judgment. Exit **0** when listing is valid (even if
/// `would_fail > 0`).
///
/// Phase23-M6: additive `would_symlinks` counts Symlink entries classified as
/// would-write (missing dest, or existing symlink + `--force`). `would_write`
/// still includes those cases (≡ 1.12). Absolute/empty/conflict → `would_fail`
/// only. Always emit `would_symlinks` in JSON (incl. **0**). ≠ prune ≠ sync ≠
/// pack ≠ write mount.
#[allow(clippy::too_many_arguments)]
fn cmd_extract_dry_run(
    archive: &DirArchive,
    out_dir: &Path,
    force: bool,
    skip_unchanged: bool,
    skip_trust_mtime: bool,
    format: CliFormat,
    path_filter: &PathFilter,
    progress: bool,
    cache_stats: Option<&CacheStatsRef>,
) -> Result<()> {
    // Refuse only when the named output root already exists as a file — same
    // hard gate as real extract; we still create/modify nothing.
    if out_dir.exists() && out_dir.is_file() {
        bail!(
            "extract output {} exists and is a file (refusing to overwrite)",
            out_dir.display()
        );
    }

    let mut would_skip = 0usize;
    let mut would_write = 0usize;
    let mut would_dirs = 0usize;
    let mut would_fail = 0usize;
    // Phase23-M6 / P1: additive counter for Symlink entries classified as
    // would-write (symmetry with write-path `wrote_symlinks`). Does **not**
    // change `would_write` (still includes symlink writes ≡ 1.12).
    let mut would_symlinks = 0usize;

    // Same File-unit progress as real extract (PathFilter after; Dir not counted).
    let file_total = archive
        .entries
        .iter()
        .filter(|e| path_filter.allows(&e.path) && matches!(e.kind, DirEntryKind::File { .. }))
        .count();
    let prog = ProgressReporter::new(progress, "extract", Some(file_total));

    for entry in &archive.entries {
        if !path_filter.allows(&entry.path) {
            continue;
        }
        let dest = join_archive_path(out_dir, &entry.path)?;
        match &entry.kind {
            DirEntryKind::Dir { .. } => {
                if dest.exists() && !dest.is_dir() {
                    would_fail += 1;
                } else {
                    would_dirs += 1;
                }
            }
            DirEntryKind::Symlink { target, .. } => {
                // Phase22-M3: dry-run classify symlink destinations (no writes).
                if target.is_empty() || Path::new(target).is_absolute() {
                    would_fail += 1;
                    continue;
                }
                match fs::symlink_metadata(&dest) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        would_write += 1;
                        would_symlinks += 1;
                    }
                    Err(_) => {
                        would_fail += 1;
                    }
                    Ok(meta) => {
                        let ft = meta.file_type();
                        if ft.is_dir() {
                            // Never replace a directory with a symlink.
                            would_fail += 1;
                        } else if ft.is_symlink() {
                            let same_target = fs::read_link(&dest).ok().is_some_and(|p| {
                                p.as_os_str() == std::ffi::OsStr::new(target.as_str())
                            });
                            if skip_unchanged && same_target {
                                would_skip += 1;
                            } else if force {
                                would_write += 1;
                                would_symlinks += 1;
                            } else {
                                would_fail += 1;
                            }
                        } else {
                            // Regular file / other: refuse even with --force
                            // (force only overwrites an existing symlink).
                            would_fail += 1;
                        }
                    }
                }
            }
            DirEntryKind::File {
                size,
                mtime_secs,
                blob_blake3,
                ..
            } => {
                if skip_unchanged {
                    let verdict = judge_extract_unchanged_opts(
                        &dest,
                        *size,
                        blob_blake3,
                        *mtime_secs,
                        skip_trust_mtime,
                    )
                    .with_context(|| format!("stat/hash {}", dest.display()))?;
                    match verdict {
                        UnchangedVerdict::Unchanged => {
                            would_skip += 1;
                        }
                        UnchangedVerdict::Missing => {
                            would_write += 1;
                        }
                        UnchangedVerdict::TypeMismatch => {
                            if dest.is_dir() {
                                // Type conflict: --force does not change type.
                                would_fail += 1;
                            } else if force {
                                would_write += 1;
                            } else {
                                would_fail += 1;
                            }
                        }
                        UnchangedVerdict::SizeMismatch | UnchangedVerdict::ContentMismatch => {
                            if force {
                                would_write += 1;
                            } else {
                                would_fail += 1;
                            }
                        }
                    }
                } else if dest.exists() {
                    if dest.is_dir() {
                        would_fail += 1;
                    } else if force {
                        would_write += 1;
                    } else {
                        would_fail += 1;
                    }
                } else {
                    would_write += 1;
                }
                prog.tick();
            }
        }
    }

    // G3 / M3: nailed field names (would_fail included — dry-run conflict path).
    // Phase23-M6: always emit additive `would_symlinks` (0 when none / no Symlink).
    match format {
        CliFormat::Text => {
            eprintln!(
                "extract: dry-run: would_skip={would_skip} would_write={would_write} would_dirs={would_dirs} would_fail={would_fail} would_symlinks={would_symlinks}"
            );
        }
        CliFormat::Json => {
            let mut obj = serde_json::json!({
                "ok": true,
                "dry_run": true,
                "would_skip": would_skip,
                "would_write": would_write,
                "would_dirs": would_dirs,
                "would_fail": would_fail,
                "would_symlinks": would_symlinks,
            });
            apply_cache_ops_json(&mut obj, cache_stats);
            println!("{obj}");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_doctor(
    source: &dyn ChunkSource,
    origin_spec: &str,
    index_paths: &[PathBuf],
    deep: bool,
    no_probe: bool,
    jobs: usize,
    http_retries: u32,
    format: CliFormat,
    progress: bool,
    cache_stats: Option<&CacheStatsRef>,
    path_filter: &PathFilter,
) -> Result<()> {
    // Optional local-store meta.toml summary.
    maybe_print_local_store_meta(origin_spec);

    // Optional one-shot HTTP base probe (connectivity only).
    let trimmed = origin_spec.trim();
    let is_http = trimmed.starts_with("http://") || trimmed.starts_with("https://");
    if is_http && !no_probe {
        probe_http_base(trimmed)?;
    }

    // Load + validate all listings (`.cfidx` / `.cfdir`) first (serial; cheap).
    // PathFilter narrows `.cfdir` File chunk ids; `.cfidx` + active filter → error.
    let mut checks: Vec<(String, ChunkId)> = Vec::new();
    let mut listings_ok = 0usize;
    for listing_path in index_paths {
        let display = listing_path.display().to_string();
        for id in listing_chunk_ids_filtered(listing_path, path_filter)? {
            checks.push((display.clone(), id));
        }
        listings_ok += 1;
    }
    let checked = checks.len();

    // Progress is per checked chunk id (TOTAL = referenced chunk ids).
    let prog = ProgressReporter::new(progress, "doctor", Some(checked));

    let mut missing: Vec<ChunkId> = Vec::new();

    if jobs <= 1 {
        // Serial fail-fast ≡ 0.3.0.
        for (listing_display, chunk_id) in &checks {
            let present = if deep {
                match source.get(chunk_id) {
                    Ok(_bytes) => true,
                    Err(chunkforge_store::SourceError::NotFound(_)) => false,
                    Err(e) => {
                        bail!(
                            "doctor: chunk {chunk_id} check error (listing {listing_display}): {e}"
                        );
                    }
                }
            } else {
                match source.has(chunk_id) {
                    Ok(true) => true,
                    Ok(false) => false,
                    Err(e) => {
                        bail!(
                            "doctor: chunk {chunk_id} presence check error (listing {listing_display});                              retry with --deep to use get instead of has: {e}"
                        );
                    }
                }
            };
            if !present {
                missing.push(*chunk_id);
            }
            prog.tick();
        }
    } else {
        let outcomes = parallel::map_indexed(&checks, jobs, |_i, (listing_display, chunk_id)| {
            let result = if deep {
                match source.get(chunk_id) {
                    Ok(_bytes) => Ok(true),
                    Err(chunkforge_store::SourceError::NotFound(_)) => Ok(false),
                    Err(e) => Err(format!(
                        "doctor: chunk {chunk_id} check error (listing {listing_display}): {e}"
                    )),
                }
            } else {
                match source.has(chunk_id) {
                    Ok(true) => Ok(true),
                    Ok(false) => Ok(false),
                    Err(e) => Err(format!(
                        "doctor: chunk {chunk_id} presence check error (listing {listing_display});                          retry with --deep to use get instead of has: {e}"
                    )),
                }
            };
            // Tick after each worker unit completes (like gc/push).
            prog.tick();
            result
        });

        let mut first_err: Option<String> = None;
        for (outcome, (_disp, chunk_id)) in outcomes.into_iter().zip(checks.iter()) {
            match outcome {
                Ok(true) => {}
                Ok(false) => missing.push(*chunk_id),
                Err(msg) => {
                    if first_err.is_none() {
                        first_err = Some(msg);
                    }
                }
            }
        }
        if let Some(msg) = first_err {
            bail!("{msg}");
        }
    }

    // Deduplicate while sorting (multi-listing overlap) — same as prior behaviour.
    missing.sort();
    missing.dedup();

    if missing.is_empty() {
        match format {
            CliFormat::Text => {
                eprintln!(
                    "doctor: ok ({} listing{}, {} chunk id{} checked, deep={}, retries={http_retries})",
                    listings_ok,
                    if listings_ok == 1 { "" } else { "s" },
                    checked,
                    if checked == 1 { "" } else { "s" },
                    deep
                );
            }
            CliFormat::Json => {
                let mut obj = serde_json::json!({
                    "ok": true,
                    "listings": listings_ok,
                    "checked": checked,
                    "missing": 0,
                    "deep": deep,
                    "retries": http_retries,
                });
                apply_cache_ops_json(&mut obj, cache_stats);
                println!("{obj}");
            }
        }
        Ok(())
    } else {
        match format {
            CliFormat::Text => {
                for id in &missing {
                    println!("{id}");
                }
            }
            CliFormat::Json => {
                let missing_ids: Vec<String> = missing.iter().map(|id| id.to_string()).collect();
                let mut obj = serde_json::json!({
                    "ok": false,
                    "listings": listings_ok,
                    "checked": checked,
                    "missing": missing_ids,
                    "deep": deep,
                    "retries": http_retries,
                });
                apply_cache_ops_json(&mut obj, cache_stats);
                println!("{obj}");
            }
        }
        bail!(
            "doctor: {} missing chunk{} ({} listing{}, {} checked)",
            missing.len(),
            if missing.len() == 1 { "" } else { "s" },
            listings_ok,
            if listings_ok == 1 { "" } else { "s" },
            checked
        );
    }
}

fn cmd_gc(
    store_path: &Path,
    index_paths: &[PathBuf],
    apply: bool,
    jobs: usize,
    format: CliFormat,
    progress: bool,
) -> Result<()> {
    let store = Store::open(store_path)
        .with_context(|| format!("open store at {}", store_path.display()))?;

    let (referenced, listings_ok) = union_listing_chunk_ids(index_paths)?;

    let listed = store
        .list_chunk_ids()
        .with_context(|| format!("list chunks under {}", store_path.display()))?;

    let mut unreferenced: Vec<ChunkId> = listed
        .into_iter()
        .filter(|id| !referenced.contains(id))
        .collect();
    unreferenced.sort();

    let dry_run = !apply;
    let candidate_count = unreferenced.len();
    let deleted_count = if apply { candidate_count } else { 0 };

    if unreferenced.is_empty() {
        match format {
            CliFormat::Text => {
                eprintln!(
                    "gc: nothing to reclaim ({} listing{}, {} referenced chunk id{}, dry_run={})",
                    listings_ok,
                    if listings_ok == 1 { "" } else { "s" },
                    referenced.len(),
                    if referenced.len() == 1 { "" } else { "s" },
                    dry_run
                );
            }
            CliFormat::Json => {
                // Phase12-M2: always emit both unreferenced (candidates) and
                // deleted (0 on dry-run); see docs/doctor-gc.md / Phase12 §3.2.
                let obj = serde_json::json!({
                    "ok": true,
                    "dry_run": dry_run,
                    "applied": apply,
                    "listings": listings_ok,
                    "referenced": referenced.len(),
                    "unreferenced": 0,
                    "deleted": 0,
                });
                println!("{obj}");
            }
        }
        return Ok(());
    }

    // Text: ordered stdout paths (serial) so dry-run stays stable across --jobs.
    // Json: do not list paths — the JSON object is the sole stdout payload.
    if format == CliFormat::Text {
        for id in &unreferenced {
            let path = store.chunk_path(id);
            println!("{}", path.display());
        }
    }

    // Progress only ticks on `--apply` deletes (dry-run is an ordered path dump).
    let prog = ProgressReporter::new(progress, "gc", Some(unreferenced.len()));
    if apply {
        // Per-id .cnk files are independent; parallel::map_indexed with jobs=1
        // stays on the calling thread (≡ 1.1.0 serial). Result set is identical
        // for any jobs >= 1.
        let outcomes = parallel::map_indexed(&unreferenced, jobs, |_i, id| {
            let r = store
                .remove(id)
                .with_context(|| format!("delete unreferenced chunk {id}"));
            prog.tick();
            r
        });
        for r in outcomes {
            r?;
        }
    }

    match format {
        CliFormat::Text => {
            if apply {
                eprintln!(
                    "gc: deleted {} unreferenced chunk{} ({} listing{}, {} referenced retained)",
                    candidate_count,
                    if candidate_count == 1 { "" } else { "s" },
                    listings_ok,
                    if listings_ok == 1 { "" } else { "s" },
                    referenced.len()
                );
            } else {
                eprintln!(
                    "gc: dry-run: {} unreferenced chunk{} (pass --apply to delete; {} listing{}, {} referenced)",
                    candidate_count,
                    if candidate_count == 1 { "" } else { "s" },
                    listings_ok,
                    if listings_ok == 1 { "" } else { "s" },
                    referenced.len()
                );
            }
        }
        CliFormat::Json => {
            let obj = serde_json::json!({
                "ok": true,
                "dry_run": dry_run,
                "applied": apply,
                "listings": listings_ok,
                "referenced": referenced.len(),
                "unreferenced": candidate_count,
                "deleted": deleted_count,
            });
            println!("{obj}");
        }
    }
    Ok(())
}

fn open_chunk_sink(
    dest: &str,
    http_tmpl: &HttpTemplateArgs,
    compression: Option<Compression>,
) -> Result<Box<dyn ChunkSink>> {
    let trimmed = dest.trim();

    if is_http_spec(trimmed) {
        // Disk --compression is local/`file://` create-only; never silently
        // no-op on HTTP (including explicit `--compression none`).
        if compression.is_some() {
            bail!(
                "--compression applies only to local/file:// --dest create                  (omit ≡ none ≡ 1.10); http(s):// destinations reject any                  --compression (including explicit none) — disk zstd is not                  HTTP wire compression / Content-Encoding, and push stays                  single-dest (≠ --fallback / store recompress)"
            );
        }
        let mut builder = HttpChunkSink::builder(trimmed)
            .timeout(Some(Duration::from_secs(30)))
            .retry_policy(retry_policy_from_http_args(http_tmpl));
        if let Some(ref tmpl) = http_tmpl.url_template {
            builder = builder.url_template(tmpl.clone());
        }
        if let Some(ref prefix) = http_tmpl.prefix {
            builder = builder.prefix(prefix.clone());
        }
        for raw in &http_tmpl.headers {
            let (name, value_tmpl) = parse_header_flag(raw)?;
            builder = builder.header(name, value_tmpl);
        }
        if let Some(signer) = sigv4_signer_from_http_args(http_tmpl)? {
            builder = builder.aws_sigv4(signer);
        }
        let sink = builder.build().context("build HTTP chunk sink")?;
        return Ok(Box::new(sink));
    }

    // Local path or file:// → Store as ChunkSink (single dest; ≠ fallback chain).
    // HTTP-only knobs must not silently no-op on a local dest.
    if http_template_flags_set(http_tmpl) || http_tmpl.http_retries != 0 {
        bail!(
            "--url-template / --prefix / --header / --aws-sigv4 / --http-retries apply only to              http(s):// destinations; got local/file --dest {trimmed:?}              (push local dest is a single Store ChunkSink — not a fallback chain / multi-dest)"
        );
    }

    let path = parse_store_location(trimmed)
        .with_context(|| format!("parse push --dest {trimmed:?} (local path or file:// URL)"))?;
    // Existing store: open as recorded (explicit mismatch → non-zero).
    // Missing: create with omit ≡ none ≡ 1.10 (or explicit zstd). Same
    // open_or_create_store helper as pull/make/store create.
    let store = open_or_create_store(&path, compression)?;
    Ok(Box::new(store))
}

#[allow(clippy::too_many_arguments)]
fn cmd_push(
    store_path: &Path,
    dest: &str,
    http_tmpl: &HttpTemplateArgs,
    dry_run: bool,
    verify: bool,
    index_paths: &[PathBuf],
    jobs: usize,
    format: CliFormat,
    progress: bool,
    path_filter: &PathFilter,
    compression: Option<Compression>,
) -> Result<()> {
    let store = Store::open(store_path)
        .with_context(|| format!("open store at {}", store_path.display()))?;
    let sink = open_chunk_sink(dest, http_tmpl, compression)?;

    let (referenced, listings_ok) = union_listing_chunk_ids_filtered(index_paths, path_filter)?;

    let mut ids: Vec<ChunkId> = referenced.into_iter().collect();
    ids.sort();

    #[derive(Clone, Copy)]
    enum PushOne {
        Skipped,
        Uploaded,
        Failed(SummaryFailureBucket),
    }

    let store_display = store_path.display().to_string();
    let prog = ProgressReporter::new(progress, "push", Some(ids.len()));
    let outcomes = parallel::map_indexed(&ids, jobs, |_i, id| {
        let outcome = (|| {
            let plain = match store.get(id) {
                Ok(bytes) => bytes,
                Err(e) => {
                    let msg =
                        format!("local chunk {id} unavailable from store {store_display}: {e}");
                    if format == CliFormat::Text {
                        eprintln!("push: fail {id}: {msg}");
                    }
                    // Local store miss / I/O → permanent (not an HTTP transient).
                    return (PushOne::Failed(SummaryFailureBucket::Permanent), Some(msg));
                }
            };

            match sink.has(id) {
                Ok(true) => return (PushOne::Skipped, None),
                Ok(false) => {}
                Err(e) => {
                    let msg = format!("remote has check failed for {id}: {e}");
                    if format == CliFormat::Text {
                        eprintln!("push: fail {id}: {msg}");
                    }
                    let bucket = classify_sink_error(&e).summary_bucket();
                    return (PushOne::Failed(bucket), Some(msg));
                }
            }

            if dry_run {
                return (PushOne::Uploaded, None);
            }

            match ChunkSink::put(&sink, id, &plain) {
                Ok(PutOutcome::Written) => (PushOne::Uploaded, None),
                Ok(PutOutcome::SkippedExists) => (PushOne::Skipped, None),
                Err(e) => {
                    let msg = format!("put failed for {id}: {e}");
                    if format == CliFormat::Text {
                        eprintln!("push: fail {id}: {msg}");
                    }
                    let bucket = classify_sink_error(&e).summary_bucket();
                    (PushOne::Failed(bucket), Some(msg))
                }
            }
        })();
        prog.tick();
        outcome
    });

    let mut skipped = 0usize;
    let mut uploaded = 0usize;
    let mut failed_transient = 0usize;
    let mut failed_permanent = 0usize;
    let mut first_error: Option<String> = None;
    for (outcome, err) in outcomes {
        match outcome {
            PushOne::Skipped => skipped += 1,
            PushOne::Uploaded => uploaded += 1,
            PushOne::Failed(bucket) => {
                match bucket {
                    SummaryFailureBucket::Transient => failed_transient += 1,
                    SummaryFailureBucket::Permanent => failed_permanent += 1,
                }
                if first_error.is_none() {
                    first_error = err;
                }
            }
        }
    }
    let failed = failed_transient + failed_permanent;

    let retries = http_tmpl.http_retries;
    let unique_chunks = ids.len();
    let ok = failed == 0;
    match format {
        CliFormat::Text => {
            eprintln!(
                "push: skipped={skipped} uploaded={uploaded} failed={failed}          failed_transient={failed_transient} failed_permanent={failed_permanent}          retries={retries} ({unique_chunks} unique chunk id{}, {listings_ok} listing{}, dry_run={dry_run})",
                if unique_chunks == 1 { "" } else { "s" },
                if listings_ok == 1 { "" } else { "s" },
            );
        }
        CliFormat::Json => {
            let obj = serde_json::json!({
                "ok": ok,
                "skipped": skipped,
                "uploaded": uploaded,
                "failed": failed,
                "failed_transient": failed_transient,
                "failed_permanent": failed_permanent,
                "retries": retries,
                "unique_chunks": unique_chunks,
                "listings": listings_ok,
                "dry_run": dry_run,
            });
            println!("{obj}");
        }
    }

    if failed > 0 {
        // Push already failed: do not claim verify success; skip post-verify.
        // Exit code is independent of `--format` (JSON already emitted above).
        bail!(
            "push: {failed} failure{}{}",
            if failed == 1 { "" } else { "s" },
            first_error
                .map(|m| format!(" (first: {m})"))
                .unwrap_or_default()
        );
    }

    if verify {
        if dry_run {
            eprintln!(
                "push: --verify skipped (dry-run; nothing was uploaded — remote not verified)"
            );
            return Ok(());
        }
        eprintln!(
            "push: verifying {} listing{} against --dest …",
            listings_ok,
            if listings_ok == 1 { "" } else { "s" },
        );
        let source = open_primary_source(dest, http_tmpl)
            .context("build chunk source from --dest for push --verify")?;
        // Post-push verify stays full-listing (path filter only shrunk the upload
        // set). Empty PathFilter ≡ 1.9.0 verify behaviour.
        let full =
            &PathFilter::new(Vec::<String>::new(), Vec::<String>::new()).expect("empty PathFilter");
        for path in index_paths {
            cmd_verify(
                source.as_ref(),
                path,
                jobs,
                CliFormat::Text,
                false,
                None,
                full,
            )
            .with_context(|| {
                format!(
                    "push --verify failed for {} (remote missing/corrupt chunk or hash mismatch)",
                    path.display()
                )
            })?;
        }
        eprintln!(
            "push: verify ok ({} listing{})",
            listings_ok,
            if listings_ok == 1 { "" } else { "s" },
        );
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_pull(
    store_path: &Path,
    source_spec: &str,
    fallbacks: &[String],
    cache: Option<&Path>,
    cache_max_bytes: Option<u64>,
    cache_stats: bool,
    http_tmpl: &HttpTemplateArgs,
    dry_run: bool,
    verify: bool,
    index_paths: &[PathBuf],
    jobs: usize,
    format: CliFormat,
    progress: bool,
    path_filter: &PathFilter,
    compression: Option<Compression>,
) -> Result<()> {
    let (source, stats) = open_chunk_source(
        None,
        Some(source_spec),
        cache,
        cache_max_bytes,
        http_tmpl,
        fallbacks,
    )
    .with_context(|| format!("open chunk source {source_spec:?}"))?;
    let _cache_stats_emit = CacheStatsEmit {
        stats: &stats,
        enabled: cache_stats,
    };

    let (referenced, listings_ok) = union_listing_chunk_ids_filtered(index_paths, path_filter)?;

    let mut ids: Vec<ChunkId> = referenced.into_iter().collect();
    ids.sort();

    // Dry-run must not create / write the store. Open only if meta already exists.
    let store = if dry_run {
        let meta = store_path.join("meta.toml");
        if meta.is_file() {
            Some(
                Store::open(store_path)
                    .with_context(|| format!("open store at {}", store_path.display()))?,
            )
        } else {
            None
        }
    } else {
        Some(open_or_create_store(store_path, compression)?)
    };

    #[derive(Clone, Copy)]
    enum PullOne {
        Skipped,
        Fetched,
        Failed(SummaryFailureBucket),
    }

    let store_display = store_path.display().to_string();
    let prog = ProgressReporter::new(progress, "pull", Some(ids.len()));
    let outcomes = parallel::map_indexed(&ids, jobs, |_i, id| {
        let outcome = (|| {
            let already = match store.as_ref() {
                Some(s) => s.has(id),
                None => false,
            };
            if already {
                return (PullOne::Skipped, None);
            }

            if dry_run {
                return (PullOne::Fetched, None);
            }

            let store = store
                .as_ref()
                .expect("non-dry-run pull always opens or creates the store");

            let plain = match source.get(id) {
                Ok(bytes) => bytes,
                Err(e) => {
                    let msg = format!("source get failed for {id} (store {store_display}): {e}");
                    if format == CliFormat::Text {
                        eprintln!("pull: fail {id}: {msg}");
                    }
                    let bucket = classify_source_error(&e).summary_bucket();
                    return (PullOne::Failed(bucket), Some(msg));
                }
            };

            match ChunkSink::put(store, id, &plain) {
                Ok(PutOutcome::Written) => (PullOne::Fetched, None),
                Ok(PutOutcome::SkippedExists) => (PullOne::Skipped, None),
                Err(e) => {
                    let msg = format!("store put failed for {id}: {e}");
                    if format == CliFormat::Text {
                        eprintln!("pull: fail {id}: {msg}");
                    }
                    let bucket = classify_sink_error(&e).summary_bucket();
                    (PullOne::Failed(bucket), Some(msg))
                }
            }
        })();
        prog.tick();
        outcome
    });

    let mut skipped = 0usize;
    let mut fetched = 0usize;
    let mut failed_transient = 0usize;
    let mut failed_permanent = 0usize;
    let mut first_error: Option<String> = None;
    for (outcome, err) in outcomes {
        match outcome {
            PullOne::Skipped => skipped += 1,
            PullOne::Fetched => fetched += 1,
            PullOne::Failed(bucket) => {
                match bucket {
                    SummaryFailureBucket::Transient => failed_transient += 1,
                    SummaryFailureBucket::Permanent => failed_permanent += 1,
                }
                if first_error.is_none() {
                    first_error = err;
                }
            }
        }
    }
    let failed = failed_transient + failed_permanent;

    let retries = http_tmpl.http_retries;
    let unique_chunks = ids.len();
    let ok = failed == 0;
    match format {
        CliFormat::Text => {
            eprintln!(
                "pull: skipped={skipped} fetched={fetched} failed={failed}          failed_transient={failed_transient} failed_permanent={failed_permanent}          retries={retries} ({unique_chunks} unique chunk id{}, {listings_ok} listing{}, dry_run={dry_run})",
                if unique_chunks == 1 { "" } else { "s" },
                if listings_ok == 1 { "" } else { "s" },
            );
        }
        CliFormat::Json => {
            let mut obj = serde_json::json!({
                "ok": ok,
                "skipped": skipped,
                "fetched": fetched,
                "failed": failed,
                "failed_transient": failed_transient,
                "failed_permanent": failed_permanent,
                "retries": retries,
                "unique_chunks": unique_chunks,
                "listings": listings_ok,
                "dry_run": dry_run,
            });
            apply_cache_ops_json(&mut obj, stats.as_ref());
            println!("{obj}");
        }
    }

    if failed > 0 {
        // Pull already failed: do not claim verify success; skip post-verify.
        // Exit code is independent of `--format` (JSON already emitted above).
        bail!(
            "pull: {failed} failure{}{}",
            if failed == 1 { "" } else { "s" },
            first_error
                .map(|m| format!(" (first: {m})"))
                .unwrap_or_default()
        );
    }

    if verify {
        if dry_run {
            eprintln!(
                "pull: --verify skipped (dry-run; nothing was written — local store not verified)"
            );
            return Ok(());
        }
        let Some(store) = store.as_ref() else {
            // Defensive: non-dry-run always opens/creates the store above.
            eprintln!(
                "pull: --verify skipped (no local store available — local store not verified)"
            );
            return Ok(());
        };
        eprintln!(
            "pull: verifying {} listing{} against --store …",
            listings_ok,
            if listings_ok == 1 { "" } else { "s" },
        );
        for path in index_paths {
            // Post-pull verify stays full-listing (path filter only shrunk the fetch
            // set). Empty PathFilter ≡ 1.9.0 verify behaviour.
            let full = &PathFilter::new(Vec::<String>::new(), Vec::<String>::new())
                .expect("empty PathFilter");
            cmd_verify(store, path, jobs, CliFormat::Text, false, None, full).with_context(|| {
                format!(
                    "pull --verify failed for {} (local store missing/corrupt chunk or hash mismatch)",
                    path.display()
                )
            })?;
        }
        eprintln!(
            "pull: verify ok ({} listing{})",
            listings_ok,
            if listings_ok == 1 { "" } else { "s" },
        );
    }

    Ok(())
}

fn maybe_print_local_store_meta(origin_spec: &str) {
    let trimmed = origin_spec.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return;
    }
    let path = match chunkforge_remote::parse_store_location(trimmed) {
        Ok(p) => p,
        Err(_) => return,
    };
    let meta_path = path.join("meta.toml");
    match chunkforge_store::StoreMeta::read_from(&meta_path) {
        Ok(meta) => {
            eprintln!(
                "doctor: store meta at {} (magic={}, version={}, compression={})",
                meta_path.display(),
                meta.magic,
                meta.version,
                meta.compression
            );
        }
        Err(_) => {
            // Not a local store / unreadable meta — skip quietly.
        }
    }
}

/// One HEAD (or GET fallback) against the HTTP base URL to confirm reachability.
fn probe_http_base(base: &str) -> Result<()> {
    let base = base.trim_end_matches('/');
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();

    match agent.head(base).call() {
        Ok(_resp) => {
            eprintln!("doctor: base probe ok (HEAD {base})");
            Ok(())
        }
        Err(ureq::Error::StatusCode(404) | ureq::Error::StatusCode(410)) => {
            // Base path itself often has no document; server answered → ok.
            eprintln!("doctor: base probe ok (HEAD {base} → not found, server reachable)");
            Ok(())
        }
        Err(ureq::Error::StatusCode(405) | ureq::Error::StatusCode(501)) => {
            match agent.get(base).call() {
                Ok(mut resp) => {
                    let _ = resp.body_mut().read_to_vec();
                    eprintln!("doctor: base probe ok (GET {base})");
                    Ok(())
                }
                Err(ureq::Error::StatusCode(404) | ureq::Error::StatusCode(410)) => {
                    eprintln!("doctor: base probe ok (GET {base} → not found, server reachable)");
                    Ok(())
                }
                Err(e) => bail!("doctor: base probe failed for {base}: {e}"),
            }
        }
        Err(ureq::Error::StatusCode(code)) => {
            // Any other HTTP status still means the server is reachable.
            eprintln!("doctor: base probe ok (HEAD {base} → HTTP {code})");
            Ok(())
        }
        Err(e) => bail!("doctor: base probe failed for {base}: {e}"),
    }
}

fn cmd_chunk_id(input: &Path, chunk_size: Option<&str>, format: CliFormat) -> Result<()> {
    let params = parse_chunk_size(chunk_size)?;
    let data = fs::read(input).with_context(|| format!("read input {}", input.display()))?;
    let chunks: Vec<ChunkInfo> = chunk_bytes(&data, &params);
    match format {
        CliFormat::Text => {
            for c in &chunks {
                println!("{}\t{}\t{}", c.offset, c.length, c.id);
            }
        }
        CliFormat::Json => {
            let arr: Vec<serde_json::Value> = chunks
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "offset": c.offset,
                        "length": c.length,
                        "id": c.id.to_string(),
                    })
                })
                .collect();
            let obj = serde_json::json!({
                "ok": true,
                "chunks": arr,
            });
            println!("{obj}");
        }
    }
    Ok(())
}

fn cmd_store_create(
    store_path: &Path,
    compression: Option<Compression>,
    format: CliFormat,
) -> Result<()> {
    let c = compression.unwrap_or(Compression::None);
    let _store = Store::create(store_path, c)
        .with_context(|| format!("store create at {}", store_path.display()))?;
    match format {
        CliFormat::Text => {
            eprintln!(
                "store create: ok store={} compression={}",
                store_path.display(),
                c.as_str()
            );
        }
        CliFormat::Json => {
            let obj = serde_json::json!({
                "ok": true,
                "store": store_path.display().to_string(),
                "compression": c.as_str(),
            });
            println!("{obj}");
        }
    }
    Ok(())
}

fn cmd_store_has(store_path: &Path, hex_id: &str, format: CliFormat) -> Result<()> {
    let id = ChunkId::from_hex(hex_id).map_err(|e| anyhow::anyhow!("{e}"))?;
    let store = Store::open(store_path)
        .with_context(|| format!("open store at {}", store_path.display()))?;
    let present = store.has(&id);
    match format {
        CliFormat::Text => {
            if present {
                println!("present\t{id}");
                Ok(())
            } else {
                bail!("missing\t{id}");
            }
        }
        CliFormat::Json => {
            let obj = serde_json::json!({
                "ok": present,
                "present": present,
                "id": id.to_string(),
            });
            println!("{obj}");
            if present {
                Ok(())
            } else {
                bail!("store has: missing {id}");
            }
        }
    }
}

/// Fetch one chunk's plaintext to `-o` via `Store::get_verify`.
///
/// Default (`verify=false`): `get_verify(id, false)` — decode on-disk encoding,
/// skip BLAKE3 re-hash (trust disk; lighter than library `Store::get`, which
/// always verifies). With `--verify`: `get_verify(id, true)` re-hashes.
///
/// JSON field names pinned for ops-json: **`ok`**, **`id`**, **`bytes`**.
/// **Not** scrub / cat / extract / recompress / remove / trim / multi-id.
fn cmd_store_get(
    store_path: &Path,
    hex_id: &str,
    output: &Path,
    verify: bool,
    format: CliFormat,
) -> Result<()> {
    let id =
        ChunkId::from_hex(hex_id).map_err(|e| anyhow::anyhow!("store get: bad chunk id: {e}"))?;
    let store = Store::open(store_path)
        .with_context(|| format!("open store at {}", store_path.display()))?;
    // Phase26 §3.2: default trust on-disk (no re-hash); `--verify` → re-hash.
    // Library `Store::get` always verifies, so CLI uses `get_verify` for both.
    let plain = store.get_verify(&id, verify).map_err(|e| match e {
        StoreError::NotFound(missing) => anyhow::anyhow!("store get: missing {missing}"),
        StoreError::Corrupt(bad) => anyhow::anyhow!("store get: corrupt {bad}"),
        other => anyhow::anyhow!("store get: {other}"),
    })?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create parent dir {}", parent.display()))?;
        }
    }
    fs::write(output, &plain).with_context(|| format!("write output {}", output.display()))?;
    let bytes = plain.len() as u64;
    match format {
        CliFormat::Text => {
            eprintln!("store get: ok id={id} bytes={bytes}");
        }
        CliFormat::Json => {
            // ops-json field names for `store get` (pinned in docs/ops-json.md):
            // ok / id / bytes
            let obj = serde_json::json!({
                "ok": true,
                "id": id.to_string(),
                "bytes": bytes,
            });
            println!("{obj}");
        }
    }
    Ok(())
}

/// Read-only CAS integrity scrub: list loose chunks and `get_verify` each id.
///
/// With `listing`: only the chunk ids referenced by that `.cfidx` / `.cfdir`
/// (unique, sorted). Missing ids count as unreadable. **Local store only** —
/// not remote scrub / ListObjects.
///
/// Prints per-bad-chunk lines (`scrub: corrupt <id>` / `scrub: unreadable <id>`)
/// and a summary `scrub: ok=… corrupt=… unreadable=…`. Never deletes. Exit
/// non-zero iff corrupt+unreadable > 0 (empty store / empty listing → zeros, exit 0).
fn cmd_store_scrub(
    store_path: &Path,
    listing: Option<&Path>,
    jobs: usize,
    format: CliFormat,
    progress: bool,
) -> Result<()> {
    let store = Store::open(store_path)
        .with_context(|| format!("open store at {}", store_path.display()))?;
    let mut ids = if let Some(listing_path) = listing {
        let raw = listing_chunk_ids(listing_path)
            .with_context(|| format!("load listing {}", listing_path.display()))?;
        let mut set: HashSet<ChunkId> = HashSet::new();
        for id in raw {
            set.insert(id);
        }
        let mut v: Vec<ChunkId> = set.into_iter().collect();
        v.sort();
        v
    } else {
        store
            .list_chunk_ids()
            .with_context(|| format!("list chunks in {}", store_path.display()))?
    };
    ids.sort();

    #[derive(Clone, Copy)]
    enum ScrubOne {
        Ok,
        Corrupt,
        Unreadable,
    }

    let prog = ProgressReporter::new(progress, "scrub", Some(ids.len()));
    let outcomes = parallel::map_indexed(&ids, jobs, |_i, id| {
        let outcome = match store.get_verify(id, true) {
            Ok(_) => ScrubOne::Ok,
            Err(StoreError::Corrupt(_)) => ScrubOne::Corrupt,
            Err(_) => ScrubOne::Unreadable,
        };
        prog.tick();
        outcome
    });

    let mut ok_count = 0u64;
    let mut corrupt = 0u64;
    let mut unreadable = 0u64;
    let mut corrupt_ids: Vec<String> = Vec::new();
    let mut unreadable_ids: Vec<String> = Vec::new();
    for (id, outcome) in ids.iter().zip(outcomes.iter()) {
        match outcome {
            ScrubOne::Ok => ok_count += 1,
            ScrubOne::Corrupt => {
                corrupt += 1;
                if format == CliFormat::Text {
                    println!("scrub: corrupt {id}");
                } else {
                    corrupt_ids.push(id.to_string());
                }
            }
            ScrubOne::Unreadable => {
                unreadable += 1;
                if format == CliFormat::Text {
                    println!("scrub: unreadable {id}");
                } else {
                    unreadable_ids.push(id.to_string());
                }
            }
        }
    }

    let checked = ok_count + corrupt + unreadable;
    let ok = corrupt + unreadable == 0;

    match format {
        CliFormat::Text => {
            println!("scrub: ok={ok_count} corrupt={corrupt} unreadable={unreadable}");
        }
        CliFormat::Json => {
            // Phase12-M3: bad ids only in arrays (no text lines); sole stdout
            // payload is one JSON object. See docs/doctor-gc.md / Phase12 §3.2.
            let obj = serde_json::json!({
                "ok": ok,
                "checked": checked,
                "ok_count": ok_count,
                "corrupt": corrupt,
                "unreadable": unreadable,
                "corrupt_ids": corrupt_ids,
                "unreadable_ids": unreadable_ids,
            });
            println!("{obj}");
        }
    }

    if corrupt + unreadable > 0 {
        bail!(
            "scrub: {} bad chunk(s) (corrupt={corrupt} unreadable={unreadable})",
            corrupt + unreadable
        );
    }
    Ok(())
}

/// Local CAS size observation: chunk count + on-disk `.cnk` bytes (+ optional
/// plaintext).
///
/// Text (default): one stdout line
/// `store stats: chunks=N bytes_on_disk=M [bytes_plaintext=P] compression=…`
/// (`bytes_plaintext` only when `Some`).
/// Json: one object (`ok`, `chunks`, `bytes_on_disk`, `bytes_plaintext`,
/// `compression`); `bytes_plaintext` is a number or `null`. No text dual-write.
/// Never deletes. Exit 0 on success for both formats.
///
/// When `decode` is false, uses cheap [`Store::stats`] (none → plaintext ≡
/// on_disk; zstd → `bytes_plaintext=None`). When `decode` is true, uses
/// [`Store::stats_with_decode`] (zstd pays a full-store `get`).
fn cmd_store_stats(store_path: &Path, format: CliFormat, decode: bool) -> Result<()> {
    let store = Store::open(store_path)
        .with_context(|| format!("open store at {}", store_path.display()))?;
    let s = if decode {
        store.stats_with_decode()
    } else {
        store.stats()
    }
    .with_context(|| format!("stats for store {}", store_path.display()))?;
    match format {
        CliFormat::Text => {
            if let Some(plain) = s.bytes_plaintext {
                println!(
                    "store stats: chunks={} bytes_on_disk={} bytes_plaintext={} compression={}",
                    s.chunks,
                    s.bytes_on_disk,
                    plain,
                    s.compression.as_str()
                );
            } else {
                println!(
                    "store stats: chunks={} bytes_on_disk={} compression={}",
                    s.chunks,
                    s.bytes_on_disk,
                    s.compression.as_str()
                );
            }
        }
        CliFormat::Json => {
            let obj = serde_json::json!({
                "ok": true,
                "chunks": s.chunks,
                "bytes_on_disk": s.bytes_on_disk,
                "bytes_plaintext": s.bytes_plaintext,
                "compression": s.compression.as_str(),
            });
            println!("{obj}");
        }
    }
    Ok(())
}

/// Read-only enumeration of loose chunk ids via `Store::list_chunk_ids`.
///
/// Text: one lowercase hex id per line, stably sorted. Json: `{ok, chunks, ids}`.
/// **Not** GC / scrub / trim / LRU — observation only.
fn cmd_store_list(store_path: &Path, format: CliFormat) -> Result<()> {
    let store = Store::open(store_path)
        .with_context(|| format!("open store at {}", store_path.display()))?;
    let mut ids = store
        .list_chunk_ids()
        .with_context(|| format!("list chunks in {}", store_path.display()))?;
    ids.sort();
    match format {
        CliFormat::Text => {
            for id in &ids {
                println!("{id}");
            }
        }
        CliFormat::Json => {
            let hexes: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
            let obj = serde_json::json!({
                "ok": true,
                "chunks": hexes.len(),
                "ids": hexes,
            });
            println!("{obj}");
        }
    }
    Ok(())
}

fn mount_prefetch_note(prefetch: bool, prefetch_chunks: usize) -> String {
    if !prefetch {
        "; --no-prefetch".to_string()
    } else if prefetch_chunks == 1 {
        "; prefetch on".to_string()
    } else {
        format!("; prefetch-chunks={prefetch_chunks}")
    }
}

fn ensure_mount_supported() -> Result<()> {
    #[cfg(feature = "fuse")]
    {
        Ok(())
    }
    #[cfg(not(feature = "fuse"))]
    {
        bail!(
            "mount is unsupported in this build (cargo feature `fuse` disabled).\n\
             On Linux, rebuild with: cargo build -p chunkforge-cli --features fuse\n\
             Runtime requires fuse3 (Debian/Ubuntu: sudo apt install fuse3) and a usable /dev/fuse.\n\
             macOS/Windows are not Phase 2 acceptance platforms (experimental only)."
        );
    }
}

fn cmd_mount(
    source: Box<dyn ChunkSource>,
    index_path: &Path,
    mountpoint: &Path,
    name: Option<&str>,
    prefetch: bool,
    prefetch_chunks: usize,
    path_filter: &PathFilter,
) -> Result<()> {
    #[cfg(feature = "fuse")]
    {
        cmd_mount_fuse(
            source,
            index_path,
            mountpoint,
            name,
            prefetch,
            prefetch_chunks,
            path_filter,
        )
    }
    #[cfg(not(feature = "fuse"))]
    {
        let _ = (
            source,
            index_path,
            mountpoint,
            name,
            prefetch,
            prefetch_chunks,
            path_filter,
        );
        // ensure_mount_supported() already rejected; keep a defensive message.
        ensure_mount_supported()
    }
}

#[cfg(feature = "fuse")]
fn cmd_mount_fuse(
    source: Box<dyn ChunkSource>,
    index_path: &Path,
    mountpoint: &Path,
    name: Option<&str>,
    prefetch: bool,
    prefetch_chunks: usize,
    path_filter: &PathFilter,
) -> Result<()> {
    use chunkforge_fuse::{BlobFs, DirFs, MountOption, default_blob_name, mount_ro};

    if !mountpoint.exists() {
        bail!(
            "mountpoint {} does not exist (create an empty directory first)",
            mountpoint.display()
        );
    }
    if !mountpoint.is_dir() {
        bail!("mountpoint {} is not a directory", mountpoint.display());
    }

    let opts = [
        MountOption::FSName("chunkforge".into()),
        MountOption::AutoUnmount,
        MountOption::DefaultPermissions,
    ];

    let filter_active = !path_filter.paths().is_empty() || !path_filter.excludes().is_empty();

    match peek_listing_kind(index_path)? {
        ListingKind::Index => {
            // `.cfidx` + any path/exclude flag → clear non-zero (same as doctor/verify/push/pull).
            if filter_active {
                bail!(
                    "--path/--path-from/--exclude applies to `.cfdir` File entries; {} looks like a `.cfidx` (use without path flags for full single-blob mount)",
                    index_path.display()
                );
            }
            let blob_name = match name {
                Some(n) => {
                    if n.is_empty() || n.contains('/') || n.contains('\\') {
                        bail!("invalid --name {n:?}: must be a single non-empty path component");
                    }
                    n.to_string()
                }
                None => default_blob_name(index_path),
            };

            let index = load_index(index_path)?;
            let fs = if prefetch {
                BlobFs::new(index, source, blob_name.clone()).with_prefetch_chunks(prefetch_chunks)
            } else {
                BlobFs::new(index, source, blob_name.clone()).with_prefetch(false)
            };

            eprintln!(
                "mount: {} → {}/{} (read-only{}; Ctrl-C or fusermount3 -u to unmount)",
                index_path.display(),
                mountpoint.display(),
                blob_name,
                mount_prefetch_note(prefetch, prefetch_chunks),
            );

            mount_ro(fs, mountpoint, opts).map_err(explain_fuse_mount_error)?;
        }
        ListingKind::DirArchive => {
            if name.is_some() {
                eprintln!(
                    "warning: --name is ignored for `.cfdir` directory mounts ({})",
                    index_path.display()
                );
            }
            let archive = load_dir_archive(index_path)?;
            // PathFilter → filter_dir_archive → DirFs (empty filter ≡ 1.10 full tree).
            let archive = chunkforge_index::filter_dir_archive(&archive, path_filter);
            let fs = if prefetch {
                DirFs::new(archive, source).with_prefetch_chunks(prefetch_chunks)
            } else {
                DirFs::new(archive, source).with_prefetch(false)
            };

            eprintln!(
                "mount: {} → {}/ (directory tree, read-only{}; Ctrl-C or fusermount3 -u to unmount)",
                index_path.display(),
                mountpoint.display(),
                mount_prefetch_note(prefetch, prefetch_chunks),
            );

            mount_ro(fs, mountpoint, opts).map_err(explain_fuse_mount_error)?;
        }
    }

    Ok(())
}

/// Turn a FUSE mount I/O failure into a readable install/permission hint.
#[cfg(feature = "fuse")]
fn explain_fuse_mount_error(err: std::io::Error) -> anyhow::Error {
    let mut hints: Vec<String> = Vec::new();

    let dev_fuse = Path::new("/dev/fuse");
    if !dev_fuse.exists() {
        hints.push(
            "/dev/fuse is missing — load the fuse module or install fuse3 \
             (Debian/Ubuntu: sudo apt install fuse3)."
                .into(),
        );
    } else {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(dev_fuse)
        {
            Ok(_) => {}
            Err(e) => hints.push(format!(
                "cannot open /dev/fuse ({e}); check group membership (often `fuse`) \
                 or device permissions (ls -l /dev/fuse)."
            )),
        }
    }

    if which_cmd("fusermount3").is_none() && which_cmd("fusermount").is_none() {
        hints.push(
            "fusermount3/fusermount not found on PATH — install fuse3 \
             (Debian/Ubuntu: sudo apt install fuse3)."
                .into(),
        );
    }

    let msg = err.to_string();
    let lower = msg.to_lowercase();
    if lower.contains("permission")
        || lower.contains("not permitted")
        || err.raw_os_error() == Some(1)
    {
        hints.push(
            "permission denied mounting — ensure you can use FUSE as this user \
             (no need for allow_other unless mounting for other users)."
                .into(),
        );
    }

    if hints.is_empty() {
        anyhow::anyhow!("FUSE mount failed: {err}")
    } else {
        anyhow::anyhow!(
            "FUSE mount failed: {err}\n{}",
            hints
                .iter()
                .map(|h| format!("  hint: {h}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

#[cfg(feature = "fuse")]
fn which_cmd(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod open_or_create_store_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn create_omitted_is_none() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("store");
        let store = open_or_create_store(&root, None).unwrap();
        assert_eq!(store.compression(), Compression::None);
        let opened = open_or_create_store(&root, None).unwrap();
        assert_eq!(opened.compression(), Compression::None);
    }

    #[test]
    fn create_explicit_zstd_put_get_plaintext_roundtrip() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("store");
        let store = open_or_create_store(&root, Some(Compression::Zstd)).unwrap();
        assert_eq!(store.compression(), Compression::Zstd);
        let data = b"zzzzzzzzzzzzzzzz compressible payload for cli helper zzzzzzzzzzzz";
        let (id, outcome) = store.put(data).unwrap();
        assert!(outcome.is_new());
        assert_eq!(store.get(&id).unwrap(), data);
        // On-disk payload differs from plaintext under zstd.
        let on_disk = std::fs::read(store.chunk_path(&id)).unwrap();
        assert_ne!(on_disk, data);
    }

    #[test]
    fn explicit_matching_opens_ok() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("store");
        open_or_create_store(&root, Some(Compression::Zstd)).unwrap();
        let store = open_or_create_store(&root, Some(Compression::Zstd)).unwrap();
        assert_eq!(store.compression(), Compression::Zstd);

        let root2 = dir.path().join("store-none");
        open_or_create_store(&root2, Some(Compression::None)).unwrap();
        let store2 = open_or_create_store(&root2, Some(Compression::None)).unwrap();
        assert_eq!(store2.compression(), Compression::None);
    }

    #[test]
    fn explicit_conflict_is_error() {
        let dir = tempdir().unwrap();
        let root_none = dir.path().join("store-none");
        open_or_create_store(&root_none, None).unwrap();
        let err = open_or_create_store(&root_none, Some(Compression::Zstd)).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("compression mismatch"),
            "expected mismatch error, got: {msg}"
        );

        let root_z = dir.path().join("store-z");
        open_or_create_store(&root_z, Some(Compression::Zstd)).unwrap();
        let err2 = open_or_create_store(&root_z, Some(Compression::None)).unwrap_err();
        let msg2 = format!("{err2:#}");
        assert!(
            msg2.contains("compression mismatch"),
            "expected mismatch error, got: {msg2}"
        );
    }

    #[test]
    fn omit_opens_existing_zstd_without_error() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("store");
        open_or_create_store(&root, Some(Compression::Zstd)).unwrap();
        let store = open_or_create_store(&root, None).unwrap();
        assert_eq!(store.compression(), Compression::Zstd);
    }

    #[test]
    fn parse_cli_compression_accepts_case_insensitive() {
        assert_eq!(parse_cli_compression("none").unwrap(), Compression::None);
        assert_eq!(parse_cli_compression("None").unwrap(), Compression::None);
        assert_eq!(parse_cli_compression("NONE").unwrap(), Compression::None);
        assert_eq!(parse_cli_compression("zstd").unwrap(), Compression::Zstd);
        assert_eq!(parse_cli_compression("ZSTD").unwrap(), Compression::Zstd);
        assert_eq!(parse_cli_compression(" Zstd ").unwrap(), Compression::Zstd);
    }

    #[test]
    fn parse_cli_compression_rejects_unknown() {
        let err = parse_cli_compression("gzip").unwrap_err();
        assert!(err.contains("unknown compression"), "{err}");
        assert!(err.contains("none|zstd"), "{err}");
    }
}
