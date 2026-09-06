//! Path forms that external host tooling can consume.
//!
//! Incan canonicalizes paths freely, and on Windows `fs::canonicalize` returns the extended-length `\\?\` form. That
//! form is correct for Rust's own file APIs but is not universally understood by the third-party executables this
//! compiler drives. Handing it to them fails in ways that name something other than the path, so the rule lives here
//! once rather than being rediscovered at each boundary.

use std::path::PathBuf;

/// The Windows extended-length path prefix, which `fs::canonicalize` prepends to every path it returns.
#[cfg(windows)]
const EXTENDED_LENGTH_PREFIX: &str = r"\\?\";

/// Render a path in the form external host tooling can consume.
///
/// On Windows, strips the extended-length prefix that `fs::canonicalize` adds. Two independent boundaries need this,
/// and both fail misleadingly without it:
///
/// - **Cargo manifests.** A `path` dependency carrying the prefix reads as `//?/C:\...` once separators are normalized,
///   which is not a path URL Cargo understands. The manifest fails to parse with a message about the dependency rather
///   than about the prefix.
/// - **The MSVC linker.** Given a prefixed path whose length exceeds `MAX_PATH`, `link.exe` fails to canonicalize it
///   and derives an *empty* filename, then reports the default object extension: `LNK1181: cannot open input file
///   '.obj'`. The same paths in ordinary drive form link successfully well past `MAX_PATH`, because that form goes
///   through Windows long-path support. The error names neither the file nor the prefix that caused it.
///
/// Only ordinary drive paths are unwrapped. A verbatim UNC path keeps its prefix, because there the prefix is
/// load-bearing rather than decoration. Off Windows this is the identity.
pub(crate) fn tool_visible_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let rendered = path.to_string_lossy();
        if let Some(remainder) = rendered.strip_prefix(EXTENDED_LENGTH_PREFIX) {
            let drive_letter_follows = remainder.as_bytes().get(1) == Some(&b':');
            if drive_letter_follows {
                return PathBuf::from(remainder.to_string());
            }
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::tool_visible_path;
    use std::path::PathBuf;

    /// A canonicalized drive path is the case both boundaries hit, and the prefix is pure decoration there.
    #[test]
    #[cfg(windows)]
    fn drive_paths_lose_the_extended_length_prefix() {
        assert_eq!(
            tool_visible_path(PathBuf::from(r"\\?\C:\dev\incan\target")),
            PathBuf::from(r"C:\dev\incan\target")
        );
    }

    /// On a verbatim UNC path the prefix carries meaning, so removing it would name a different location.
    #[test]
    #[cfg(windows)]
    fn verbatim_unc_paths_keep_their_prefix() {
        let unc = PathBuf::from(r"\\?\UNC\server\share\dir");
        assert_eq!(tool_visible_path(unc.clone()), unc);
    }

    /// Paths that never carried the prefix must survive untouched, on every host.
    #[test]
    fn ordinary_paths_are_unchanged() {
        let plain = PathBuf::from(if cfg!(windows) { r"C:\dev\incan" } else { "/dev/incan" });
        assert_eq!(tool_visible_path(plain.clone()), plain);
    }
}
