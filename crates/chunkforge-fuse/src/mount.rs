//! Mount helpers with [`MountOption::RO`] hard-coded.

use fuser::{Filesystem, MountOption, mount2};
use std::io;
use std::path::Path;

/// Build mount options that **always** include [`MountOption::RO`].
///
/// Callers may pass extra options (e.g. [`MountOption::AutoUnmount`],
/// [`MountOption::FSName`]); any attempt to request [`MountOption::RW`] is
/// stripped so the kernel-side mount stays read-only.
pub fn mount_options(extra: impl IntoIterator<Item = MountOption>) -> Vec<MountOption> {
    let mut opts = vec![MountOption::RO];
    for o in extra {
        match o {
            MountOption::RW => {
                // Phase 2: write mounts are out of scope; never honor RW.
            }
            MountOption::RO => {
                // already present
            }
            other => {
                if !opts.contains(&other) {
                    opts.push(other);
                }
            }
        }
    }
    opts
}

/// Mount any FUSE [`Filesystem`] at `mountpoint` with read-only options (blocks until unmount).
///
/// Always injects [`MountOption::RO`]. Requires a working FUSE stack (`fuse3`,
/// `/dev/fuse`) at runtime; library unit tests do not call this.
///
/// Accepts [`crate::BlobFs`] or [`crate::DirFs`] (or any other [`Filesystem`]).
pub fn mount_ro<FS>(
    fs: FS,
    mountpoint: impl AsRef<Path>,
    extra: impl IntoIterator<Item = MountOption>,
) -> io::Result<()>
where
    FS: Filesystem,
{
    let opts = mount_options(extra);
    debug_assert!(
        opts.contains(&MountOption::RO),
        "mount_options must always include RO"
    );
    debug_assert!(
        !opts.contains(&MountOption::RW),
        "mount_options must never include RW"
    );
    mount2(fs, mountpoint, &opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_options_always_ro_strips_rw() {
        let opts = mount_options([
            MountOption::RW,
            MountOption::FSName("chunkforge".into()),
            MountOption::AutoUnmount,
            MountOption::RO,
        ]);
        assert!(opts.contains(&MountOption::RO));
        assert!(!opts.contains(&MountOption::RW));
        assert!(opts.contains(&MountOption::FSName("chunkforge".into())));
        assert!(opts.contains(&MountOption::AutoUnmount));
        // RO appears exactly once
        assert_eq!(
            opts.iter().filter(|o| matches!(o, MountOption::RO)).count(),
            1
        );
    }
}
