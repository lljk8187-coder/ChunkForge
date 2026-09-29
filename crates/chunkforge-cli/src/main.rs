//! ChunkForge CLI: make / cat / verify / mount (+ chunk-id debug).

use anyhow::{Context, Result, bail};
use chunkforge_chunk::{ChunkId, ChunkInfo, ChunkParams, chunk_bytes};
use chunkforge_index::{FLAG_CHUNKS_COMPRESSED_IN_STORE, Index, IndexEntry, entry_length};
use chunkforge_remote::{FileUrlSource, HttpChunkSource};
use chunkforge_store::{CacheSource, ChunkSource, Compression, PutOutcome, Store};
use clap::{Parser, Subcommand};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(
    name = "chunkforge",
    version,
    about = "Content-defined chunking + BLAKE3 CAS (make / cat / verify / mount)",
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
        /// Override the virtual file name (default: index stem without `.cfidx`)
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Input .cfidx
        index: PathBuf,
        /// Empty directory to mount onto
        mountpoint: PathBuf,
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
            index,
            output,
        } => {
            let src = open_chunk_source(store.as_deref(), source.as_deref(), cache.as_deref())?;
            cmd_cat(src.as_ref(), &index, &output)
        }
        Commands::Verify {
            store,
            source,
            cache,
            index,
        } => {
            let src = open_chunk_source(store.as_deref(), source.as_deref(), cache.as_deref())?;
            cmd_verify(src.as_ref(), &index)
        }
        Commands::ChunkId { input, chunk_size } => cmd_chunk_id(&input, chunk_size.as_deref()),
        Commands::Mount {
            store,
            source,
            cache,
            name,
            index,
            mountpoint,
        } => {
            ensure_mount_supported()?;
            let src = open_chunk_source(store.as_deref(), source.as_deref(), cache.as_deref())?;
            cmd_mount(src, &index, &mountpoint, name.as_deref())
        }
        Commands::Store {
            command: StoreCommands::Has { store, hex_id },
        } => cmd_store_has(&store, &hex_id),
    }
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

/// Resolve `--store` / `--source` / `--cache` into a boxed [`ChunkSource`].
///
/// `--store PATH` is a Phase 1 synonym for `--source PATH` (local only).
/// With `--cache`, reads go through [`CacheSource`] (fill on miss; never write primary).
fn open_chunk_source(
    store: Option<&Path>,
    source: Option<&str>,
    cache: Option<&Path>,
) -> Result<Box<dyn ChunkSource>> {
    let spec = match (store, source) {
        (Some(path), None) => path.to_string_lossy().into_owned(),
        (None, Some(s)) => s.to_string(),
        (Some(_), Some(_)) => bail!("use either --store or --source, not both"),
        (None, None) => bail!("missing chunk origin: pass --store <path> or --source <PATH|URL>"),
    };

    let primary = open_primary_source(&spec)?;
    match cache {
        None => Ok(primary),
        Some(cache_path) => {
            let cache_store = open_or_create_store(cache_path)?;
            Ok(Box::new(CacheSource::new(primary, cache_store)))
        }
    }
}

fn open_primary_source(spec: &str) -> Result<Box<dyn ChunkSource>> {
    let trimmed = spec.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let src = HttpChunkSource::builder(trimmed)
            .timeout(Some(Duration::from_secs(30)))
            .build();
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
            PutOutcome::Inserted => new_chunks += 1,
            PutOutcome::AlreadyPresent => reused_chunks += 1,
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

fn cmd_cat(source: &dyn ChunkSource, index_path: &Path, output: &Path) -> Result<()> {
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

    for (i, entry) in index.entries.iter().enumerate() {
        let expected_len = entry_length(&index.entries, i).expect("entry index in range");
        let plain = source.get(&entry.chunk_id).with_context(|| {
            format!(
                "missing or corrupt chunk {} (index entry {i})",
                entry.chunk_id
            )
        })?;
        if plain.len() as u64 != expected_len {
            bail!(
                "chunk {} length mismatch: source has {} bytes, index expects {expected_len}",
                entry.chunk_id,
                plain.len()
            );
        }
        writer
            .write_all(&plain)
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

fn cmd_verify(source: &dyn ChunkSource, index_path: &Path) -> Result<()> {
    let index = load_index(index_path)?;
    index
        .validate()
        .map_err(|e| anyhow::anyhow!("index structure: {e}"))?;

    let mut hasher = blake3::Hasher::new();
    let mut assembled: u64 = 0;

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
        hasher.update(&plain);
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
