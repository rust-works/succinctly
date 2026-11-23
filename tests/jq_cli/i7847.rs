//! #7847: a write to a closed pipe ends the run with SIGPIPE every time, as in jq, on macOS too.
//!
//! macOS raises a write's SIGPIPE on the process, and the kernel hands such a signal to the first
//! thread that does not block it: the main thread, waiting for the evaluation thread that wrote.
//! The writer carried on past its `EPIPE`, and when it ended the process itself before the main
//! thread acted on the signal, the run exited normally: `debug` to a closed standard error exited
//! 0 now and then, `error("x")` exited 5, where jq 1.7.1 and 1.8.2 (single-threaded) always die of
//! SIGPIPE. The main thread now blocks SIGPIPE while the evaluation thread runs, so the writer takes
//! it (`src/bin/succinctly/sigpipe.rs` pins the masks themselves).
//!
//! The lost race cannot be forced from outside, so the first test runs the three shapes for a
//! bounded time from several threads at once, the contention under which the unfixed binary lost
//! it; it can pass on that binary when the race is never lost. Linux raises SIGPIPE on the writing
//! thread and always passed. Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`) and jq
//! 1.8.2 (`/opt/homebrew/bin/jq`) on macOS arm64, which agree. Unix only: Windows has no SIGPIPE.

#![cfg(unix)]
#![allow(unsafe_code)] // STYLE-0004: `pre_exec` making the closed pipe, and blocking SIGPIPE, in the child.

use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::Result;

/// SIGPIPE's number on every Unix this binary builds for.
const SIGPIPE: i32 = 13;

/// Which of the run's output descriptors is a pipe with no reader.
#[derive(Clone, Copy, Debug)]
enum Closed {
    Stdout,
    Stderr,
}

/// `succinctly jq <args>` with `closed` a pipe whose reader is gone and the other output captured,
/// SIGPIPE at its default (`Command` resets it in the child) and, when `blocked`, blocked in the
/// mask the process starts with.
///
/// The pipe is made in the child, after `fork`: one made here could have its read end inherited by
/// a process another test thread forks in the moment before the descriptor is marked close-on-exec
/// (macOS has no `pipe2`), and a pipe with a reader somewhere does not fail the write.
fn run(closed: Closed, blocked: bool, args: &[&str]) -> Result<Output> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command.arg("jq").args(args).stdin(Stdio::null());
    let target = match closed {
        Closed::Stdout => {
            command.stdout(Stdio::null()).stderr(Stdio::piped());
            libc::STDOUT_FILENO
        }
        Closed::Stderr => {
            command.stdout(Stdio::piped()).stderr(Stdio::null());
            libc::STDERR_FILENO
        }
    };
    // SAFETY: runs in the forked child before `exec` and calls only async-signal-safe functions on
    // descriptors and a signal set of its own; the pipe's write end replaces `target`, which `exec`
    // keeps, as it keeps the one thread's signal mask.
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
            if blocked {
                let mut set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::sigaddset(&mut set, libc::SIGPIPE);
                let e = libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
                if e != 0 {
                    return Err(std::io::Error::from_raw_os_error(e));
                }
            }
            Ok(())
        });
    }
    Ok(command.output()?)
}

/// Under contention from several threads, every run that writes to a closed pipe dies of SIGPIPE:
/// `debug` and `error` to standard error, a result to standard output.
#[test]
fn a_write_to_a_closed_pipe_always_dies_of_sigpipe_under_contention_7847() -> Result<()> {
    const SHAPES: [(Closed, &str); 3] = [
        (Closed::Stderr, "\"x\"|debug"),
        (Closed::Stderr, "error(\"x\")"),
        (Closed::Stdout, "\"x\""),
    ];
    let deadline = Instant::now() + Duration::from_millis(2000);
    let workers: Vec<_> = (0..6)
        .map(|_| {
            std::thread::spawn(move || -> Result<(usize, Vec<String>)> {
                let (mut runs, mut wrong) = (0, Vec::new());
                while Instant::now() < deadline || runs < SHAPES.len() {
                    let (closed, program) = SHAPES[runs % SHAPES.len()];
                    let status = run(closed, false, &["-n", program])?.status;
                    if status.signal() != Some(SIGPIPE) {
                        wrong.push(format!("{closed:?} {program}: {status:?}"));
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

/// A process started with SIGPIPE blocked keeps it blocked, as jq does: its write to a closed
/// standard error fails `EPIPE`, which jq ignores, and the run ends as it would have. The main
/// thread's own block (above) must not be lifted on the evaluation thread in that case.
#[test]
fn an_inherited_blocked_sigpipe_stays_blocked_7847() -> Result<()> {
    for (program, code, stdout) in [("\"x\"|debug", 0, "\"x\"\n"), ("error(\"x\")", 5, "")] {
        let got = run(Closed::Stderr, true, &["-n", program])?;
        assert_eq!(
            (
                got.status.signal(),
                got.status.code(),
                String::from_utf8(got.stdout)?.as_str()
            ),
            (None, Some(code), stdout),
            "{program}"
        );
    }
    Ok(())
}
