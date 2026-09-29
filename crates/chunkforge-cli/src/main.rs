//! ChunkForge CLI: make / cat / verify / mount / doctor / gc / push (+ chunk-id debug).

mod parallel;

use anyhow::{Context, Result, bail};
use chunkforge_chunk::{ChunkId, ChunkInfo, ChunkParams, chunk_bytes};
use chunkforge_index::{FLAG_CHUNKS_COMPRESSED_IN_STORE, Index, IndexEntry, entry_length};
use chunkforge_remote::{FileUrlSource, HttpChunkSink, HttpChunkSource};
use chunkforge_store::{CacheSource, ChunkSink, ChunkSource, Compression, PutOutcome, Store};
use clap::{Parser, Subcommand};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(
    name = "chunkforge",
    version,
    about = "Content-defined chunking + BLAKE3 CAS (make / cat / verify / mount / doctor / gc / push)",
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
    /// Verify index integrity, chunk presence/hashes, and blob_blake3
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
        /// Input .cfidx
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
    /// Mount a .cfidx as a single read-only virtual file (Linux + fuse3)
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
        /// Override the virtual file name (default: index stem without `.cfidx`)
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Input .cfidx
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
        /// One or more `.cfidx` files to check
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
        /// One or more `.cfidx` files whose chunk ids are retained
        #[arg(required = true, num_args = 1..)]
        indexes: Vec<PathBuf>,
    },
    /// Upload missing chunks referenced by .cfidx files to an HTTP(S) destination
    ///
    /// Reads plaintext chunks from the local `--store`, probes the remote with
    /// `has`, and PUTs only missing ids. Does **not** upload `.cfidx` files.
    /// Template flags (`--url-template` / `--prefix` / `--header`) match read-side
    /// layout so a successful push is readable with `verify --source`.
    Push {
        /// Local CAS store providing plaintext chunks
        #[arg(long)]
        store: PathBuf,
        /// HTTP(S) destination base URL (same layout as `--source` for verify/cat)
        #[arg(long, value_name = "URL")]
        dest: String,
        #[command(flatten)]
        http_tmpl: HttpTemplateArgs,
        /// Max concurrent has/PUT workers (default 1 = serial)
        #[arg(long, default_value_t = 1, value_name = "N")]
        jobs: u32,
        /// Probe and count only; do not issue PUT
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// One or more `.cfidx` files whose chunk ids are uploaded
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

/// Optional HTTP URL / header templates for `cat` / `verify` / `mount` / `doctor` / `push`.
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
            indexes,
        } => {
            let jobs = parse_jobs(jobs)?;
            cmd_push(&store, &dest, &http_tmpl, dry_run, &indexes, jobs)
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

fn load_index(path: &Path) -> Result<Index> {
    let bytes = fs::read(path).with_context(|| format!("read index {}", path.display()))?;
    Index::decode(&bytes).map_err(|e| anyhow::anyhow!("decode index {}: {e}", path.display()))
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
    let plains = fetch_index_plains(source, &index, jobs, "cat")?;
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

/// Fetch every index entry's plaintext with bounded concurrency; results in entry order.
///
/// When `jobs == 1`, work is serial and fail-fast on the calling thread (≡ 0.3.0).
/// Errors always include the chunk id.
fn fetch_index_plains(
    source: &dyn ChunkSource,
    index: &Index,
    jobs: usize,
    op: &str,
) -> Result<Vec<Vec<u8>>> {
    let tasks: Vec<(usize, ChunkId, u64)> = index
        .entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let expected_len = entry_length(&index.entries, i).expect("entry index in range");
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

fn cmd_verify(source: &dyn ChunkSource, index_path: &Path, jobs: usize) -> Result<()> {
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
        fetch_index_plains(source, &index, jobs, "verify failed")?
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

    // Load + validate all indexes first (serial; cheap).
    let mut loaded: Vec<(PathBuf, Index)> = Vec::with_capacity(index_paths.len());
    for index_path in index_paths {
        let index = load_index(index_path)?;
        index
            .validate()
            .map_err(|e| anyhow::anyhow!("index structure {}: {e}", index_path.display()))?;
        loaded.push((index_path.clone(), index));
    }
    let indexes_ok = loaded.len();

    let mut missing: Vec<ChunkId> = Vec::new();
    let mut checked: usize = 0;

    if jobs <= 1 {
        // Serial fail-fast ≡ 0.3.0.
        for (path, index) in &loaded {
            for entry in &index.entries {
                checked += 1;
                let present = if deep {
                    match source.get(&entry.chunk_id) {
                        Ok(_bytes) => true,
                        Err(chunkforge_store::SourceError::NotFound(_)) => false,
                        Err(e) => {
                            bail!(
                                "doctor: chunk {} check error (index {}): {e}",
                                entry.chunk_id,
                                path.display()
                            );
                        }
                    }
                } else {
                    match source.has(&entry.chunk_id) {
                        Ok(true) => true,
                        Ok(false) => false,
                        Err(e) => {
                            bail!(
                                "doctor: chunk {} presence check error (index {}); \
                                 retry with --deep to use get instead of has: {e}",
                                entry.chunk_id,
                                path.display()
                            );
                        }
                    }
                };
                if !present {
                    missing.push(entry.chunk_id);
                }
            }
        }
    } else {
        let mut checks: Vec<(String, ChunkId)> = Vec::new();
        for (path, index) in &loaded {
            let display = path.display().to_string();
            for entry in &index.entries {
                checks.push((display.clone(), entry.chunk_id));
            }
        }
        checked = checks.len();

        let outcomes = parallel::map_indexed(&checks, jobs, |_i, (index_display, chunk_id)| {
            if deep {
                match source.get(chunk_id) {
                    Ok(_bytes) => Ok(true),
                    Err(chunkforge_store::SourceError::NotFound(_)) => Ok(false),
                    Err(e) => Err(format!(
                        "doctor: chunk {chunk_id} check error (index {index_display}): {e}"
                    )),
                }
            } else {
                match source.has(chunk_id) {
                    Ok(true) => Ok(true),
                    Ok(false) => Ok(false),
                    Err(e) => Err(format!(
                        "doctor: chunk {chunk_id} presence check error (index {index_display}); \
                         retry with --deep to use get instead of has: {e}"
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

    // Deduplicate while sorting (multi-index overlap) — same as prior behaviour.
    missing.sort();
    missing.dedup();

    if missing.is_empty() {
        eprintln!(
            "doctor: ok ({} index{}, {} chunk id{} checked, deep={})",
            indexes_ok,
            if indexes_ok == 1 { "" } else { "es" },
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
            "doctor: {} missing chunk{} ({} index{}, {} checked)",
            missing.len(),
            if missing.len() == 1 { "" } else { "s" },
            indexes_ok,
            if indexes_ok == 1 { "" } else { "es" },
            checked
        );
    }
}

fn cmd_gc(store_path: &Path, index_paths: &[PathBuf], apply: bool) -> Result<()> {
    let store = Store::open(store_path)
        .with_context(|| format!("open store at {}", store_path.display()))?;

    let mut referenced: HashSet<ChunkId> = HashSet::new();
    let mut indexes_ok = 0usize;
    for index_path in index_paths {
        let index = load_index(index_path)?;
        index
            .validate()
            .map_err(|e| anyhow::anyhow!("index structure {}: {e}", index_path.display()))?;
        indexes_ok += 1;
        for entry in &index.entries {
            referenced.insert(entry.chunk_id);
        }
    }

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
            "gc: nothing to reclaim ({} index{}, {} referenced chunk id{}, dry_run={})",
            indexes_ok,
            if indexes_ok == 1 { "" } else { "es" },
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
            "gc: deleted {} unreferenced chunk{} ({} index{}, {} referenced retained)",
            unreferenced.len(),
            if unreferenced.len() == 1 { "" } else { "s" },
            indexes_ok,
            if indexes_ok == 1 { "" } else { "es" },
            referenced.len()
        );
    } else {
        eprintln!(
            "gc: dry-run: {} unreferenced chunk{} (pass --apply to delete; {} index{}, {} referenced)",
            unreferenced.len(),
            if unreferenced.len() == 1 { "" } else { "s" },
            indexes_ok,
            if indexes_ok == 1 { "" } else { "es" },
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
    index_paths: &[PathBuf],
    jobs: usize,
) -> Result<()> {
    let store = Store::open(store_path)
        .with_context(|| format!("open store at {}", store_path.display()))?;
    let sink = open_http_chunk_sink(dest, http_tmpl)?;

    let mut referenced: HashSet<ChunkId> = HashSet::new();
    let mut indexes_ok = 0usize;
    for index_path in index_paths {
        let index = load_index(index_path)?;
        index
            .validate()
            .map_err(|e| anyhow::anyhow!("index structure {}: {e}", index_path.display()))?;
        indexes_ok += 1;
        for entry in &index.entries {
            referenced.insert(entry.chunk_id);
        }
    }

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
         ({} unique chunk id{}, {} index{}, dry_run={dry_run})",
        ids.len(),
        if ids.len() == 1 { "" } else { "s" },
        indexes_ok,
        if indexes_ok == 1 { "" } else { "es" },
    );

    if failed > 0 {
        bail!(
            "push: {failed} failure{}{}",
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
    use chunkforge_fuse::{BlobFs, MountOption, default_blob_name, mount_ro};

    if !mountpoint.exists() {
        bail!(
            "mountpoint {} does not exist (create an empty directory first)",
            mountpoint.display()
        );
    }
    if !mountpoint.is_dir() {
        bail!("mountpoint {} is not a directory", mountpoint.display());
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
    let fs = BlobFs::new(index, source, blob_name.clone());

    eprintln!(
        "mount: {} → {}/{} (read-only; Ctrl-C or fusermount3 -u to unmount)",
        index_path.display(),
        mountpoint.display(),
        blob_name
    );

    mount_ro(
        fs,
        mountpoint,
        [
            MountOption::FSName("chunkforge".into()),
            MountOption::AutoUnmount,
            MountOption::DefaultPermissions,
        ],
    )
    .map_err(explain_fuse_mount_error)?;

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
