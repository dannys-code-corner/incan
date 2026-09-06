//! Process-tree containment shared by Oven's bounded child-process paths.
//!
//! Containment is a host capability, not a guarantee this module can provide everywhere. Unix has process groups, so
//! a bounded child and everything it spawns can be stopped together. Windows has an equivalent in Job Objects, but
//! reaching them requires Win32 FFI that this crate cannot contain while it forbids `unsafe`; that work is #1341 and
//! is scheduled for 0.7, where it can be an Incan-authored host component instead.
//!
//! Until then this module reports the gap rather than hiding it. Terminating a child on a host without containment
//! reaps the child and leaves its descendants running, and callers are told so, because two of them terminate
//! specifically to stop a runaway build consuming disk — something surviving descendants carry on doing.

use std::io;
use std::process::{Child, Command, ExitStatus};

#[cfg(unix)]
use std::process::Stdio;

/// Whether this host can stop a child's descendants alongside the child itself.
///
/// Unix establishes a process group before spawning and signals the whole group on termination. No other supported
/// host does yet, so termination there reaches the direct child only.
const HOST_CONTAINS_DESCENDANTS: bool = cfg!(unix);

/// Outcome of terminating a bounded child, including whether its descendants were contained.
///
/// A successful termination is not proof that the process tree stopped, which is why the two facts are returned
/// together rather than as a bare [`ExitStatus`] a caller could mistake for containment.
pub(crate) struct ProcessGroupTermination {
    /// Exit status of the direct child.
    pub(crate) status: ExitStatus,
    /// Whether the host also stopped the processes the child had already spawned.
    pub(crate) descendants_contained: bool,
}

impl ProcessGroupTermination {
    /// Return a diagnostic clause naming the containment gap, or `None` when the tree really did stop.
    ///
    /// Callers append this to the failure they are already reporting so every site describes the same limitation in
    /// the same words, rather than each inventing its own phrasing or omitting it.
    pub(crate) fn uncontained_note(&self) -> Option<&'static str> {
        if self.descendants_contained {
            return None;
        }
        Some(
            "this host cannot contain a terminated child's descendants, so processes it had already spawned may still be running and holding their output files open",
        )
    }
}

/// Put a child and all normally spawned descendants in an isolated process group where the host provides one.
///
/// This is a no-op on hosts without process groups. That is deliberate rather than overlooked: there is nothing to
/// establish before spawning on Windows, because a Job Object is created separately and the child is assigned to it
/// after it exists. [`terminate_process_group`] reports the resulting gap.
pub(crate) fn isolate_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        command.process_group(0);
    }
    #[cfg(not(unix))]
    {
        // Bind the parameter so the no-op is stated in code rather than surfacing as an unused-argument warning that
        // a reader has to interpret.
        let _ = command;
    }
}

/// Terminate and reap a bounded child, together with its process group where the host supports one.
///
/// On Unix, Cargo, Rustdoc, libtest, build scripts, compilers, and linkers inherit the isolated group unless they
/// explicitly create a new session, which none of the supported Oven child paths permit or require, so the whole tree
/// stops. GNU `kill` needs `--` before a negative process-group ID, while the BSD implementation shipped by macOS
/// rejects that separator.
///
/// On every other host only the direct child is terminated. The returned
/// [`ProcessGroupTermination::descendants_contained`] says which happened; callers must not infer containment from a
/// successful return.
pub(crate) fn terminate_process_group(child: &mut Child) -> io::Result<ProcessGroupTermination> {
    #[cfg(unix)]
    {
        let process_group = format!("-{}", child.id());
        let mut command = Command::new("/bin/kill");
        command.arg("-KILL");
        #[cfg(target_os = "linux")]
        command.arg("--");
        let status = command
            .arg(&process_group)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !status.success() && child.try_wait()?.is_none() {
            return Err(io::Error::other(format!(
                "failed to terminate process group {process_group}: {status}"
            )));
        }
    }
    #[cfg(not(unix))]
    child.kill()?;

    Ok(ProcessGroupTermination {
        status: child.wait()?,
        descendants_contained: HOST_CONTAINS_DESCENDANTS,
    })
}

#[cfg(all(test, unix))]
/// Return whether a process is still running rather than an unreaped zombie.
///
/// Terminating a process group necessarily reaps the direct child, but its descendants become children of the host
/// reaper. A normal host init reaps those immediately; a test container may run a non-reaping PID 1, leaving an
/// inert zombie that still answers `kill -0`. Capacity containment cares about executable descendants and their disk
/// activity, so a zombie is correctly considered stopped.
pub(crate) fn process_is_running(pid: u32) -> io::Result<bool> {
    let exists = Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success();
    if !exists {
        return Ok(false);
    }

    let status = Command::new("/bin/ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !status.status.success() {
        return Ok(false);
    }
    Ok(!String::from_utf8_lossy(&status.stdout).trim_start().starts_with('Z'))
}

#[cfg(test)]
mod tests {
    use super::HOST_CONTAINS_DESCENDANTS;

    /// The advertised containment capability must match the platform that actually implements it.
    ///
    /// This is the fact every caller's diagnostic depends on, so it is asserted rather than assumed. When #1341 adds
    /// Job Object containment this test is the first thing that should change.
    #[test]
    fn containment_capability_matches_the_host() {
        assert_eq!(
            HOST_CONTAINS_DESCENDANTS,
            cfg!(unix),
            "only Unix establishes a process group today; see #1341 for Windows Job Objects"
        );
    }
}
