//! Keep a child that a test ends on purpose with a fatal signal (`abort()`, a stack overflow) from
//! leaving a core dump behind (#7869).
//!
//! `ulimit -c 0` is not enough on Linux: when `/proc/sys/kernel/core_pattern` starts with a pipe
//! (WSL's crash capture, systemd-coredump, apport) the kernel does not apply a limit of 0 to what it
//! feeds the handler, so every deliberate abort still handed a dump of the debug binary to it (about
//! 21 per `jq_cli_tests` run, 100 MB to 2.1 GB each, on a WSL box). `prctl(PR_SET_DUMPABLE, 0)` is
//! no help either: `execve` resets the flag for a normal, readable binary. What survives `execve`
//! and what the kernel does honour for a pipe is `RLIMIT_CORE` of exactly 1 (core(5), "Piping core
//! dumps to a program": the kernel aborts the dump when the limit is 1, the value it gives the
//! handler itself to catch a handler that crashes in turn); a file `core_pattern` takes the 1 as
//! "smaller than a page" and writes nothing either. The soft and the hard limit are both set, so
//! the child cannot raise it again. A shell's `ulimit -c 1` cannot do this: its unit is a block of
//! 512 or 1024 bytes, not a byte, and `ulimit -c 0` in a wrapper script would overwrite this limit
//! with 0, so a wrapper that runs a child through this helper must not set `-c` itself.
//!
//! Nothing under test changes: stdout, stderr and the terminating signal are what the tests read.
//! Included via `#[path = "common/no_core_dump.rs"]`, like its siblings in this directory.

#![cfg(unix)]
#![allow(unsafe_code)] // STYLE-0004: `pre_exec` setting `RLIMIT_CORE` in the child.
#![allow(dead_code)] // Each consumer uses a different subset.

use std::io;
use std::os::unix::process::CommandExt;
use std::process::Command;

/// Set `RLIMIT_CORE` to 1 byte, soft and hard, in `command`'s child between `fork` and `exec`.
///
/// Apply it to the command that runs the process to be killed (when that is a `sh -c` wrapper that
/// `exec`s the process, to the `sh`: the limit is inherited across both).
pub fn without_core_dump(command: &mut Command) -> &mut Command {
    // SAFETY: the closure runs in the forked child before `exec` and makes one `setrlimit` call, which
    // is async-signal-safe; it allocates nothing and takes no lock.
    unsafe {
        command.pre_exec(|| {
            let one = libc::rlimit {
                rlim_cur: 1,
                rlim_max: 1,
            };
            if libc::setrlimit(libc::RLIMIT_CORE, &one) == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The limit the helper sets reaches the child, soft and hard. `ulimit -c` of a POSIX `sh` counts
    /// blocks, so one byte reads as 0 there: a limit of 0 blocks that is not the unset "unlimited".
    #[test]
    fn the_child_runs_under_a_core_limit_of_one_byte() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("ulimit -c; ulimit -H -c");
        let output = without_core_dump(&mut command).output().expect("ran");
        assert!(output.status.success(), "{output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout), "0\n0\n");
    }
}
