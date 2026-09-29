//! Read-only directory-tree FUSE [`Filesystem`] backed by a `.cfdir` [`DirArchive`].

use crate::prefetch::PrefetchCache;
use crate::read::read_entries_cached;
use chunkforge_index::{DirArchive, DirEntryKind, IndexEntry};
use chunkforge_store::{ChunkSource, SourceError};
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen,
    ReplyStatfs, ReplyWrite, Request, TimeOrNow,
};
use libc::{EACCES, EINVAL, EISDIR, ENOENT, ENOTDIR, EROFS};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::fs::ROOT_INO;

const TTL: Duration = Duration::from_secs(1);
const BLKSIZE: u32 = 512;
/// Default mode for directories synthesized from path prefixes (no explicit Dir entry).
const SYNTH_DIR_MODE: u32 = 0o555;

/// Error from path-based library helpers on [`DirFs`] (no FUSE session required).
#[derive(Debug, thiserror::Error)]
pub enum DirFsError {
    #[error("path not found")]
    NotFound,
    #[error("path is a directory, not a file")]
    IsDirectory,
    #[error("path is not a directory")]
    NotADirectory,
    #[error(transparent)]
    Source(#[from] SourceError),
}

#[derive(Debug)]
enum NodeKind {
    Dir {
        /// Child name → inode (sorted for stable readdir).
        children: BTreeMap<String, u64>,
        mode: u32,
    },
    File {
        mode: u32,
        size: u64,
        mtime_secs: u64,
        chunks: Vec<IndexEntry>,
    },
}

#[derive(Debug)]
struct Node {
    parent: u64,
    kind: NodeKind,
}

/// Read-only directory tree backed by a `.cfdir` [`DirArchive`] + [`ChunkSource`].
///
/// Mount layout mirrors archive relative paths (synthesizing parent dirs from prefixes
/// when the archive lists only files). Writes return `EROFS`.
pub struct DirFs<S: ChunkSource> {
    source: S,
    /// Inode → node. Root is always [`ROOT_INO`].
    nodes: BTreeMap<u64, Node>,
    next_ino: u64,
    uid: u32,
    gid: u32,
    atime: SystemTime,
    ctime: SystemTime,
    /// Aggregate byte size of all file entries (for statfs).
    total_bytes: u64,
    file_count: u64,
    /// Process-local sequential prefetch (not Store `--cache`). Default: on.
    /// Cross-file reads (different ino) cold-start the window.
    prefetch: Mutex<PrefetchCache>,
}

impl<S: ChunkSource> DirFs<S> {
    /// Build an in-memory tree from `archive` served via `source`.
    ///
    /// Paths are assumed validated by [`DirArchive`] (no `..`, absolute, or empty).
    /// Parent directories missing as explicit entries are synthesized.
    pub fn new(archive: DirArchive, source: S) -> Self {
        let (uid, gid) = current_ids();
        let now = SystemTime::now();
        let mut fs = Self {
            source,
            nodes: BTreeMap::new(),
            next_ino: ROOT_INO + 1,
            uid,
            gid,
            atime: now,
            ctime: now,
            total_bytes: 0,
            file_count: 0,
            // M1 default: sequential prefetch on (CLI `--no-prefetch` is M2).
            prefetch: Mutex::new(PrefetchCache::enabled()),
        };
        fs.nodes.insert(
            ROOT_INO,
            Node {
                parent: ROOT_INO,
                kind: NodeKind::Dir {
                    children: BTreeMap::new(),
                    mode: SYNTH_DIR_MODE,
                },
            },
        );
        for entry in archive.entries {
            match entry.kind {
                DirEntryKind::Dir { mode } => {
                    fs.ensure_dir_path(&entry.path, mode);
                }
                DirEntryKind::File {
                    mode,
                    size,
                    mtime_secs,
                    blob_blake3: _,
                    chunks,
                } => {
                    fs.insert_file(&entry.path, mode, size, mtime_secs, chunks);
                }
            }
        }
        fs
    }

    /// Override ownership shown in getattr.
    pub fn with_owner(mut self, uid: u32, gid: u32) -> Self {
        self.uid = uid;
        self.gid = gid;
        self
    }

    /// Enable or disable sequential prefetch (library switch; CLI flag is M2).
    ///
    /// Default is **on**. Disabling cold-starts the window (≡ 0.9.0 on-demand get).
    pub fn with_prefetch(mut self, enabled: bool) -> Self {
        *self.prefetch.get_mut().unwrap_or_else(|e| e.into_inner()) = if enabled {
            PrefetchCache::enabled()
        } else {
            PrefetchCache::disabled()
        };
        self
    }

    /// Resolve a relative `/`-separated archive path to an inode (`""` / `"."` = root).
    pub fn lookup_path(&self, path: &str) -> Option<u64> {
        let path = path.trim_matches('/');
        if path.is_empty() || path == "." {
            return Some(ROOT_INO);
        }
        let mut ino = ROOT_INO;
        for seg in path.split('/') {
            if seg.is_empty() || seg == "." {
                continue;
            }
            if seg == ".." {
                return None;
            }
            let node = self.nodes.get(&ino)?;
            match &node.kind {
                NodeKind::Dir { children, .. } => {
                    ino = *children.get(seg)?;
                }
                NodeKind::File { .. } => return None,
            }
        }
        Some(ino)
    }

    /// Whether `path` resolves to a directory.
    pub fn is_dir_path(&self, path: &str) -> bool {
        self.lookup_path(path)
            .and_then(|ino| self.nodes.get(&ino))
            .is_some_and(|n| matches!(n.kind, NodeKind::Dir { .. }))
    }

    /// Whether `path` resolves to a regular file.
    pub fn is_file_path(&self, path: &str) -> bool {
        self.lookup_path(path)
            .and_then(|ino| self.nodes.get(&ino))
            .is_some_and(|n| matches!(n.kind, NodeKind::File { .. }))
    }

    /// List direct child names of a directory path (no `.` / `..`).
    ///
    /// Returns `(name, is_dir)` sorted by name.
    pub fn readdir_path(&self, path: &str) -> Result<Vec<(String, bool)>, DirFsError> {
        let ino = self.lookup_path(path).ok_or(DirFsError::NotFound)?;
        let node = self.nodes.get(&ino).ok_or(DirFsError::NotFound)?;
        match &node.kind {
            NodeKind::Dir { children, .. } => {
                let mut out = Vec::with_capacity(children.len());
                for (name, child_ino) in children {
                    let is_dir = self
                        .nodes
                        .get(child_ino)
                        .is_some_and(|n| matches!(n.kind, NodeKind::Dir { .. }));
                    out.push((name.clone(), is_dir));
                }
                Ok(out)
            }
            NodeKind::File { .. } => Err(DirFsError::NotADirectory),
        }
    }

    /// Read a byte range from a file path (library helper; no FUSE session required).
    pub fn read_at_path(&self, path: &str, offset: u64, size: u32) -> Result<Vec<u8>, DirFsError> {
        let ino = self.lookup_path(path).ok_or(DirFsError::NotFound)?;
        self.read_at_ino(ino, offset, size)
    }

    fn read_at_ino(&self, ino: u64, offset: u64, size: u32) -> Result<Vec<u8>, DirFsError> {
        let node = self.nodes.get(&ino).ok_or(DirFsError::NotFound)?;
        match &node.kind {
            NodeKind::Dir { .. } => Err(DirFsError::IsDirectory),
            NodeKind::File {
                size: file_size,
                chunks,
                ..
            } => {
                let mut cache = self.prefetch.lock().unwrap_or_else(|e| e.into_inner());
                Ok(read_entries_cached(
                    chunks,
                    *file_size,
                    &self.source,
                    offset,
                    size,
                    &mut cache,
                    ino,
                )?)
            }
        }
    }

    fn alloc_ino(&mut self) -> u64 {
        let ino = self.next_ino;
        self.next_ino = self.next_ino.saturating_add(1);
        ino
    }

    /// Ensure every prefix of `path` exists as a directory; return the inode of `path`.
    /// If `path` already exists as a dir, update mode when `mode` is provided via overwrite.
    fn ensure_dir_path(&mut self, path: &str, mode: u32) -> u64 {
        let path = path.trim_matches('/');
        if path.is_empty() {
            // Root: optionally bump mode if an explicit Dir "" were ever allowed (it isn't).
            if let Some(Node {
                kind: NodeKind::Dir { mode: m, .. },
                ..
            }) = self.nodes.get_mut(&ROOT_INO)
            {
                *m = mode;
            }
            return ROOT_INO;
        }
        let mut parent = ROOT_INO;
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        for (i, seg) in segments.iter().enumerate() {
            let is_last = i + 1 == segments.len();
            parent =
                self.ensure_child_dir(parent, seg, if is_last { mode } else { SYNTH_DIR_MODE });
        }
        parent
    }

    fn ensure_child_dir(&mut self, parent_ino: u64, name: &str, mode: u32) -> u64 {
        // Look up existing child (copy ino to end the immutable borrow before mutating).
        let existing = self
            .nodes
            .get(&parent_ino)
            .and_then(|node| match &node.kind {
                NodeKind::Dir { children, .. } => children.get(name).copied(),
                NodeKind::File { .. } => None,
            });
        if let Some(child) = existing {
            if let Some(Node {
                kind: NodeKind::Dir { mode: m, .. },
                ..
            }) = self.nodes.get_mut(&child)
            {
                // Prefer explicitly requested mode when called for the leaf Dir entry.
                *m = mode;
            }
            return child;
        }

        let ino = self.alloc_ino();
        self.nodes.insert(
            ino,
            Node {
                parent: parent_ino,
                kind: NodeKind::Dir {
                    children: BTreeMap::new(),
                    mode,
                },
            },
        );
        if let Some(Node {
            kind: NodeKind::Dir { children, .. },
            ..
        }) = self.nodes.get_mut(&parent_ino)
        {
            children.insert(name.to_string(), ino);
        }
        ino
    }

    fn insert_file(
        &mut self,
        path: &str,
        mode: u32,
        size: u64,
        mtime_secs: u64,
        chunks: Vec<IndexEntry>,
    ) {
        let path = path.trim_matches('/');
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert!(!segments.is_empty(), "file path must be non-empty");
        let (file_name, parents) = segments.split_last().unwrap();
        let mut parent = ROOT_INO;
        for seg in parents {
            parent = self.ensure_child_dir(parent, seg, SYNTH_DIR_MODE);
        }

        // Replace existing child with same name if present (should not happen for valid archives).
        let old_ino = self.nodes.get(&parent).and_then(|node| match &node.kind {
            NodeKind::Dir { children, .. } => children.get(*file_name).copied(),
            NodeKind::File { .. } => None,
        });
        if let Some(old) = old_ino {
            self.nodes.remove(&old);
        }

        let ino = self.alloc_ino();
        self.nodes.insert(
            ino,
            Node {
                parent,
                kind: NodeKind::File {
                    mode,
                    size,
                    mtime_secs,
                    chunks,
                },
            },
        );
        if let Some(Node {
            kind: NodeKind::Dir { children, .. },
            ..
        }) = self.nodes.get_mut(&parent)
        {
            children.insert((*file_name).to_string(), ino);
        }
        self.total_bytes = self.total_bytes.saturating_add(size);
        self.file_count = self.file_count.saturating_add(1);
    }

    fn attr_for(&self, ino: u64) -> Option<FileAttr> {
        let node = self.nodes.get(&ino)?;
        match &node.kind {
            NodeKind::Dir { children, mode } => {
                let child_inos: Vec<u64> = children.values().copied().collect();
                let mode = *mode;
                let subdir_count = child_inos
                    .iter()
                    .filter(|&&c| {
                        self.nodes
                            .get(&c)
                            .is_some_and(|n| matches!(n.kind, NodeKind::Dir { .. }))
                    })
                    .count() as u32;
                let nlink = 2u32.saturating_add(subdir_count);
                Some(FileAttr {
                    ino,
                    size: 0,
                    blocks: 0,
                    atime: self.atime,
                    mtime: self.ctime,
                    ctime: self.ctime,
                    crtime: UNIX_EPOCH,
                    kind: FileType::Directory,
                    perm: (mode & 0o7777) as u16,
                    nlink,
                    uid: self.uid,
                    gid: self.gid,
                    rdev: 0,
                    blksize: BLKSIZE,
                    flags: 0,
                })
            }
            NodeKind::File {
                mode,
                size,
                mtime_secs,
                ..
            } => {
                let mtime = secs_to_system_time(*mtime_secs);
                Some(FileAttr {
                    ino,
                    size: *size,
                    blocks: size.div_ceil(u64::from(BLKSIZE)),
                    atime: self.atime,
                    mtime,
                    ctime: mtime,
                    crtime: UNIX_EPOCH,
                    kind: FileType::RegularFile,
                    perm: (*mode & 0o7777) as u16,
                    nlink: 1,
                    uid: self.uid,
                    gid: self.gid,
                    rdev: 0,
                    blksize: BLKSIZE,
                    flags: 0,
                })
            }
        }
    }
}

fn secs_to_system_time(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

fn current_ids() -> (u32, u32) {
    #[cfg(unix)]
    {
        (unsafe { libc::geteuid() }, unsafe { libc::getegid() })
    }
    #[cfg(not(unix))]
    {
        (0, 0)
    }
}

fn open_wants_write(flags: i32) -> bool {
    let acc = flags & libc::O_ACCMODE;
    acc == libc::O_WRONLY
        || acc == libc::O_RDWR
        || flags & libc::O_APPEND != 0
        || flags & libc::O_TRUNC != 0
}

impl<S: ChunkSource + 'static> Filesystem for DirFs<S> {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let Some(node) = self.nodes.get(&parent) else {
            reply.error(ENOENT);
            return;
        };
        let NodeKind::Dir { children, .. } = &node.kind else {
            reply.error(ENOTDIR);
            return;
        };
        let Some(name_str) = name.to_str() else {
            reply.error(ENOENT);
            return;
        };
        let Some(&ino) = children.get(name_str) else {
            reply.error(ENOENT);
            return;
        };
        match self.attr_for(ino) {
            Some(attr) => reply.entry(&TTL, &attr, 0),
            None => reply.error(ENOENT),
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        match self.attr_for(ino) {
            Some(attr) => reply.attr(&TTL, &attr),
            None => reply.error(ENOENT),
        }
    }

    fn setattr(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        _size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        reply.error(EROFS);
    }

    fn mknod(
        &mut self,
        _req: &Request<'_>,
        _parent: u64,
        _name: &OsStr,
        _mode: u32,
        _umask: u32,
        _rdev: u32,
        reply: ReplyEntry,
    ) {
        reply.error(EROFS);
    }

    fn mkdir(
        &mut self,
        _req: &Request<'_>,
        _parent: u64,
        _name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        reply.error(EROFS);
    }

    fn unlink(
        &mut self,
        _req: &Request<'_>,
        _parent: u64,
        _name: &OsStr,
        reply: fuser::ReplyEmpty,
    ) {
        reply.error(EROFS);
    }

    fn rmdir(&mut self, _req: &Request<'_>, _parent: u64, _name: &OsStr, reply: fuser::ReplyEmpty) {
        reply.error(EROFS);
    }

    fn symlink(
        &mut self,
        _req: &Request<'_>,
        _parent: u64,
        _link_name: &OsStr,
        _target: &std::path::Path,
        reply: ReplyEntry,
    ) {
        reply.error(EROFS);
    }

    fn rename(
        &mut self,
        _req: &Request<'_>,
        _parent: u64,
        _name: &OsStr,
        _newparent: u64,
        _newname: &OsStr,
        _flags: u32,
        reply: fuser::ReplyEmpty,
    ) {
        reply.error(EROFS);
    }

    fn link(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _newparent: u64,
        _newname: &OsStr,
        reply: ReplyEntry,
    ) {
        reply.error(EROFS);
    }

    fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        let Some(node) = self.nodes.get(&ino) else {
            reply.error(ENOENT);
            return;
        };
        match &node.kind {
            NodeKind::File { .. } => {
                if open_wants_write(flags) {
                    reply.error(EROFS);
                } else {
                    reply.opened(0, 0);
                }
            }
            NodeKind::Dir { .. } => reply.error(EISDIR),
        }
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        if offset < 0 {
            reply.error(EINVAL);
            return;
        }
        match self.read_at_ino(ino, offset as u64, size) {
            Ok(data) => reply.data(&data),
            Err(DirFsError::NotFound) => reply.error(ENOENT),
            Err(DirFsError::IsDirectory) => reply.error(EISDIR),
            Err(DirFsError::NotADirectory) => reply.error(ENOTDIR),
            Err(DirFsError::Source(_)) => reply.error(libc::EIO),
        }
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _offset: i64,
        _data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        reply.error(EROFS);
    }

    fn flush(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _lock_owner: u64,
        reply: fuser::ReplyEmpty,
    ) {
        reply.ok();
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: fuser::ReplyEmpty,
    ) {
        reply.ok();
    }

    fn fsync(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _datasync: bool,
        reply: fuser::ReplyEmpty,
    ) {
        reply.ok();
    }

    fn opendir(&mut self, _req: &Request<'_>, ino: u64, _flags: i32, reply: ReplyOpen) {
        let Some(node) = self.nodes.get(&ino) else {
            reply.error(ENOENT);
            return;
        };
        match &node.kind {
            NodeKind::Dir { .. } => reply.opened(0, 0),
            NodeKind::File { .. } => reply.error(ENOTDIR),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let Some(node) = self.nodes.get(&ino) else {
            reply.error(ENOENT);
            return;
        };
        let (child_pairs, parent_ino) = match &node.kind {
            NodeKind::Dir { children, .. } => (
                children
                    .iter()
                    .map(|(n, &c)| (n.clone(), c))
                    .collect::<Vec<_>>(),
                node.parent,
            ),
            NodeKind::File { .. } => {
                reply.error(ENOTDIR);
                return;
            }
        };

        // Build flat entry list: . , .. , then children in BTree order.
        let mut entries: Vec<(u64, FileType, String)> = Vec::with_capacity(2 + child_pairs.len());
        entries.push((ino, FileType::Directory, ".".into()));
        entries.push((parent_ino, FileType::Directory, "..".into()));
        for (name, child_ino) in child_pairs {
            let kind = match self.nodes.get(&child_ino).map(|n| &n.kind) {
                Some(NodeKind::Dir { .. }) => FileType::Directory,
                Some(NodeKind::File { .. }) => FileType::RegularFile,
                None => continue,
            };
            entries.push((child_ino, kind, name));
        }

        for (i, (entry_ino, kind, name)) in entries.into_iter().enumerate().skip(offset as usize) {
            let next = (i + 1) as i64;
            if reply.add(entry_ino, next, kind, name) {
                break;
            }
        }
        reply.ok();
    }

    fn releasedir(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _flags: i32,
        reply: fuser::ReplyEmpty,
    ) {
        reply.ok();
    }

    fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: ReplyStatfs) {
        let blocks = self.total_bytes.div_ceil(u64::from(BLKSIZE));
        let files = self.nodes.len() as u64;
        reply.statfs(
            blocks, // blocks
            0,      // bfree
            0,      // bavail
            files,  // files
            0,      // ffree
            BLKSIZE, 255, // namelen
            BLKSIZE,
        );
    }

    fn setxattr(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _name: &OsStr,
        _value: &[u8],
        _flags: i32,
        _position: u32,
        reply: fuser::ReplyEmpty,
    ) {
        reply.error(EROFS);
    }

    fn removexattr(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _name: &OsStr,
        reply: fuser::ReplyEmpty,
    ) {
        reply.error(EROFS);
    }

    fn access(&mut self, _req: &Request<'_>, ino: u64, mask: i32, reply: fuser::ReplyEmpty) {
        if !self.nodes.contains_key(&ino) {
            reply.error(ENOENT);
            return;
        }
        if mask & libc::W_OK != 0 {
            reply.error(EACCES);
            return;
        }
        reply.ok();
    }

    fn create(
        &mut self,
        _req: &Request<'_>,
        _parent: u64,
        _name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: fuser::ReplyCreate,
    ) {
        reply.error(EROFS);
    }

    fn fallocate(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _offset: i64,
        _length: i64,
        _mode: i32,
        reply: fuser::ReplyEmpty,
    ) {
        reply.error(EROFS);
    }

    fn copy_file_range(
        &mut self,
        _req: &Request<'_>,
        _ino_in: u64,
        _fh_in: u64,
        _offset_in: i64,
        _ino_out: u64,
        _fh_out: u64,
        _offset_out: i64,
        _len: u64,
        _flags: u32,
        reply: ReplyWrite,
    ) {
        reply.error(EROFS);
    }
}
