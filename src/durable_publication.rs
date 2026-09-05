//! Parent-directory synchronization for the crash-safe publication recipe.
//!
//! RFC 112 separates three properties of a publication: replacement atomicity controls what concurrent readers
//! observe, synchronization requests durable persistence, and advisory locks coordinate cooperating writers.
//! Synchronizing the staged file alone does not persist the *directory entry* produced by the replacement, so
//! the recipe ends with an explicit directory synchronization step. This module owns that one step for compiler-host
//! code, which runs before any user program exists and therefore cannot call the generated `std.fs` library.
//!
//! The boundary exists because the step is spelled differently per host and getting it wrong fails in a way that is
//! easy to misread. On Unix a plain read handle to the directory is enough. On Windows two separate conditions must
//! both hold, and missing either produces an identical `ERROR_ACCESS_DENIED` that looks like a permissions problem
//! rather than a wrong-API problem. Centralizing the call keeps that knowledge in one place and stops each new
//! durable-write site from rediscovering it.

use std::fs::File;
use std::io;
use std::path::Path;

/// Request that a directory's contents be durably persisted, so a completed rename into it survives a crash.
///
/// Callers invoke this as the final step of the publication recipe, after the staged file has been synchronized and
/// atomically renamed over its destination. A failure here means the replacement may not survive a crash; it does not
/// mean the replacement failed, because the rename has already completed by this point. Callers map the returned
/// [`io::Error`] into their own error type rather than this module choosing one for them.
pub(crate) fn sync_directory(directory: &Path) -> io::Result<()> {
    open_directory_for_sync(directory)?.sync_all()
}

/// Open a directory handle suitable for synchronization on Unix hosts.
///
/// A plain read handle accepts `fsync`, so no extra access mode or flag is required.
#[cfg(unix)]
fn open_directory_for_sync(directory: &Path) -> io::Result<File> {
    File::open(directory)
}

/// Open a directory handle suitable for synchronization on Windows hosts.
///
/// Two conditions must hold together, and each fails with the same `ERROR_ACCESS_DENIED` when missing, which is why
/// they are documented here rather than left to the reader. `FILE_FLAG_BACKUP_SEMANTICS` is required for `CreateFile`
/// to return a handle to a directory at all — without it, a directory simply cannot be opened, which is why the
/// ordinary [`File::open`] used on Unix fails immediately. `GENERIC_WRITE` is then required for `FlushFileBuffers`
/// to be accepted on that handle; a read-only directory handle opens successfully and only fails at the flush.
#[cfg(windows)]
fn open_directory_for_sync(directory: &Path) -> io::Result<File> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    /// `FILE_FLAG_BACKUP_SEMANTICS` — permits `CreateFile` to return a directory handle.
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    /// `GENERIC_WRITE` — the access right `FlushFileBuffers` requires of its handle.
    const GENERIC_WRITE: u32 = 0x4000_0000;

    OpenOptions::new()
        .access_mode(GENERIC_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(directory)
}

/// Refuse directory synchronization on hosts that cannot express it.
///
/// RFC 112 requires that a host unable to honor a requested guarantee return a typed filesystem error rather than
/// silently weakening the operation, so this arm reports the limitation instead of returning `Ok` and leaving the
/// caller believing durability was requested.
#[cfg(not(any(unix, windows)))]
fn open_directory_for_sync(directory: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "this host cannot synchronize a directory, so publication durability cannot be requested for {}",
            directory.display()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use super::sync_directory;

    /// Exercise the full recipe tail: stage, synchronize, rename, then synchronize the parent directory.
    ///
    /// This is the case that failed on native Windows before the host-specific open was introduced, and it passes on
    /// every supported host because the step is genuinely available on all of them.
    #[test]
    fn synchronizes_a_directory_after_an_atomic_replacement() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let staged = directory.path().join(".receipt.json.tmp");
        let published = directory.path().join("receipt.json");

        let mut file = fs::File::create(&staged)?;
        file.write_all(b"{}\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&staged, &published)?;

        sync_directory(directory.path())?;

        assert!(
            published.exists(),
            "the published file should remain after synchronization"
        );
        Ok(())
    }

    /// Synchronizing a directory that does not exist reports an error rather than silently succeeding.
    #[test]
    fn reports_an_error_for_a_missing_directory() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let missing = directory.path().join("absent");

        assert!(
            sync_directory(&missing).is_err(),
            "a missing directory cannot be synchronized and must not report success"
        );
        Ok(())
    }

    /// A directory holding an unpublished staged file is still synchronizable.
    ///
    /// Publication failure paths synchronize before cleaning up a staged sibling, so this shape must not error.
    #[test]
    fn synchronizes_a_directory_containing_a_staged_file() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        fs::write(directory.path().join(".partial.tmp"), b"partial")?;

        sync_directory(directory.path())?;
        Ok(())
    }
}
