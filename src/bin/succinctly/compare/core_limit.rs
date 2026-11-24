//! No child of a comparison run leaves a core dump behind (#7891).
//!
//! Some probes end a tool with a fatal signal on purpose: jq 1.7.1 on `strftime/1` and
//! `strflocaltime/1` (features), on `array-set-too-huge-index`, and both tools on the
//! `mem-limit-*` rows under `ulimit -v` (robustness). Where `/proc/sys/kernel/core_pattern` is a
//! pipe (WSL's crash capture, systemd-coredump, apport) each of those handed a dump to the handler,
//! hundreds of megabytes for the allocation rows: the capture ran inside the row's wall time and
//! filled the disk of a shared machine.
//!
//! The run therefore lowers its own `RLIMIT_CORE` to exactly 1 byte, soft and hard, before it
//! spawns anything. A resource limit is inherited across `fork` and kept across `execve`, so every
//! child has it, and every process a child starts in turn: the tools, the `bash` wrappers of the
//! robustness rows, `/usr/bin/time`, `valgrind`, and the sweep and fuzz scripts with the tools
//! they run. The hard limit keeps any of them from raising it again.
//!
//! The value has to be 1, not 0: the kernel does not apply a limit of 0 to what it feeds a piped
//! handler, and aborts the dump only for a limit of exactly 1 (core(5), "Piping core dumps to a
//! program"); a file `core_pattern` writes nothing for either. A shell's `ulimit -c 1` cannot set
//! it, its unit being a block of 512 or 1024 bytes. `tests/common/no_core_dump.rs` (#7869) sets the
//! same limit for the test suite's deliberate aborts.
//!
//! It is set in this process instead of in each child's `pre_exec` because a `Command` with a
//! `pre_exec` closure is started by `fork` and `exec` where the others are started by
//! `posix_spawn`, and the timed rows start their clock before the spawn: a per-child hook would
//! have changed what the performance dimension measures. Nothing a child prints, its exit status
//! and the signal that ends it are unchanged; only what the kernel does after the signal is.
//!
//! Not covered: macOS's ReportCrash, which writes its report whatever the limit is. Platforms
//! without `setrlimit` are left as they were.

/// The limit in bytes, soft and hard.
#[cfg(unix)]
const CORE_LIMIT: libc::rlim_t = 1;

/// Lower this process's `RLIMIT_CORE` to 1 byte, soft and hard, for it and every process it starts.
///
/// A refusal (a hard limit that is already 0 cannot be raised to 1) is reported on stderr and is
/// not an error: the run is as it was before.
#[cfg(unix)]
pub fn apply() {
    if let Err(e) = set() {
        eprintln!(
            "note: RLIMIT_CORE could not be set to 1 byte ({e}); a probe that ends on a signal \
             may leave a core dump"
        );
    }
}

#[cfg(not(unix))]
pub fn apply() {}

#[cfg(unix)]
#[allow(unsafe_code)] // one `setrlimit(2)` call, reading only a local
fn set() -> std::io::Result<()> {
    let one = libc::rlimit {
        rlim_cur: CORE_LIMIT,
        rlim_max: CORE_LIMIT,
    };
    // SAFETY: `setrlimit` reads only the `rlimit` it is given.
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &one) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// What the harness's tests share to end a child on a signal without leaving a dump: the limit
/// is set by [`apply`], the one mechanism a run uses, in a copy of the test binary, never in the
/// test binary itself, where it could not be raised again and would hide from the test below
/// whether `apply` set it.
#[cfg(all(test, unix))]
pub(crate) mod testing {
    use super::super::harness::Tool;

    /// This process's `RLIMIT_CORE`, soft and hard, in bytes.
    #[allow(unsafe_code)] // one `getrlimit(2)` call, writing only into a local
    pub(crate) fn core_limit() -> (libc::rlim_t, libc::rlim_t) {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `getrlimit` writes only into the `rlimit` it is given.
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut limit) }, 0);
        (limit.rlim_cur, limit.rlim_max)
    }

    /// This test binary, re-run for the one test `test` of the module `module_path!()` names. The
    /// test's path is taken from the module's, so it follows the module if that moves: an
    /// `--exact` name that matches nothing runs no test and exits 0.
    pub(crate) fn rerun(module: &str, test: &str) -> Tool {
        let name = format!(
            "{}::{test}",
            module.split_once("::").map_or(module, |(_, rest)| rest)
        );
        Tool {
            name: "self".into(),
            program: std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            prefix: ["--exact", &name, "--nocapture", "--test-threads=1"]
                .map(String::from)
                .to_vec(),
        }
    }

    /// End this process on a signal, as a tool that crashes does, leaving no dump: `abort` under
    /// the 1-byte limit of [`apply`](super::apply). Where that limit is refused (a hard limit
    /// that is already 0, which a piped `core_pattern` handler ignores) it is `SIGKILL`
    /// instead, which is never dumped and is as much a death by signal to the caller.
    pub(crate) fn die_on_a_signal_without_a_dump() -> ! {
        super::apply();
        if core_limit() == (super::CORE_LIMIT, super::CORE_LIMIT) {
            std::process::abort();
        }
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &std::process::id().to_string()])
            .status();
        // Not reached unless `kill` could not be run: fail the caller instead of dumping.
        std::process::exit(101)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::exec::exec_env;
    use super::super::harness::Tool;
    use super::super::robust::{classify, Class};
    use super::testing::core_limit;
    use std::path::Path;
    use std::process::Command;

    const ROLE: &str = "COMPARE_CORE_LIMIT_ROLE";

    fn this_test() -> Tool {
        super::testing::rerun(
            module_path!(),
            "a_harness_child_that_aborts_ran_under_a_core_limit_of_one_byte",
        )
    }

    /// A child the harness's own spawn helper starts after [`apply`](super::apply) runs under a
    /// limit of exactly 1 byte, soft and hard, and a child that then aborts is still read as a
    /// crash with its signal and its output: the limit changes no class.
    ///
    /// Three processes, because the limit cannot be raised again and so must not be set in the
    /// test binary the other tests run in: this one starts a `harness` copy of itself, which calls
    /// `apply` and runs a `tool` copy through `exec_env`; the `tool` prints its limit and aborts.
    ///
    /// Where the hard limit is already below 1 byte (a shell after `ulimit -c 0`) it cannot be
    /// raised to 1: `apply` prints its note and the limit stays what it was, which the child must
    /// then inherit unchanged. A file `core_pattern` writes no dump under it either, but a piped
    /// handler ignores a limit of 0, so the `tool` does not abort there: the test would otherwise
    /// leave the dump this module exists to prevent.
    #[test]
    fn a_harness_child_that_aborts_ran_under_a_core_limit_of_one_byte() {
        match std::env::var(ROLE).ok().as_deref() {
            Some("tool") => {
                let (soft, hard) = core_limit();
                println!("core-limit soft={soft} hard={hard}");
                if (soft, hard) == (1, 1) {
                    std::process::abort();
                }
            }
            Some("harness") => {
                let before = core_limit();
                super::apply();
                let expected = if before.1 >= 1 { (1, 1) } else { before };
                assert_eq!(core_limit(), expected);
                let env = [(ROLE.to_string(), "tool".to_string())];
                let e = exec_env(&this_test(), &[], "", Path::new("."), &env).unwrap();
                // libtest prints `test <name> ... ` in front of it, on the same line.
                let out = String::from_utf8_lossy(&e.stdout);
                let line = format!("core-limit soft={} hard={}\n", expected.0, expected.1);
                assert!(out.contains(&line), "{out}");
                if expected == (1, 1) {
                    assert_eq!((e.exit, e.signal), (None, Some(libc::SIGABRT)));
                    assert_eq!(classify(&e), Class::Crash);
                } else {
                    assert_eq!((e.exit, e.signal), (Some(0), None));
                }
                println!("harness-saw-the-limit");
            }
            _ => {
                let tool = this_test();
                let out = Command::new(&tool.program)
                    .args(&tool.prefix)
                    .env(ROLE, "harness")
                    .output()
                    .unwrap();
                let text = String::from_utf8_lossy(&out.stdout);
                assert!(
                    out.status.success() && text.contains("harness-saw-the-limit"),
                    "{text}\n{}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
        }
    }
}
