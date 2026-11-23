//! #7847: `succinctly yq`'s write to a closed pipe ends the run with SIGPIPE every time, as yq
//! v4.53.3's does, on macOS too.
//!
//! The race is the one `tests/jq_cli/i7847.rs` describes: macOS handed the write's SIGPIPE to the
//! main thread, and the evaluation thread that wrote could end the process first, so `error("x")`
//! to a closed standard error now and then exited 1. It cannot be forced from outside, so the test
//! runs for a bounded time from several threads at once and can pass on an unfixed binary when the
//! race is never lost. Unix only: Windows has no SIGPIPE.

#![cfg(unix)]
#![allow(unsafe_code)] // STYLE-0004: `pre_exec` making the closed pipe in the child before `exec`.

use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Result;

/// SIGPIPE's number on every Unix this binary builds for.
const SIGPIPE: i32 = 13;

/// `succinctly yq -n <program>` with standard error (`to_stderr`) or standard output a pipe whose
/// reader is gone; SIGPIPE at its default, which `Command` resets in the child.
///
/// The pipe is made in the child, after `fork`: one made here could have its read end inherited by
/// a process another test thread forks in the moment before the descriptor is marked close-on-exec
/// (macOS has no `pipe2`), and a pipe with a reader somewhere does not fail the write.
fn status(to_stderr: bool, program: &str) -> Result<std::process::ExitStatus> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command
        .args(["yq", "-n", program])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let target = if to_stderr {
        libc::STDERR_FILENO
    } else {
        libc::STDOUT_FILENO
    };
    // SAFETY: runs in the forked child before `exec` and calls only async-signal-safe functions on
    // descriptors of its own; the pipe's write end replaces `target`, which `exec` keeps.
    unsafe {
        command.pre_exec(move || {
            let mut fds = [0; 2];
            if libc::pipe(fds.as_mut_ptr()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::close(fds[0]);
            if libc::dup2(fds[1], target) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::close(fds[1]);
            Ok(())
        });
    }
    Ok(command.status()?)
}

#[test]
fn a_write_to_a_closed_pipe_always_dies_of_sigpipe_under_contention_7847() -> Result<()> {
    const SHAPES: [(bool, &str); 2] = [(true, "error(\"x\")"), (false, "\"x\"")];
    let deadline = Instant::now() + Duration::from_millis(1000);
    let workers: Vec<_> = (0..6)
        .map(|_| {
            std::thread::spawn(move || -> Result<(usize, Vec<String>)> {
                let (mut runs, mut wrong) = (0, Vec::new());
                while Instant::now() < deadline || runs < SHAPES.len() {
                    let (to_stderr, program) = SHAPES[runs % SHAPES.len()];
                    let got = status(to_stderr, program)?;
                    if got.signal() != Some(SIGPIPE) {
                        wrong.push(format!("stderr={to_stderr} {program}: {got:?}"));
                    }
                    runs += 1;
                }
                Ok((runs, wrong))
            })
        })
        .collect();
    let (mut runs, mut wrong) = (0, Vec::new());
    for worker in workers {
        let (n, w) = worker.join().expect("worker")?;
        runs += n;
        wrong.extend(w);
    }
    assert!(
        wrong.is_empty(),
        "{} of {runs} runs: {wrong:?}",
        wrong.len()
    );
    Ok(())
}
