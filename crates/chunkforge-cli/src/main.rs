//! ChunkForge CLI: make / archive / extract / cat / verify / mount / doctor / gc / push / pull / diff / store (+ chunk-id debug).

mod parallel;
mod progress;

use anyhow::{Context, Result, bail};
use chunkforge_chunk::{ChunkId, ChunkInfo, ChunkParams, chunk_bytes};
use chunkforge_index::{
    DIR_FORMAT_VERSION_V1, DIR_MAGIC_PREFIX, DiffReport, DirArchive, DirEntry, DirEntryKind,
    FLAG_CHUNKS_COMPRESSED_IN_STORE, Index, IndexEntry, MAGIC_PREFIX, PathFilter, SeedDecision,
    UnchangedVerdict, decide_seed_for_entry_ex, diff_dir_archives, entry_length, hash_reader,
    judge_extract_unchanged_opts, load_exclude_file, seed_file_map, validate_archive_path,
};
use chunkforge_remote::{
    FileUrlSource, HttpChunkSink, HttpChunkSource, RetryPolicy, SigV4Config, SigV4Signer,
    SummaryFailureBucket, classify_sink_error, classify_source_error,
};
use chunkforge_store::{
    CacheSource, ChunkSink, ChunkSource, Compression, Error as StoreError, PutOutcome, Store,
};
use clap::{Parser, Subcommand, ValueEnum};
use progress::ProgressReporter;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(
    name = "chunkforge",
    version,
    about = "Content-defined chunking + BLAKE3 CAS (make / archive / extract / cat / verify / mount / doctor / gc / push / pull / diff / store)",
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
    /// format-independent.
    Make {
        /// Local CAS store directory (created if missing)
        #[arg(long)]
        store: PathBuf,
        /// Output .cfidx path
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
    },
    /// Archive a directory tree into a local store + `.cfdir` listing
    ///
    /// Recurses regular files only (FastCDC + BLAKE3 per file). Chunks are
    /// written into `--store` with content-addressed dedup; the output `.cfdir`
    /// records relative paths and per-file chunk tables. Symlinks, fifos,
    /// sockets, and device nodes are **skipped with a stderr warning** (P0
    /// policy: do not follow / do not record) **before** `--path`/`--exclude`
    /// filtering. Empty directories are omitted (extract can recreate parents
    /// from file paths). Optional repeatable `--path` / `--exclude` / `--exclude-from` restrict
    /// which regular files are chunked and listed (default: full tree ≡ 1.2.0).
    /// Default **`--format text`** (≡ 1.2.0): summary on stderr.
    /// **`--format json`**: one JSON object on stdout; no duplicate text
    /// summary; exit codes are format-independent. `make` single-file
    /// semantics are unchanged.
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
        /// Output format: `text` (default ≡ 1.2.0 stderr summary) or `json`
        /// (one object on stdout; no duplicate stderr summary)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
    },
    /// Materialize a directory tree from a `.cfdir` + chunk source
    ///
    /// Reads the **full** `.cfdir` listing and reconstitutes matching regular
    /// files under `-o` from `--store` / `--source` (same origin flags as
    /// `cat` / `verify`). Parent directories are created as needed for written
    /// files. Optional repeatable `--path` / `--exclude` / `--exclude-from` restrict which
    /// listing entries are materialized (default: full tree ≡ 1.2.0). Path
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
    /// would_fail for the **filtered** set and exit is 0 unless the listing
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
        /// Optional local cache store (filled on miss; never writes primary)
        #[arg(long, value_name = "DIR")]
        cache: Option<PathBuf>,
        /// Soft fill budget for `--cache` in bytes (pure integer; no KiB suffix).
        /// Requires `--cache`. Omit ≡ 1.4 unbounded fill. Over budget skips
        /// fill (still serves primary); never evicts / LRU.
        #[arg(long = "cache-max-bytes", value_name = "N")]
        cache_max_bytes: Option<u64>,
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
        /// `extract: dry-run: would_skip=… would_write=… would_dirs=… would_fail=…`.
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
    },
    /// Reassemble a blob from a .cfidx + chunk source
    #[command(group(clap::ArgGroup::new("origin").required(true).args(["store", "source"])))]
    Cat {
        /// Local CAS store (Phase 1 compat; synonym for `--source <path>`)
        #[arg(long)]
        store: Option<PathBuf>,
        /// Chunk source: local path, `file://`, or `http(s)://`
        #[arg(long, value_name = "PATH|URL")]
        source: Option<String>,
        /// Optional local cache store (filled on miss; never writes primary)
        #[arg(long, value_name = "DIR")]
        cache: Option<PathBuf>,
        /// Soft fill budget for `--cache` in bytes (pure integer; no KiB suffix).
        /// Requires `--cache`. Omit ≡ 1.4 unbounded fill. Over budget skips
        /// fill (still serves primary); never evicts / LRU.
        #[arg(long = "cache-max-bytes", value_name = "N")]
        cache_max_bytes: Option<u64>,
        #[command(flatten)]
        http_tmpl: HttpTemplateArgs,
        /// Max concurrent chunk fetches (default 1 = serial / 0.3.0 behaviour)
        #[arg(long, default_value_t = 1, value_name = "N")]
        jobs: u32,
        /// Input .cfidx
        index: PathBuf,
        /// Output file path
        #[arg(short = 'o', long = "output")]
        output: PathBuf,
    },
    /// Verify `.cfidx` / `.cfdir` integrity, chunk presence/hashes, and blob_blake3
    ///
    /// Magic-dispatches: `.cfidx` → single-blob verify (unchanged); `.cfdir` →
    /// tree verify (structure + per-file `blob_blake3` + missing chunks fail with id).
    /// Default **`--format text`** (≡ 0.9.0): summary on stderr. **`--format json`**:
    /// one JSON object on stdout (`ok` / `kind` / size fields); exit code is
    /// format-independent (ok → 0, failure → non-zero).
    #[command(group(clap::ArgGroup::new("origin").required(true).args(["store", "source"])))]
    Verify {
        /// Local CAS store (Phase 1 compat; synonym for `--source <path>`)
        #[arg(long)]
        store: Option<PathBuf>,
        /// Chunk source: local path, `file://`, or `http(s)://`
        #[arg(long, value_name = "PATH|URL")]
        source: Option<String>,
        /// Optional local cache store (filled on miss; never writes primary)
        #[arg(long, value_name = "DIR")]
        cache: Option<PathBuf>,
        /// Soft fill budget for `--cache` in bytes (pure integer; no KiB suffix).
        /// Requires `--cache`. Omit ≡ 1.4 unbounded fill. Over budget skips
        /// fill (still serves primary); never evicts / LRU.
        #[arg(long = "cache-max-bytes", value_name = "N")]
        cache_max_bytes: Option<u64>,
        #[command(flatten)]
        http_tmpl: HttpTemplateArgs,
        /// Max concurrent chunk fetches (default 1 = serial / 0.3.0 behaviour)
        #[arg(long, default_value_t = 1, value_name = "N")]
        jobs: u32,
        /// Output format: `text` (default ≡ 0.9.0 stderr summary) or `json`
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
        /// Input `.cfidx` or `.cfdir`
        index: PathBuf,
    },
    /// Debug: chunk + hash only; print offset/len/id (no store write)
    #[command(name = "chunk-id")]
    ChunkId {
        /// Input file
        input: PathBuf,
        /// Override FastCDC sizes as min:avg:max (bytes; all even, min≤avg≤max)
        #[arg(long = "chunk-size", value_name = "MIN:AVG:MAX")]
        chunk_size: Option<String>,
    },
    /// Mount a `.cfidx` (single file) or `.cfdir` (directory tree) read-only (Linux + fuse3)
    #[command(group(clap::ArgGroup::new("origin").required(true).args(["store", "source"])))]
    Mount {
        /// Local CAS store (Phase 1 compat; synonym for `--source <path>`)
        #[arg(long)]
        store: Option<PathBuf>,
        /// Chunk source: local path, `file://`, or `http(s)://`
        #[arg(long, value_name = "PATH|URL")]
        source: Option<String>,
        /// Optional local cache store (filled on miss; never writes primary)
        #[arg(long, value_name = "DIR")]
        cache: Option<PathBuf>,
        /// Soft fill budget for `--cache` in bytes (pure integer; no KiB suffix).
        /// Requires `--cache`. Omit ≡ 1.4 unbounded fill. Over budget skips
        /// fill (still serves primary); never evicts / LRU.
        #[arg(long = "cache-max-bytes", value_name = "N")]
        cache_max_bytes: Option<u64>,
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
        /// Input `.cfidx` or `.cfdir`
        index: PathBuf,
        /// Empty directory to mount onto
        mountpoint: PathBuf,
    },
    /// Check indexes and chunk presence (missing ids → non-zero exit)
    ///
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
    /// Upload missing chunks referenced by `.cfidx` / `.cfdir` listings to an HTTP(S) destination
    ///
    /// Reads plaintext chunks from the local `--store`, probes the remote with
    /// `has`, and PUTs only missing ids. Does **not** upload `.cfidx` / `.cfdir`
    /// listing files themselves (chunks only).
    /// Template flags (`--url-template` / `--prefix` / `--header`) match read-side
    /// layout so a successful push is readable with `verify --source`.
    /// `--http-retries N` (default 0 ≡ 0.7.0) retries transient HTTP failures
    /// (extra attempts after the first try); see also `--http-retry-backoff-ms`.
    /// Optional repeatable `--path` / `--exclude` / `--exclude-from` restrict which
    /// **File** entries in a `.cfdir` contribute chunk ids (Dir entries never
    /// contribute; listing itself is **not** uploaded). Omit all path/exclude
    /// flags ⇒ full reference set (≡ 1.3.0). `.cfidx` + any path/exclude flag
    /// (including `--exclude-from`) → clear non-zero error.
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
        /// HTTP(S) destination base URL (same layout as `--source` for verify/cat)
        #[arg(long, value_name = "URL")]
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
    /// Optional repeatable `--path` / `--exclude` / `--exclude-from` restrict which
    /// **File** entries in a `.cfdir` contribute chunk ids (Dir entries never
    /// contribute; listing file unchanged / not downloaded). Omit all path/exclude
    /// flags ⇒ full reference set (≡ 1.2.0). Orthogonal to `--dry-run` / `--format` /
    /// `--jobs` /
    /// `--progress` / retries / SigV4. `--source` accepts a local path,
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
        #[command(flatten)]
        http_tmpl: HttpTemplateArgs,
        /// Max concurrent has/get/put workers (default 1 = serial)
        #[arg(long, default_value_t = 1, value_name = "N")]
        jobs: u32,
        /// Probe and count only; do not write the local store
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// Output format: `text` (default ≡ 1.0.0 stderr summary) or `json`
        /// (one object on stdout; no duplicate stderr summary)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
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
        /// One or more `.cfidx` / `.cfdir` listings whose chunk ids are fetched
        #[arg(required = true, num_args = 1..)]
        indexes: Vec<PathBuf>,
    },
    /// Compare `.cfdir` listings, or a source tree against a listing (`--tree`)
    ///
    /// **Listing↔listing:** two `.cfdir` args (`.cfidx` / bad magic → error).
    /// **Tree↔listing:** `--tree <src-dir> <listing.cfdir>` — builds an ephemeral
    /// in-memory `DirArchive` from regular files in `src-dir` (left) and compares
    /// it to the listing (right). Read-only: does **not** write store or `.cfdir`.
    ///
    /// Reports path-level **added** / **removed** / **changed** (content) /
    /// **meta_changed** (same blake3, mode/mtime differ). Default **`--format
    /// text`** (≡ 0.7.0): path lists (non-empty categories) plus a stable
    /// summary line on **stdout**:
    /// `diff: added=… removed=… changed=… meta_changed=… chunks_shared=… chunks_only_left=… chunks_only_right=…`
    /// **`--format json`**: one JSON object with the same path arrays and chunk
    /// stats fields (full arrays; `--max-paths` applies to text listings only).
    /// Exit codes are format-independent: **0** when identical, **1** when any
    /// path or chunk-set difference; usage / decode errors use the usual
    /// non-zero clap/anyhow path. See `docs/diff.md`.
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
        /// Left `.cfdir` (listing↔listing), or the listing `.cfdir` when `--tree` is set
        left: PathBuf,
        /// Right `.cfdir` (listing↔listing only). Must be omitted with `--tree`.
        right: Option<PathBuf>,
    },
    /// Query the local store
    Store {
        #[command(subcommand)]
        command: StoreCommands,
    },
}

#[derive(Debug, Subcommand)]
enum StoreCommands {
    /// Check whether a chunk id exists in the store
    Has {
        /// Local CAS store directory
        #[arg(long)]
        store: PathBuf,
        /// Chunk id as 64 lowercase hex characters
        hex_id: String,
    },
    /// Rehash every loose chunk in a local store (bitrot / integrity scrub)
    Scrub {
        /// Local CAS store directory
        #[arg(long)]
        store: PathBuf,
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
    /// Read-only observation via `Store::stats` — never deletes, trims, or
    /// applies LRU. Default **`--format text`**: one stdout summary line
    /// `store stats: chunks=N bytes_on_disk=M compression=none|zstd`.
    /// **`--format json`**: one JSON object on stdout (`ok` / `chunks` /
    /// `bytes_on_disk` / `compression`); no text dual-write. Exit codes are
    /// format-independent (success → 0).
    #[command(visible_alias = "du")]
    Stats {
        /// Local CAS store directory
        #[arg(long)]
        store: PathBuf,
        /// Output format (default text; json = one object on stdout)
        #[arg(long = "format", value_enum, default_value_t = CliFormat::Text)]
        format: CliFormat,
    },
}

/// Shared `--format text|json` for `diff` / `verify` / `doctor` / `extract` /
/// `push` / `pull` / `gc` / `store scrub` / `store stats` / `archive` / `make`
/// (Phase 8–12 + Phase 13 M2 + Phase 14 M2 + Phase 15 M3).
/// Default `text` preserves prior behaviour (`diff` ≡ 0.7.0; `verify`/`doctor` ≡ 0.9.0;
/// `extract` / `push` / `pull` ≡ 1.0.0; `gc` / `store scrub` ≡ 1.1.0; `archive` ≡ 1.2.0;
/// `store stats` ≡ text summary; `make` ≡ 1.4.0 stderr summary).
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
/// are only meaningful with an `http(s)://` `--source` / `--dest`. Omitting them
/// preserves the Phase 2 default layout (`{base}/chunks/<2hex>/<62hex>.cnk`).
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
        } => cmd_make(&store, &output, &input, chunk_size.as_deref(), format),
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
            format,
        } => {
            let jobs = parse_jobs(jobs)?;
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
            )
        }
        Commands::Extract {
            store,
            source,
            cache,
            cache_max_bytes,
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
        } => {
            let jobs = parse_jobs(jobs)?;
            let excludes = merged_excludes(&excludes, &exclude_from)?;
            let path_filter = PathFilter::new(paths.iter().cloned(), excludes.iter().cloned())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            // Dry-run never opens store/source (no chunk get; G2 / §3.2).
            if dry_run {
                cmd_extract(
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
                )
            } else {
                let src = open_chunk_source(
                    store.as_deref(),
                    source.as_deref(),
                    cache.as_deref(),
                    cache_max_bytes,
                    &http_tmpl,
                )?;
                cmd_extract(
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
                )
            }
        }
        Commands::Cat {
            store,
            source,
            cache,
            cache_max_bytes,
            http_tmpl,
            jobs,
            index,
            output,
        } => {
            let jobs = parse_jobs(jobs)?;
            let src = open_chunk_source(
                store.as_deref(),
                source.as_deref(),
                cache.as_deref(),
                cache_max_bytes,
                &http_tmpl,
            )?;
            cmd_cat(src.as_ref(), &index, &output, jobs)
        }
        Commands::Verify {
            store,
            source,
            cache,
            cache_max_bytes,
            http_tmpl,
            jobs,
            format,
            index,
        } => {
            let jobs = parse_jobs(jobs)?;
            let src = open_chunk_source(
                store.as_deref(),
                source.as_deref(),
                cache.as_deref(),
                cache_max_bytes,
                &http_tmpl,
            )?;
            cmd_verify(src.as_ref(), &index, jobs, format)
        }
        Commands::ChunkId { input, chunk_size } => cmd_chunk_id(&input, chunk_size.as_deref()),
        Commands::Mount {
            store,
            source,
            cache,
            cache_max_bytes,
            http_tmpl,
            name,
            no_prefetch,
            prefetch_chunks,
            index,
            mountpoint,
        } => {
            ensure_mount_supported()?;
            let src = open_chunk_source(
                store.as_deref(),
                source.as_deref(),
                cache.as_deref(),
                cache_max_bytes,
                &http_tmpl,
            )?;
            cmd_mount(
                src,
                &index,
                &mountpoint,
                name.as_deref(),
                !no_prefetch,
                prefetch_chunks as usize,
            )
        }
        Commands::Doctor {
            store,
            source,
            http_tmpl,
            jobs,
            deep,
            no_probe,
            format,
            indexes,
        } => {
            let jobs = parse_jobs(jobs)?;
            let src = open_chunk_source(store.as_deref(), source.as_deref(), None, None, &http_tmpl)?;
            let origin_spec = match (store.as_deref(), source.as_deref()) {
                (Some(path), None) => path.to_string_lossy().into_owned(),
                (None, Some(s)) => s.to_string(),
                _ => unreachable!("clap origin group requires exactly one of --store/--source"),
            };
            cmd_doctor(
                src.as_ref(),
                &origin_spec,
                &indexes,
                deep,
                no_probe,
                jobs,
                http_tmpl.http_retries,
                format,
            )
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
            indexes,
        } => {
            let jobs = parse_jobs(jobs)?;
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
            )
        }
        Commands::Pull {
            store,
            source,
            http_tmpl,
            jobs,
            dry_run,
            format,
            progress,
            paths,
            excludes,
            exclude_from,
            indexes,
        } => {
            let jobs = parse_jobs(jobs)?;
            let excludes = merged_excludes(&excludes, &exclude_from)?;
            let path_filter = PathFilter::new(paths.iter().cloned(), excludes.iter().cloned())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            cmd_pull(
                &store,
                &source,
                &http_tmpl,
                dry_run,
                &indexes,
                jobs,
                format,
                progress,
                &path_filter,
            )
        }
        Commands::Diff {
            format,
            max_paths,
            tree,
            left,
            right,
        } => cmd_diff_dispatch(tree.as_deref(), &left, right.as_deref(), max_paths, format),
        Commands::Store {
            command: StoreCommands::Has { store, hex_id },
        } => cmd_store_has(&store, &hex_id),
        Commands::Store {
            command:
                StoreCommands::Scrub {
                    store,
                    jobs,
                    format,
                    progress,
                },
        } => {
            let jobs = parse_jobs(jobs)?;
            cmd_store_scrub(&store, jobs, format, progress)
        }
        Commands::Store {
            command: StoreCommands::Stats { store, format },
        } => cmd_store_stats(&store, format),
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

fn open_or_create_store(root: &Path) -> Result<Store> {
    let meta = root.join("meta.toml");
    if meta.is_file() {
        Store::open(root).with_context(|| format!("open store at {}", root.display()))
    } else {
        Store::create(root, Compression::None)
            .with_context(|| format!("create store at {}", root.display()))
    }
}

/// Resolve `--store` / `--source` / `--cache` (+ optional HTTP templates) into a boxed [`ChunkSource`].
///
/// `--store PATH` is a Phase 1 synonym for `--source PATH` (local only).
/// With `--cache`, reads go through [`CacheSource`] (fill on miss; never write primary).
/// `--cache-max-bytes N` sets a soft fill budget (requires `--cache`); omit ≡ 1.4 unbounded.
/// `--url-template` / `--prefix` / `--header` apply only to `http(s)://` sources.
fn open_chunk_source(
    store: Option<&Path>,
    source: Option<&str>,
    cache: Option<&Path>,
    cache_max_bytes: Option<u64>,
    http_tmpl: &HttpTemplateArgs,
) -> Result<Box<dyn ChunkSource>> {
    if cache_max_bytes.is_some() && cache.is_none() {
        bail!("--cache-max-bytes requires --cache <DIR>");
    }

    let spec = match (store, source) {
        (Some(path), None) => path.to_string_lossy().into_owned(),
        (None, Some(s)) => s.to_string(),
        (Some(_), Some(_)) => bail!("use either --store or --source, not both"),
        (None, None) => bail!("missing chunk origin: pass --store <path> or --source <PATH|URL>"),
    };

    let primary = open_primary_source(&spec, http_tmpl)?;
    match cache {
        None => Ok(primary),
        Some(cache_path) => {
            let cache_store = open_or_create_store(cache_path)?;
            // `None` ≡ CacheSource::new (1.4 unbounded); Some(N) soft-refuses fill.
            Ok(Box::new(CacheSource::with_max_bytes(
                primary,
                cache_store,
                cache_max_bytes,
            )))
        }
    }
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
    let is_http = trimmed.starts_with("http://") || trimmed.starts_with("https://");

    if !is_http && http_template_flags_set(http_tmpl) {
        bail!(
            "--url-template / --prefix / --header / --aws-sigv4 apply only to http(s):// sources;              got non-HTTP source {trimmed:?}"
        );
    }

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

fn cmd_make(
    store_path: &Path,
    output: &Path,
    input: &Path,
    chunk_size: Option<&str>,
    format: CliFormat,
) -> Result<()> {
    let params = parse_chunk_size(chunk_size)?;
    let data = fs::read(input).with_context(|| format!("read input {}", input.display()))?;
    let store = open_or_create_store(store_path)?;

    let chunks = chunk_bytes(&data, &params);
    let blob_blake3 = ChunkId::hash(&data);

    let mut entries = Vec::with_capacity(chunks.len());
    let mut new_chunks = 0usize;
    let mut reused_chunks = 0usize;
    for c in &chunks {
        let start = c.offset as usize;
        let end = (c.offset + c.length) as usize;
        let slice = data
            .get(start..end)
            .with_context(|| format!("chunk range {start}..{end} out of bounds"))?;
        let outcome = store
            .put_with_id(&c.id, slice)
            .with_context(|| format!("put chunk {}", c.id))?;
        match outcome {
            PutOutcome::Written => new_chunks += 1,
            PutOutcome::SkippedExists => reused_chunks += 1,
        }
        entries.push(IndexEntry {
            end_offset: c.offset + c.length,
            chunk_id: c.id,
        });
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

    match format {
        CliFormat::Text => {
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
        CliFormat::Json => {
            let obj = serde_json::json!({
                "ok": true,
                "bytes": data.len() as u64,
                "chunks": chunks.len(),
                "new": new_chunks,
                "reused": reused_chunks,
            });
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
        Some(open_or_create_store(store_path)?)
    };

    let mut flags = 0u16;
    if let Some(ref store) = store {
        if !matches!(store.compression(), Compression::None) {
            flags |= FLAG_CHUNKS_COMPRESSED_IN_STORE;
        }
    }

    // Collect regular files first (sorted) so .cfdir output is deterministic.
    // Order: type skip (symlink/special) inside walk, then PathFilter on candidates.
    let mut file_paths: Vec<PathBuf> = Vec::new();
    let mut skipped_symlinks = 0usize;
    let mut skipped_special = 0usize;
    collect_archive_files(
        src_dir,
        src_dir,
        &mut file_paths,
        &mut skipped_symlinks,
        &mut skipped_special,
    )?;
    file_paths.sort();

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

    if format == CliFormat::Text && (skipped_symlinks > 0 || skipped_special > 0) {
        eprintln!(
            "archive: symlink policy = skip+warn (not recorded / not followed); \
             skipped {skipped_symlinks} symlink{}, {skipped_special} special (fifo/socket/device)",
            if skipped_symlinks == 1 { "" } else { "s" },
        );
    }

    // Dry-run cross-file dedup set (Mutex so --jobs > 1 stays correct).
    let dry_seen: Mutex<HashSet<ChunkId>> = Mutex::new(HashSet::new());

    let seed_ctx = seed_map.as_ref().map(|m| (m, seed_trust_mtime));
    let outcomes = parallel::map_indexed(&file_paths, jobs, |_idx, full| {
        archive_one_file(
            src_dir,
            full,
            &params,
            store.as_ref(),
            seed_ctx,
            dry_run,
            &dry_seen,
        )
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

    let file_count = entries.len();
    // Empty Dir entries are omitted (≡ 1.2.0); dirs counter stays 0 for File-only listings.
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
                    DirEntryKind::Dir { .. } => unreachable!("seed map is files only"),
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

/// Recursively collect regular-file paths under `dir` (relative walk from `root`).
///
/// Symlinks (including symlink-to-dir) and special files are skipped with a
/// per-path stderr warning **before** any `--path`/`--exclude` filter (Phase 13
/// M2 order: type skip → PathFilter on candidates). Does **not** follow
/// directory symlinks (avoids loops).
fn collect_archive_files(
    root: &Path,
    dir: &Path,
    out: &mut Vec<PathBuf>,
    skipped_symlinks: &mut usize,
    skipped_special: &mut usize,
) -> Result<()> {
    let entries = fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("read_dir entry under {}", dir.display()))?;
        let path = entry.path();
        let ft = entry
            .file_type()
            .with_context(|| format!("file_type {}", path.display()))?;

        if ft.is_symlink() {
            *skipped_symlinks += 1;
            eprintln!(
                "archive: skip symlink {} (policy: skip+warn; not recorded / not followed)",
                display_under_root(root, &path)
            );
            continue;
        }
        if ft.is_dir() {
            collect_archive_files(root, &path, out, skipped_symlinks, skipped_special)?;
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

fn cmd_diff_dispatch(
    tree: Option<&Path>,
    left: &Path,
    right: Option<&Path>,
    max_paths: Option<usize>,
    format: CliFormat,
) -> Result<()> {
    match tree {
        Some(src_dir) => {
            if let Some(extra) = right {
                bail!(
                    "diff --tree takes one listing `.cfdir` after the source dir;                      unexpected extra argument {}",
                    extra.display()
                );
            }
            cmd_diff_tree(src_dir, left, max_paths, format)
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
            cmd_diff_listings(left, right, max_paths, format)
        }
    }
}

fn cmd_diff_listings(
    left: &Path,
    right: &Path,
    max_paths: Option<usize>,
    format: CliFormat,
) -> Result<()> {
    let left_arch = load_cfdir_for_diff(left)?;
    let right_arch = load_cfdir_for_diff(right)?;
    let report = diff_dir_archives(&left_arch, &right_arch);
    emit_diff_report(&report, max_paths, format)
}

/// Tree↔listing: left = ephemeral DirArchive from `src_dir`, right = listing.
fn cmd_diff_tree(
    src_dir: &Path,
    listing: &Path,
    max_paths: Option<usize>,
    format: CliFormat,
) -> Result<()> {
    if !src_dir.is_dir() {
        bail!(
            "diff --tree source {} is not a directory (or is unreadable)",
            src_dir.display()
        );
    }
    if listing.is_dir() {
        bail!(
            "diff --tree expects one listing `.cfdir` (got directory {});              two trees are not supported",
            listing.display()
        );
    }
    let listing_arch = load_cfdir_for_diff(listing)?;
    let tree_arch = build_ephemeral_tree_archive(src_dir, &listing_arch)?;
    // Documented orientation: left=tree, right=listing.
    let report = diff_dir_archives(&tree_arch, &listing_arch);
    emit_diff_report(&report, max_paths, format)
}

/// Build an in-memory `DirArchive` from a source tree for `diff --tree`.
///
/// Regular files only (symlink/special skipped + warn, same policy as `archive`).
/// Does **not** write store or `.cfdir`. For each file: size, mode, mtime_secs,
/// stream BLAKE3 → `blob_blake3`. When the path exists in `listing` **and**
/// `blob_blake3` matches, copy the listing's File entry (chunk table + meta) so
/// identical tree↔listing yields `chunks_shared`≈full and `chunks_only_*=0`.
/// Otherwise use an empty chunk list (path-level diff still works). Empty
/// non-zero-size chunk tables are **not** structurally valid for encode, so this
/// archive is ephemeral and never written.
fn build_ephemeral_tree_archive(src_dir: &Path, listing: &DirArchive) -> Result<DirArchive> {
    let listing_files = seed_file_map(listing);

    let mut file_paths: Vec<PathBuf> = Vec::new();
    let mut skipped_symlinks = 0usize;
    let mut skipped_special = 0usize;
    collect_diff_tree_files(
        src_dir,
        src_dir,
        &mut file_paths,
        &mut skipped_symlinks,
        &mut skipped_special,
    )?;
    file_paths.sort();

    if skipped_symlinks > 0 || skipped_special > 0 {
        eprintln!(
            "diff: symlink policy = skip+warn (not recorded / not followed);              skipped {skipped_symlinks} symlink{}, {skipped_special} special (fifo/socket/device)",
            if skipped_symlinks == 1 { "" } else { "s" },
        );
    }

    let mut entries: Vec<DirEntry> = Vec::with_capacity(file_paths.len());
    for full in &file_paths {
        let rel = relative_archive_path(src_dir, full)?;
        validate_archive_path(&rel)
            .map_err(|e| anyhow::anyhow!("invalid archive path {rel:?}: {e}"))?;

        let meta =
            fs::symlink_metadata(full).with_context(|| format!("stat {}", full.display()))?;
        if meta.file_type().is_symlink() {
            // collect should have skipped; belt-and-suspenders.
            eprintln!(
                "diff: skip symlink {} (policy: skip+warn; not recorded)",
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

    // Ephemeral only — may contain empty chunk tables on non-zero size (not
    // encode-valid). Do not call DirArchive::new / encode / write.
    Ok(DirArchive {
        format_version: DIR_FORMAT_VERSION_V1,
        flags: 0,
        entries,
    })
}

/// Walk like [`collect_archive_files`], but warn with a `diff:` prefix.
fn collect_diff_tree_files(
    root: &Path,
    dir: &Path,
    out: &mut Vec<PathBuf>,
    skipped_symlinks: &mut usize,
    skipped_special: &mut usize,
) -> Result<()> {
    let entries = fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("read_dir entry under {}", dir.display()))?;
        let path = entry.path();
        let ft = entry
            .file_type()
            .with_context(|| format!("file_type {}", path.display()))?;

        if ft.is_symlink() {
            *skipped_symlinks += 1;
            eprintln!(
                "diff: skip symlink {} (policy: skip+warn; not recorded / not followed)",
                display_under_root(root, &path)
            );
            continue;
        }
        if ft.is_dir() {
            collect_diff_tree_files(root, &path, out, skipped_symlinks, skipped_special)?;
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
/// set (≡ 1.3.0 / 1.2.0). `--path`/`--exclude` on a `.cfidx` is an error (no File paths).
fn listing_chunk_ids_filtered(path: &Path, filter: &PathFilter) -> Result<Vec<ChunkId>> {
    let filter_active = !filter.paths().is_empty() || !filter.excludes().is_empty();
    match peek_listing_kind(path)? {
        ListingKind::Index => {
            if filter_active {
                bail!(
                    "--path/--exclude applies to `.cfdir` File entries; {} looks like a `.cfidx` (use without path flags for full single-blob reference set)",
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
                    DirEntryKind::Dir { .. } => {}
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

fn cmd_cat(source: &dyn ChunkSource, index_path: &Path, output: &Path, jobs: usize) -> Result<()> {
    let index = load_index(index_path)?;

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
    let plains = fetch_entry_plains(source, &index.entries, jobs, "cat")?;
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

    // Empty file: still created above; total_size must be 0.
    if index.total_size == 0 && !index.entries.is_empty() {
        bail!("invalid index: total_size 0 with non-empty entries");
    }
    Ok(())
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
        }
        return Ok(plains);
    }

    let results = parallel::map_indexed(&tasks, jobs, |_idx, (i, chunk_id, expected_len)| {
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
) -> Result<()> {
    match peek_listing_kind(listing_path)? {
        ListingKind::Index => cmd_verify_index(source, listing_path, jobs, format),
        ListingKind::DirArchive => cmd_verify_dir(source, listing_path, jobs, format),
    }
}

fn cmd_verify_index(
    source: &dyn ChunkSource,
    index_path: &Path,
    jobs: usize,
    format: CliFormat,
) -> Result<()> {
    let index = load_index(index_path)?;
    index
        .validate()
        .map_err(|e| anyhow::anyhow!("index structure: {e}"))?;

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
        }
        plains
    } else {
        // Concurrent get (presence implied); length-checked; errors include chunk id.
        fetch_entry_plains(source, &index.entries, jobs, "verify failed")?
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
            println!(
                "{{\"ok\":true,\"kind\":\"cfidx\",\"bytes\":{},\"chunks\":{}}}",
                index.total_size,
                index.chunk_count()
            );
        }
    }
    Ok(())
}

fn cmd_verify_dir(
    source: &dyn ChunkSource,
    archive_path: &Path,
    jobs: usize,
    format: CliFormat,
) -> Result<()> {
    let archive = load_dir_archive(archive_path)?;
    archive
        .validate()
        .map_err(|e| anyhow::anyhow!("archive structure: {e}"))?;

    let mut file_count = 0usize;
    let mut total_chunks = 0usize;

    for entry in &archive.entries {
        match &entry.kind {
            DirEntryKind::Dir { .. } => {
                // Structure already validated; nothing to fetch.
            }
            DirEntryKind::File {
                size,
                blob_blake3,
                chunks,
                ..
            } => {
                file_count += 1;
                total_chunks += chunks.len();
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
                    }
                    plains
                } else {
                    fetch_entry_plains(source, chunks, jobs, &op)?
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
            eprintln!(
                "verify: ok ({} file{}, {} chunk{})",
                file_count,
                if file_count == 1 { "" } else { "s" },
                total_chunks,
                if total_chunks == 1 { "" } else { "s" }
            );
        }
        CliFormat::Json => {
            println!(
                "{{\"ok\":true,\"kind\":\"cfdir\",\"files\":{},\"chunks\":{}}}",
                file_count, total_chunks
            );
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
    let mut skipped_count = 0usize;

    // Phase 13 M3: read full listing; only materialize matching File + Dir
    // entries (parents of written files via create_dir_all). Unmatched paths
    // are skipped (not written, never deleted — non-prune).
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
                let plains = fetch_entry_plains(source, chunks, jobs, &op)?;

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
            }
        }
    }

    match format {
        CliFormat::Text => {
            if skip_unchanged {
                // G3 / M2: field names skipped= / wrote= / dirs= (nailed).
                eprintln!(
                    "extract: {} skipped={skipped_count} wrote={file_count} dirs={dir_count}",
                    out_dir.display(),
                );
            } else {
                // ≡ 0.8.0 / 1.0.0 summary line when --skip-unchanged is off.
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
            let obj = serde_json::json!({
                "ok": true,
                "dry_run": false,
                "skipped": skipped_count,
                "wrote": file_count,
                "dirs": dir_count,
            });
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
fn cmd_extract_dry_run(
    archive: &DirArchive,
    out_dir: &Path,
    force: bool,
    skip_unchanged: bool,
    skip_trust_mtime: bool,
    format: CliFormat,
    path_filter: &PathFilter,
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
            }
        }
    }

    // G3 / M3: nailed field names (would_fail included — dry-run conflict path).
    match format {
        CliFormat::Text => {
            eprintln!(
                "extract: dry-run: would_skip={would_skip} would_write={would_write} would_dirs={would_dirs} would_fail={would_fail}"
            );
        }
        CliFormat::Json => {
            let obj = serde_json::json!({
                "ok": true,
                "dry_run": true,
                "would_skip": would_skip,
                "would_write": would_write,
                "would_dirs": would_dirs,
                "would_fail": would_fail,
            });
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
    let mut checks: Vec<(String, ChunkId)> = Vec::new();
    let mut listings_ok = 0usize;
    for listing_path in index_paths {
        let display = listing_path.display().to_string();
        for id in listing_chunk_ids(listing_path)? {
            checks.push((display.clone(), id));
        }
        listings_ok += 1;
    }
    let checked = checks.len();

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
        }
    } else {
        let outcomes = parallel::map_indexed(&checks, jobs, |_i, (listing_display, chunk_id)| {
            if deep {
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
            }
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
                let obj = serde_json::json!({
                    "ok": true,
                    "listings": listings_ok,
                    "checked": checked,
                    "missing": 0,
                    "deep": deep,
                    "retries": http_retries,
                });
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
                let obj = serde_json::json!({
                    "ok": false,
                    "listings": listings_ok,
                    "checked": checked,
                    "missing": missing_ids,
                    "deep": deep,
                    "retries": http_retries,
                });
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

fn open_http_chunk_sink(dest: &str, http_tmpl: &HttpTemplateArgs) -> Result<HttpChunkSink> {
    let trimmed = dest.trim();
    let is_http = trimmed.starts_with("http://") || trimmed.starts_with("https://");

    if !is_http {
        if http_template_flags_set(http_tmpl) {
            bail!(
                "--url-template / --prefix / --header apply only to http(s):// destinations; \
                 got non-HTTP --dest {trimmed:?}"
            );
        }
        bail!(
            "push --dest must be an http(s):// URL (got {trimmed:?}); \
             local/file destinations are not supported"
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
    builder.build().context("build HTTP chunk sink")
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
) -> Result<()> {
    let store = Store::open(store_path)
        .with_context(|| format!("open store at {}", store_path.display()))?;
    let sink = open_http_chunk_sink(dest, http_tmpl)?;

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
            .context("build HTTP chunk source from --dest for push --verify")?;
        for path in index_paths {
            cmd_verify(source.as_ref(), path, jobs, CliFormat::Text).with_context(|| {
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
    http_tmpl: &HttpTemplateArgs,
    dry_run: bool,
    index_paths: &[PathBuf],
    jobs: usize,
    format: CliFormat,
    progress: bool,
    path_filter: &PathFilter,
) -> Result<()> {
    let source = open_primary_source(source_spec, http_tmpl)
        .with_context(|| format!("open chunk source {source_spec:?}"))?;

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
        Some(open_or_create_store(store_path)?)
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
            let obj = serde_json::json!({
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
            println!("{obj}");
        }
    }

    if failed > 0 {
        // Exit code is independent of `--format` (JSON already emitted above).
        bail!(
            "pull: {failed} failure{}{}",
            if failed == 1 { "" } else { "s" },
            first_error
                .map(|m| format!(" (first: {m})"))
                .unwrap_or_default()
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

fn cmd_chunk_id(input: &Path, chunk_size: Option<&str>) -> Result<()> {
    let params = parse_chunk_size(chunk_size)?;
    let data = fs::read(input).with_context(|| format!("read input {}", input.display()))?;
    let chunks: Vec<ChunkInfo> = chunk_bytes(&data, &params);
    for c in &chunks {
        println!("{}\t{}\t{}", c.offset, c.length, c.id);
    }
    Ok(())
}

fn cmd_store_has(store_path: &Path, hex_id: &str) -> Result<()> {
    let id = ChunkId::from_hex(hex_id).map_err(|e| anyhow::anyhow!("{e}"))?;
    let store = Store::open(store_path)
        .with_context(|| format!("open store at {}", store_path.display()))?;
    if store.has(&id) {
        println!("present\t{id}");
        Ok(())
    } else {
        bail!("missing\t{id}");
    }
}

/// Read-only CAS integrity scrub: list loose chunks and `get_verify` each id.
///
/// Prints per-bad-chunk lines (`scrub: corrupt <id>` / `scrub: unreadable <id>`)
/// and a summary `scrub: ok=… corrupt=… unreadable=…`. Never deletes. Exit
/// non-zero iff corrupt+unreadable > 0 (empty store → all zeros, exit 0).
fn cmd_store_scrub(
    store_path: &Path,
    jobs: usize,
    format: CliFormat,
    progress: bool,
) -> Result<()> {
    let store = Store::open(store_path)
        .with_context(|| format!("open store at {}", store_path.display()))?;
    let mut ids = store
        .list_chunk_ids()
        .with_context(|| format!("list chunks in {}", store_path.display()))?;
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

/// Local CAS size observation: chunk count + on-disk `.cnk` bytes.
///
/// Text (default): one stdout line
/// `store stats: chunks=N bytes_on_disk=M compression=none|zstd`.
/// Json: one object (`ok`, `chunks`, `bytes_on_disk`, `compression`); no text
/// dual-write. Never deletes. Exit 0 on success for both formats.
fn cmd_store_stats(store_path: &Path, format: CliFormat) -> Result<()> {
    let store = Store::open(store_path)
        .with_context(|| format!("open store at {}", store_path.display()))?;
    let s = store
        .stats()
        .with_context(|| format!("stats for store {}", store_path.display()))?;
    match format {
        CliFormat::Text => {
            println!(
                "store stats: chunks={} bytes_on_disk={} compression={}",
                s.chunks,
                s.bytes_on_disk,
                s.compression.as_str()
            );
        }
        CliFormat::Json => {
            let obj = serde_json::json!({
                "ok": true,
                "chunks": s.chunks,
                "bytes_on_disk": s.bytes_on_disk,
                "compression": s.compression.as_str(),
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

    match peek_listing_kind(index_path)? {
        ListingKind::Index => {
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
