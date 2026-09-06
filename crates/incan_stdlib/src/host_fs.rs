//! Filesystem operations the host exposes only through per-platform APIs.
//!
//! `std.fs` is one API on every supported host, but several of the operations behind it are reached differently on
//! each. Incan has no conditional compilation, so a `.incn` source cannot carry a Unix branch and a Windows branch;
//! it imports this module instead, and the platform difference is resolved here in Rust. That keeps the standard
//! library's own sources platform-agnostic, which is also how they read best.

use std::fs::File;
use std::io;
use std::path::Path;

/// Take a blocking shared advisory lock on an open file.
///
/// Advisory locking became portable in Rust 1.89, which is below this workspace's minimum, so the platform split
/// that used to require `rustix::fs::flock` no longer exists.
pub fn lock_shared(file: &File) -> io::Result<()> {
    file.lock_shared()
}

/// Take a blocking exclusive advisory lock on an open file.
pub fn lock_exclusive(file: &File) -> io::Result<()> {
    file.lock()
}

/// Attempt a shared advisory lock, reporting contention rather than failing.
///
/// Returns `Ok(false)` when another holder owns the lock, which callers translate into an empty optional result.
/// Only genuine host failures surface as `Err`.
pub fn try_lock_shared(file: &File) -> io::Result<bool> {
    match file.try_lock_shared() {
        Ok(()) => Ok(true),
        Err(std::fs::TryLockError::WouldBlock) => Ok(false),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

/// Attempt an exclusive advisory lock, reporting contention rather than failing.
pub fn try_lock_exclusive(file: &File) -> io::Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(std::fs::TryLockError::WouldBlock) => Ok(false),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

/// Create a symbolic link at `link` pointing at `original`.
///
/// Unix creates one kind of symlink and does not care what it points at. Windows has two, and the kind is fixed
/// when the link is created rather than resolved when it is followed, so the target's kind is inspected first and a
/// directory link is created only for an existing directory. A link to a missing target becomes a file link, which
/// matches what a caller porting Unix code expects and is the only choice available without more information.
///
/// Windows additionally requires Developer Mode or elevation to create symlinks at all; without it the host refuses
/// with a permission error, which is reported unchanged rather than translated into something that hides the cause.
pub fn symlink(original: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(original, link)
    }

    #[cfg(windows)]
    {
        let target_is_directory = std::fs::metadata(original)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);
        if target_is_directory {
            std::os::windows::fs::symlink_dir(original, link)
        } else {
            std::os::windows::fs::symlink_file(original, link)
        }
    }
}

/// Report whether a path is the root of the filesystem holding it.
///
/// Unix answers this by comparing the path's device against its parent's: a differing device means the boundary is
/// here. The identity comparison is the Unix idiom for the question rather than the question itself, so Windows
/// answers it directly instead — a path is a filesystem root there when it has no parent once canonicalized, which
/// covers drive and UNC volume roots.
///
/// Known gap on Windows: a directory junction or a volume mounted into a folder is a filesystem boundary that this
/// does not report, because recognising one requires reading its reparse tag through Win32. Callers get `false`
/// for those rather than a wrong `true`.
pub fn is_mount_point(path: &Path) -> io::Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let Some(parent) = path.parent() else {
            return Ok(true);
        };
        let current = std::fs::metadata(path)?;
        let parent_metadata = std::fs::metadata(parent)?;
        Ok(current.dev() != parent_metadata.dev() || current.ino() == parent_metadata.ino())
    }

    #[cfg(windows)]
    {
        // Canonicalize so a relative path, a trailing separator, or a `.` component cannot make a root look nested.
        // The path must exist for the question to mean anything, and canonicalize reports that for us.
        let canonical = std::fs::canonicalize(path)?;
        Ok(canonical.parent().is_none())
    }
}

/// Byte counts describing the filesystem holding one path.
pub struct DiskUsageBytes {
    /// Total capacity of the filesystem.
    pub total: u64,
    /// Bytes currently allocated.
    pub used: u64,
    /// Bytes available to this caller, which may be less than the unallocated total under quotas or reservations.
    pub free: u64,
}

/// Read capacity information for the filesystem holding `path`.
///
/// Not yet implemented on Windows. The Win32 call that answers this, `GetDiskFreeSpaceExW`, is reachable only
/// through FFI, and no crate already in this workspace's dependency graph wraps it — so supporting it means either
/// admitting `unsafe` into a crate that ships inside every generated program, or adding a dependency, and that is a
/// decision for the maintainer rather than something to settle here. Reporting the gap plainly is better than
/// returning a plausible wrong number for a capacity check, which is the kind of value a caller acts on.
pub fn disk_usage(path: &Path) -> io::Result<DiskUsageBytes> {
    #[cfg(unix)]
    {
        let stats = rustix::fs::statvfs(path)?;
        let block_size = stats.f_bsize;
        let total = stats.f_blocks.saturating_mul(block_size);
        let used = stats.f_blocks.saturating_sub(stats.f_bfree).saturating_mul(block_size);
        let free = stats.f_bavail.saturating_mul(block_size);
        Ok(DiskUsageBytes { total, used, free })
    }

    #[cfg(windows)]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "std.fs disk usage is not yet implemented on Windows",
        ))
    }
}
