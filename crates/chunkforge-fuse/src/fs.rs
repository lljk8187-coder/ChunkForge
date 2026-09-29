//! Read-only single-blob FUSE [`Filesystem`]: root dir ino=1, file ino=2.

use crate::read::read_range;
use chunkforge_index::Index;
use chunkforge_store::ChunkSource;
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen,
    ReplyStatfs, ReplyWrite, Request, TimeOrNow,
};
use libc::{EACCES, EINVAL, EISDIR, ENOENT, ENOTDIR, EROFS};
use std::ffi::OsStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Root directory inode (FUSE convention).
pub const ROOT_INO: u64 = 1;
/// Single virtual regular-file inode.
pub const FILE_INO: u64 = 2;

const TTL: Duration = Duration::from_secs(1);
const BLKSIZE: u32 = 512;

/// Read-only single-blob filesystem backed by a `.cfidx` [`Index`] + [`ChunkSource`].
///
/// Mount point layout:
/// ```text
/// <mountpoint>/
///   <name>     # regular file, size = index.total_size, ino=2
/// ```
pub struct BlobFs<S: ChunkSource> {
    index: Index,
    source: S,
    name: String,
    uid: u32,
    gid: u32,
    atime: SystemTime,
    mtime: SystemTime,
    ctime: SystemTime,
}

impl<S: ChunkSource> BlobFs<S> {
    /// Build a FS presenting `index` via `source` under file name `name`.
    ///
    /// `name` should be a single path component (no `/`). Ownership defaults to
    /// the current process uid/gid when available.
    pub fn new(index: Index, source: S, name: impl Into<String>) -> Self {
        let (uid, gid) = current_ids();
        let now = SystemTime::now();
        Self {
            index,
            source,
            name: name.into(),
            uid,
            gid,
            atime: now,
            mtime: now,
            ctime: now,
        }
    }

    /// Override ownership shown in getattr.
    pub fn with_owner(mut self, uid: u32, gid: u32) -> Self {
        self.uid = uid;
        self.gid = gid;
        self
    }

    /// File name presented under the mount root.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Logical blob size (`index.total_size`).
    pub fn total_size(&self) -> u64 {
        self.index.total_size
    }

    /// Borrow the index.
    pub fn index(&self) -> &Index {
        &self.index
    }

    /// Read a byte range from the logical blob (no FUSE session required).
    ///
    /// Used by unit tests and by [`Filesystem::read`].
    pub fn read_at(
        &self,
        offset: u64,
        size: u32,
    ) -> Result<Vec<u8>, chunkforge_store::SourceError> {
        read_range(&self.index, &self.source, offset, size)
    }

    fn root_attr(&self) -> FileAttr {
        FileAttr {
            ino: ROOT_INO,
            size: 0,
            blocks: 0,
            atime: self.atime,
            mtime: self.mtime,
            ctime: self.ctime,
            crtime: UNIX_EPOCH,
            kind: FileType::Directory,
            perm: 0o555,
            nlink: 2,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: BLKSIZE,
            flags: 0,
        }
    }

    fn file_attr(&self) -> FileAttr {
        let size = self.index.total_size;
        FileAttr {
            ino: FILE_INO,
            size,
            blocks: size.div_ceil(u64::from(BLKSIZE)),
            atime: self.atime,
            mtime: self.mtime,
            ctime: self.ctime,
            crtime: UNIX_EPOCH,
            kind: FileType::RegularFile,
            perm: 0o444,
            nlink: 1,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: BLKSIZE,
            flags: 0,
        }
    }

    fn name_matches(&self, name: &OsStr) -> bool {
        name == OsStr::new(self.name.as_str())
    }
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

/// True if open flags request write access (WRONLY/RDWR/APPEND/TRUNC).
fn open_wants_write(flags: i32) -> bool {
    let acc = flags & libc::O_ACCMODE;
    acc == libc::O_WRONLY
        || acc == libc::O_RDWR
        || flags & libc::O_APPEND != 0
        || flags & libc::O_TRUNC != 0
}

impl<S: ChunkSource + 'static> Filesystem for BlobFs<S> {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        if parent != ROOT_INO {
            reply.error(ENOENT);
            return;
        }
        if self.name_matches(name) {
            reply.entry(&TTL, &self.file_attr(), 0);
        } else {
            reply.error(ENOENT);
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        match ino {
            ROOT_INO => reply.attr(&TTL, &self.root_attr()),
            FILE_INO => reply.attr(&TTL, &self.file_attr()),
            _ => reply.error(ENOENT),
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
        match ino {
            FILE_INO => {
                if open_wants_write(flags) {
                    reply.error(EROFS);
                } else {
                    reply.opened(0, 0);
                }
            }
            ROOT_INO => reply.error(EISDIR),
            _ => reply.error(ENOENT),
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
        if ino != FILE_INO {
            reply.error(if ino == ROOT_INO { EISDIR } else { ENOENT });
            return;
        }
        if offset < 0 {
            reply.error(EINVAL);
            return;
        }
        match self.read_at(offset as u64, size) {
            Ok(data) => reply.data(&data),
            Err(_) => {
                // Missing/corrupt chunk → I/O error to the reader.
                reply.error(libc::EIO);
            }
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
        if ino == ROOT_INO {
            reply.opened(0, 0);
        } else if ino == FILE_INO {
            reply.error(ENOTDIR);
        } else {
            reply.error(ENOENT);
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
        if ino != ROOT_INO {
            reply.error(ENOTDIR);
            return;
        }
        let entries: [(u64, FileType, &str); 3] = [
            (ROOT_INO, FileType::Directory, "."),
            (ROOT_INO, FileType::Directory, ".."),
            (FILE_INO, FileType::RegularFile, self.name.as_str()),
        ];
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
        let blocks = self.index.total_size.div_ceil(u64::from(BLKSIZE));
        reply.statfs(
            blocks, // blocks
            0,      // bfree
            0,      // bavail
            2,      // files (root + blob)
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
        if ino != ROOT_INO && ino != FILE_INO {
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
