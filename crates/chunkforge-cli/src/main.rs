//! ChunkForge CLI: make / archive / extract / cat / verify / mount / doctor / gc / push / pull (+ chunk-id debug).

mod parallel;

use anyhow::{Context, Result, bail};
use chunkforge_chunk::{ChunkId, ChunkInfo, ChunkParams, chunk_bytes};
use chunkforge_index::{
    DIR_MAGIC_PREFIX, DirArchive, DirEntry, DirEntryKind, FLAG_CHUNKS_COMPRESSED_IN_STORE, Index,
    IndexEntry, MAGIC_PREFIX, SeedDecision, decide_seed_for_entry, entry_length, seed_file_map,
    validate_archive_path,
};
use chunkforge_remote::{FileUrlSource, HttpChunkSink, HttpChunkSource};
use chunkforge_store::{CacheSource, ChunkSink, ChunkSource, Compression, PutOutcome, Store};
use clap::{Parser, Subcommand};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(
    name = "chunkforge",
    version,
    about = "Content-defined chunking + BLAKE3 CAS (make / archive / extract / cat / verify / mount / doctor / gc / push / pull)",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Chunk a file, write chunks into a local store (dedup), and write a .cfidx
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
    },
    /// Archive a directory tree into a local store + `.cfdir` listing
    ///
    /// Recurses regular files only (FastCDC + BLAKE3 per file). Chunks are
    /// written into `--store` with content-addressed dedup; the output `.cfdir`
    /// records relative paths and per-file chunk tables. Symlinks, fifos,
    /// sockets, and device nodes are **skipped with a stderr warning** (P0
    /// policy: do not follow / do not record). Empty directories are omitted
    /// (extract can recreate parents from file paths). `make` single-file
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
    },
    /// Materialize a directory tree from a `.cfdir` + chunk source
    ///
    /// Reads the `.cfdir` listing and reconstitutes regular files under `-o`
    /// from `--store` / `--source` (same origin flags as `cat` / `verify`).
    /// Parent directories are created as needed. If a destination path already
    /// exists, extract fails (non-zero); `--force` is deferred. Empty `Dir`
    /// entries create directories; file modes are restored on Unix when recorded.
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
        #[command(flatten)]
        http_tmpl: HttpTemplateArgs,
        /// Max concurrent chunk fetches (default 1 = serial)
        #[arg(long, default_value_t = 1, value_name = "N")]
        jobs: u32,
        /// Input `.cfdir`
        archive: PathBuf,
        /// Output directory (created if missing; must not collide with existing files)
        #[arg(short = 'o', long = "output")]
        output: PathBuf,
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
        #[command(flatten)]
        http_tmpl: HttpTemplateArgs,
        /// Max concurrent chunk fetches (default 1 = serial / 0.3.0 behaviour)
        #[arg(long, default_value_t = 1, value_name = "N")]
        jobs: u32,
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
        #[command(flatten)]
        http_tmpl: HttpTemplateArgs,
        /// Override the virtual file name for `.cfidx` mounts (default: stem without `.cfidx`; ignored for `.cfdir`)
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Input `.cfidx` or `.cfdir`
        index: PathBuf,
        /// Empty directory to mount onto
        mountpoint: PathBuf,
    },
    /// Check indexes and chunk presence (missing ids → non-zero exit)
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
        /// One or more `.cfidx` / `.cfdir` listings to check
        #[arg(required = true, num_args = 1..)]
        indexes: Vec<PathBuf>,
    },
    /// List (or delete) unreferenced loose chunks in a local store
    Gc {
        /// Local CAS store directory
        #[arg(long)]
        store: PathBuf,
        /// Actually delete unreferenced `.cnk` files (default is dry-run)
        #[arg(long)]
        apply: bool,
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
    /// With `--verify`, after a successful upload the same `--dest` is treated as a
    /// `ChunkSource` and each listing is verified (skip verify on `--dry-run` or
    /// when push already failed).
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
    /// `--source` accepts a local path, `file://`, or `http(s)://` (same templates
    /// as `verify` / `cat`). Symmetric to `push` (store→dest) but source→store.
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
        /// One or more `.cfidx` / `.cfdir` listings whose chunk ids are fetched
        #[arg(required = true, num_args = 1..)]
        indexes: Vec<PathBuf>,
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
}

/// Optional HTTP URL / header templates for `cat` / `verify` / `mount` / `doctor` / `push` / `pull`.
///
/// Only meaningful with an `http(s)://` `--source`. Omitting all three flags
/// preserves 0.2.0 / Phase 2 default layout (`{base}/chunks/<2hex>/<62hex>.cnk`).
#[derive(Debug, Clone, Default, clap::Args)]
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
        } => cmd_make(&store, &output, &input, chunk_size.as_deref()),
        Commands::Archive {
            store,
            output,
            src_dir,
            chunk_size,
            dry_run,
            seed,
        } => cmd_archive(
            &store,
            &output,
            &src_dir,
            chunk_size.as_deref(),
            dry_run,
            seed.as_deref(),
        ),
        Commands::Extract {
            store,
            source,
            cache,
            http_tmpl,
            jobs,
            archive,
            output,
        } => {
            let jobs = parse_jobs(jobs)?;
            let src = open_chunk_source(
                store.as_deref(),
                source.as_deref(),
                cache.as_deref(),
                &http_tmpl,
            )?;
            cmd_extract(src.as_ref(), &archive, &output, jobs)
        }
        Commands::Cat {
            store,
            source,
            cache,
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
                &http_tmpl,
            )?;
            cmd_cat(src.as_ref(), &index, &output, jobs)
        }
        Commands::Verify {
            store,
            source,
            cache,
            http_tmpl,
            jobs,
            index,
        } => {
            let jobs = parse_jobs(jobs)?;
            let src = open_chunk_source(
                store.as_deref(),
                source.as_deref(),
                cache.as_deref(),
                &http_tmpl,
            )?;
            cmd_verify(src.as_ref(), &index, jobs)
        }
        Commands::ChunkId { input, chunk_size } => cmd_chunk_id(&input, chunk_size.as_deref()),
        Commands::Mount {
            store,
            source,
            cache,
            http_tmpl,
            name,
            index,
            mountpoint,
        } => {
            ensure_mount_supported()?;
            let src = open_chunk_source(
                store.as_deref(),
                source.as_deref(),
                cache.as_deref(),
                &http_tmpl,
            )?;
            cmd_mount(src, &index, &mountpoint, name.as_deref())
        }
        Commands::Doctor {
            store,
            source,
            http_tmpl,
            jobs,
            deep,
            no_probe,
            indexes,
        } => {
            let jobs = parse_jobs(jobs)?;
            let src = open_chunk_source(store.as_deref(), source.as_deref(), None, &http_tmpl)?;
            let origin_spec = match (store.as_deref(), source.as_deref()) {
                (Some(path), None) => path.to_string_lossy().into_owned(),
                (None, Some(s)) => s.to_string(),
                _ => unreachable!("clap origin group requires exactly one of --store/--source"),
            };
            cmd_doctor(src.as_ref(), &origin_spec, &indexes, deep, no_probe, jobs)
        }
        Commands::Gc {
            store,
            apply,
            indexes,
        } => cmd_gc(&store, &indexes, apply),
        Commands::Push {
            store,
            dest,
            http_tmpl,
            jobs,
            dry_run,
            verify,
            indexes,
        } => {
            let jobs = parse_jobs(jobs)?;
            cmd_push(&store, &dest, &http_tmpl, dry_run, verify, &indexes, jobs)
        }
        Commands::Pull {
            store,
            source,
            http_tmpl,
            jobs,
            dry_run,
            indexes,
        } => {
            let jobs = parse_jobs(jobs)?;
            cmd_pull(&store, &source, &http_tmpl, dry_run, &indexes, jobs)
        }
        Commands::Store {
            command: StoreCommands::Has { store, hex_id },
        } => cmd_store_has(&store, &hex_id),
    }
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
/// `--url-template` / `--prefix` / `--header` apply only to `http(s)://` sources.
fn open_chunk_source(
    store: Option<&Path>,
    source: Option<&str>,
    cache: Option<&Path>,
    http_tmpl: &HttpTemplateArgs,
) -> Result<Box<dyn ChunkSource>> {
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
            Ok(Box::new(CacheSource::new(primary, cache_store)))
        }
    }
}

fn http_template_flags_set(http_tmpl: &HttpTemplateArgs) -> bool {
    http_tmpl.url_template.is_some() || http_tmpl.prefix.is_some() || !http_tmpl.headers.is_empty()
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
            "--url-template / --prefix / --header apply only to http(s):// sources;              got non-HTTP source {trimmed:?}"
        );
    }

    if is_http {
        let mut builder = HttpChunkSource::builder(trimmed).timeout(Some(Duration::from_secs(30)));
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
    Ok(())
}

fn cmd_archive(
    store_path: &Path,
    output: &Path,
    src_dir: &Path,
    chunk_size: Option<&str>,
    dry_run: bool,
    seed: Option<&Path>,
) -> Result<()> {
    let params = parse_chunk_size(chunk_size)?;

    if !src_dir.is_dir() {
        bail!(
            "archive source {} is not a directory (or is unreadable)",
            src_dir.display()
        );
    }

    // Load prior `.cfdir` for --seed (fail non-zero on bad magic / decode / .cfidx).
    let prior_arch = match seed {
        Some(seed_path) => Some(load_seed_cfdir(seed_path)?),
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

    if skipped_symlinks > 0 || skipped_special > 0 {
        eprintln!(
            "archive: symlink policy = skip+warn (not recorded / not followed); \
             skipped {skipped_symlinks} symlink{}, {skipped_special} special (fifo/socket/device)",
            if skipped_symlinks == 1 { "" } else { "s" },
        );
    }

    let mut entries: Vec<DirEntry> = Vec::with_capacity(file_paths.len());
    let mut total_chunks = 0usize;
    let mut new_chunks = 0usize;
    let mut reused_chunks = 0usize;
    let mut seed_reused_files = 0usize;
    let mut rechunked_files = 0usize;
    let mut seed_missing_chunks = 0usize;
    // Dry-run: track ids we would write this run so cross-file dedup is counted.
    let mut dry_seen: HashSet<ChunkId> = HashSet::new();

    for full in &file_paths {
        let rel = relative_archive_path(src_dir, full)?;
        validate_archive_path(&rel)
            .map_err(|e| anyhow::anyhow!("invalid archive path {rel:?}: {e}"))?;

        let meta =
            fs::symlink_metadata(full).with_context(|| format!("stat {}", full.display()))?;
        if meta.file_type().is_symlink() {
            // Race: became a symlink after collect — skip (summary already printed).
            eprintln!(
                "archive: skip symlink {} (policy: skip+warn; not recorded)",
                full.display()
            );
            continue;
        }
        if !meta.is_file() {
            eprintln!(
                "archive: skip special file {} (fifo/socket/device)",
                full.display()
            );
            continue;
        }

        let source_size = meta.len();

        // --- Seed reuse path -------------------------------------------------
        let mut reused_from_seed = false;
        if let Some(ref map) = seed_map {
            if let Some(prior_entry) = map.get(rel.as_str()) {
                let mut f = File::open(full)
                    .with_context(|| format!("open {} for seed hash", full.display()))?;
                let decision = decide_seed_for_entry(prior_entry, source_size, &mut f)
                    .map_err(|e| anyhow::anyhow!("seed decide for {rel}: {e}"))?;
                if decision == SeedDecision::Reuse {
                    let prior_chunks = match &prior_entry.kind {
                        DirEntryKind::File { chunks, .. } => chunks.as_slice(),
                        DirEntryKind::Dir { .. } => unreachable!("seed map is files only"),
                    };
                    let all_present = match store.as_ref() {
                        Some(s) => prior_chunks.iter().all(|e| s.has(&e.chunk_id)),
                        // Dry-run with no store: cannot verify → force rechunk.
                        None => false,
                    };
                    if all_present {
                        // Copy prior fields; do not FastCDC / do not put.
                        let chunk_n = prior_chunks.len();
                        total_chunks += chunk_n;
                        reused_chunks += chunk_n;
                        seed_reused_files += 1;
                        entries.push(DirEntry {
                            path: rel.clone(),
                            kind: prior_entry.kind.clone(),
                        });
                        reused_from_seed = true;
                    } else {
                        let missing: Vec<_> = match store.as_ref() {
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
                        seed_missing_chunks += 1;
                    }
                }
            }
        }

        if reused_from_seed {
            continue;
        }

        // --- FastCDC + store put (unchanged / rechunk / no seed) ------------
        rechunked_files += 1;
        let data = fs::read(full).with_context(|| format!("read {}", full.display()))?;
        let mode = file_mode_u32(&meta);
        let mtime_secs = file_mtime_secs(&meta);
        let blob_blake3 = ChunkId::hash(&data);
        let chunks = chunk_bytes(&data, &params);

        let mut index_entries = Vec::with_capacity(chunks.len());
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
                let exists_in_store = store.as_ref().is_some_and(|s| s.has(&c.id));
                if exists_in_store || dry_seen.contains(&c.id) {
                    reused_chunks += 1;
                } else {
                    dry_seen.insert(c.id);
                    new_chunks += 1;
                }
            } else {
                let store = store.as_ref().expect("store open when not dry-run");
                let outcome = store
                    .put_with_id(&c.id, slice)
                    .with_context(|| format!("put chunk {} (file {rel})", c.id))?;
                match outcome {
                    PutOutcome::Written => new_chunks += 1,
                    PutOutcome::SkippedExists => reused_chunks += 1,
                }
            }
            total_chunks += 1;
            index_entries.push(IndexEntry {
                end_offset: c.offset + c.length,
                chunk_id: c.id,
            });
        }

        entries.push(DirEntry {
            path: rel,
            kind: DirEntryKind::File {
                mode,
                size: data.len() as u64,
                mtime_secs,
                blob_blake3,
                chunks: index_entries,
            },
        });
    }

    let file_count = entries.len();

    if dry_run {
        if seeding {
            eprintln!(
                "archive: dry-run: {} file{}, {} chunk{} (would_write={}, would_reuse={}; \
                 would_seed_reuse={}, would_rechunk={}{}); \
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
                output.display()
            );
        } else {
            eprintln!(
                "archive: dry-run: {} file{}, {} chunk{} (would_write={}, would_reuse={}); \
                 no store/.cfdir written (would write {})",
                file_count,
                if file_count == 1 { "" } else { "s" },
                total_chunks,
                if total_chunks == 1 { "" } else { "s" },
                new_chunks,
                reused_chunks,
                output.display()
            );
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

    if seeding {
        eprintln!(
            "archive: wrote {} ({} file{}, {} chunk{}; new={}, reused={}; \
             seed_reused_files={}, rechunked_files={}{}) → store {}",
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
            store_path.display()
        );
    } else {
        eprintln!(
            "archive: wrote {} ({} file{}, {} chunk{}; new={}, reused={}) → store {}",
            output.display(),
            file_count,
            if file_count == 1 { "" } else { "s" },
            total_chunks,
            if total_chunks == 1 { "" } else { "s" },
            new_chunks,
            reused_chunks,
            store_path.display()
        );
    }
    Ok(())
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
/// per-path stderr warning. Does **not** follow directory symlinks (avoids loops).
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
    match peek_listing_kind(path)? {
        ListingKind::Index => {
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
            Ok(arch.all_chunk_ids().collect())
        }
    }
}

/// Union of chunk ids across mixed `.cfidx` / `.cfdir` listing args.
fn union_listing_chunk_ids(paths: &[PathBuf]) -> Result<(HashSet<ChunkId>, usize)> {
    let mut referenced = HashSet::new();
    let mut listings_ok = 0usize;
    for path in paths {
        for id in listing_chunk_ids(path)? {
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

fn cmd_verify(source: &dyn ChunkSource, listing_path: &Path, jobs: usize) -> Result<()> {
    match peek_listing_kind(listing_path)? {
        ListingKind::Index => cmd_verify_index(source, listing_path, jobs),
        ListingKind::DirArchive => cmd_verify_dir(source, listing_path, jobs),
    }
}

fn cmd_verify_index(source: &dyn ChunkSource, index_path: &Path, jobs: usize) -> Result<()> {
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

    eprintln!(
        "verify: ok ({} bytes, {} chunk{})",
        index.total_size,
        index.chunk_count(),
        if index.chunk_count() == 1 { "" } else { "s" }
    );
    Ok(())
}

fn cmd_verify_dir(source: &dyn ChunkSource, archive_path: &Path, jobs: usize) -> Result<()> {
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

    eprintln!(
        "verify: ok ({} file{}, {} chunk{})",
        file_count,
        if file_count == 1 { "" } else { "s" },
        total_chunks,
        if total_chunks == 1 { "" } else { "s" }
    );
    Ok(())
}

fn cmd_extract(
    source: &dyn ChunkSource,
    archive_path: &Path,
    out_dir: &Path,
    jobs: usize,
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

    if out_dir.exists() {
        if out_dir.is_file() {
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

    for entry in &archive.entries {
        let dest = join_archive_path(out_dir, &entry.path)?;
        match &entry.kind {
            DirEntryKind::Dir { mode } => {
                if dest.exists() {
                    if !dest.is_dir() {
                        bail!(
                            "extract target {} already exists and is not a directory",
                            dest.display()
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
                blob_blake3,
                chunks,
                ..
            } => {
                if dest.exists() {
                    bail!(
                        "extract target {} already exists (refusing to overwrite; no --force in this milestone)",
                        dest.display()
                    );
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

    eprintln!(
        "extract: wrote {} ({} file{}, {} dir{})",
        out_dir.display(),
        file_count,
        if file_count == 1 { "" } else { "s" },
        dir_count,
        if dir_count == 1 { "" } else { "s" }
    );
    Ok(())
}

fn cmd_doctor(
    source: &dyn ChunkSource,
    origin_spec: &str,
    index_paths: &[PathBuf],
    deep: bool,
    no_probe: bool,
    jobs: usize,
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
        eprintln!(
            "doctor: ok ({} listing{}, {} chunk id{} checked, deep={})",
            listings_ok,
            if listings_ok == 1 { "" } else { "s" },
            checked,
            if checked == 1 { "" } else { "s" },
            deep
        );
        Ok(())
    } else {
        for id in &missing {
            println!("{id}");
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

fn cmd_gc(store_path: &Path, index_paths: &[PathBuf], apply: bool) -> Result<()> {
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

    if unreferenced.is_empty() {
        eprintln!(
            "gc: nothing to reclaim ({} listing{}, {} referenced chunk id{}, dry_run={})",
            listings_ok,
            if listings_ok == 1 { "" } else { "s" },
            referenced.len(),
            if referenced.len() == 1 { "" } else { "s" },
            !apply
        );
        return Ok(());
    }

    for id in &unreferenced {
        let path = store.chunk_path(id);
        println!("{}", path.display());
    }

    if apply {
        for id in &unreferenced {
            store
                .remove(id)
                .with_context(|| format!("delete unreferenced chunk {id}"))?;
        }
        eprintln!(
            "gc: deleted {} unreferenced chunk{} ({} listing{}, {} referenced retained)",
            unreferenced.len(),
            if unreferenced.len() == 1 { "" } else { "s" },
            listings_ok,
            if listings_ok == 1 { "" } else { "s" },
            referenced.len()
        );
    } else {
        eprintln!(
            "gc: dry-run: {} unreferenced chunk{} (pass --apply to delete; {} listing{}, {} referenced)",
            unreferenced.len(),
            if unreferenced.len() == 1 { "" } else { "s" },
            listings_ok,
            if listings_ok == 1 { "" } else { "s" },
            referenced.len()
        );
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

    let mut builder = HttpChunkSink::builder(trimmed).timeout(Some(Duration::from_secs(30)));
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
    builder.build().context("build HTTP chunk sink")
}

fn cmd_push(
    store_path: &Path,
    dest: &str,
    http_tmpl: &HttpTemplateArgs,
    dry_run: bool,
    verify: bool,
    index_paths: &[PathBuf],
    jobs: usize,
) -> Result<()> {
    let store = Store::open(store_path)
        .with_context(|| format!("open store at {}", store_path.display()))?;
    let sink = open_http_chunk_sink(dest, http_tmpl)?;

    let (referenced, listings_ok) = union_listing_chunk_ids(index_paths)?;

    let mut ids: Vec<ChunkId> = referenced.into_iter().collect();
    ids.sort();

    #[derive(Clone, Copy)]
    enum PushOne {
        Skipped,
        Uploaded,
        Failed,
    }

    let store_display = store_path.display().to_string();
    let outcomes = parallel::map_indexed(&ids, jobs, |_i, id| {
        let plain = match store.get(id) {
            Ok(bytes) => bytes,
            Err(e) => {
                let msg = format!("local chunk {id} unavailable from store {store_display}: {e}");
                eprintln!("push: fail {id}: {msg}");
                return (PushOne::Failed, Some(msg));
            }
        };

        match sink.has(id) {
            Ok(true) => return (PushOne::Skipped, None),
            Ok(false) => {}
            Err(e) => {
                let msg = format!("remote has check failed for {id}: {e}");
                eprintln!("push: fail {id}: {msg}");
                return (PushOne::Failed, Some(msg));
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
                eprintln!("push: fail {id}: {msg}");
                (PushOne::Failed, Some(msg))
            }
        }
    });

    let mut skipped = 0usize;
    let mut uploaded = 0usize;
    let mut failed = 0usize;
    let mut first_error: Option<String> = None;
    for (outcome, err) in outcomes {
        match outcome {
            PushOne::Skipped => skipped += 1,
            PushOne::Uploaded => uploaded += 1,
            PushOne::Failed => {
                failed += 1;
                if first_error.is_none() {
                    first_error = err;
                }
            }
        }
    }

    eprintln!(
        "push: skipped={skipped} uploaded={uploaded} failed={failed} \
         ({} unique chunk id{}, {} listing{}, dry_run={dry_run})",
        ids.len(),
        if ids.len() == 1 { "" } else { "s" },
        listings_ok,
        if listings_ok == 1 { "" } else { "s" },
    );

    if failed > 0 {
        // Push already failed: do not claim verify success; skip post-verify.
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
            cmd_verify(source.as_ref(), path, jobs).with_context(|| {
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

fn cmd_pull(
    store_path: &Path,
    source_spec: &str,
    http_tmpl: &HttpTemplateArgs,
    dry_run: bool,
    index_paths: &[PathBuf],
    jobs: usize,
) -> Result<()> {
    let source = open_primary_source(source_spec, http_tmpl)
        .with_context(|| format!("open chunk source {source_spec:?}"))?;

    let (referenced, listings_ok) = union_listing_chunk_ids(index_paths)?;

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
        Failed,
    }

    let store_display = store_path.display().to_string();
    let outcomes = parallel::map_indexed(&ids, jobs, |_i, id| {
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
                eprintln!("pull: fail {id}: {msg}");
                return (PullOne::Failed, Some(msg));
            }
        };

        match ChunkSink::put(store, id, &plain) {
            Ok(PutOutcome::Written) => (PullOne::Fetched, None),
            Ok(PutOutcome::SkippedExists) => (PullOne::Skipped, None),
            Err(e) => {
                let msg = format!("store put failed for {id}: {e}");
                eprintln!("pull: fail {id}: {msg}");
                (PullOne::Failed, Some(msg))
            }
        }
    });

    let mut skipped = 0usize;
    let mut fetched = 0usize;
    let mut failed = 0usize;
    let mut first_error: Option<String> = None;
    for (outcome, err) in outcomes {
        match outcome {
            PullOne::Skipped => skipped += 1,
            PullOne::Fetched => fetched += 1,
            PullOne::Failed => {
                failed += 1;
                if first_error.is_none() {
                    first_error = err;
                }
            }
        }
    }

    eprintln!(
        "pull: skipped={skipped} fetched={fetched} failed={failed} ({} unique chunk id{}, {} listing{}, dry_run={dry_run})",
        ids.len(),
        if ids.len() == 1 { "" } else { "s" },
        listings_ok,
        if listings_ok == 1 { "" } else { "s" },
    );

    if failed > 0 {
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
) -> Result<()> {
    #[cfg(feature = "fuse")]
    {
        cmd_mount_fuse(source, index_path, mountpoint, name)
    }
    #[cfg(not(feature = "fuse"))]
    {
        let _ = (source, index_path, mountpoint, name);
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
            let fs = BlobFs::new(index, source, blob_name.clone());

            eprintln!(
                "mount: {} → {}/{} (read-only; Ctrl-C or fusermount3 -u to unmount)",
                index_path.display(),
                mountpoint.display(),
                blob_name
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
            let fs = DirFs::new(archive, source);

            eprintln!(
                "mount: {} → {}/ (directory tree, read-only; Ctrl-C or fusermount3 -u to unmount)",
                index_path.display(),
                mountpoint.display()
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
