//! #7857: started with SIGPIPE blocked in its signal mask (the disposition at its default),
//! `succinctly yq`'s write to a closed pipe ends the run with SIGPIPE, as yq v4.53.3's does: Go's
//! runtime unblocks SIGPIPE before it raises it for `EPIPE` on descriptors 1 and 2. It exited 1 with
//! `succinctly: failed to flush output: ...` and `Error: ...` instead, and a diagnostic to a closed
//! standard error exited 1. Every row was captured from yq v4.53.3 on macOS arm64. Unix only:
//! Windows has no SIGPIPE.

#![cfg(unix)]
#![allow(unsafe_code)] // STYLE-0004: `pre_exec` making the closed pipe, and blocking SIGPIPE, in the child.

use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};

use anyhow::Result;

/// SIGPIPE's number on every Unix this binary builds for.
const SIGPIPE: i32 = 13;

/// How a run ended: its signal or its exit status (`(None, Some(code))`), and the length of its
/// standard output.
type Ended = (Option<i32>, Option<i32>, usize);

/// The signal or the exit status (`(None, Some(code))`) and the length of standard output of
/// `succinctly yq <args>`, with SIGPIPE at its default (`Command` resets it in the child) and blocked
/// in the mask the process starts with, and standard error (`to_stderr`) or standard output a pipe
/// whose reader is gone.
///
/// The pipe is made in the child, after `fork`: one made here could have its read end inherited by
/// a process another test thread forks in the moment before the descriptor is marked close-on-exec
/// (macOS has no `pipe2`), and a pipe with a reader somewhere does not fail the write.
fn run(to_stderr: bool, args: &[&str]) -> Result<Ended> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command
        .arg("yq")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let target = if to_stderr {
        libc::STDERR_FILENO
    } else {
        libc::STDOUT_FILENO
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
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGPIPE);
            let e = libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
            if e != 0 {
                return Err(std::io::Error::from_raw_os_error(e));
            }
            Ok(())
        });
    }
    let output = command.output()?;
    Ok((
        output.status.signal(),
        output.status.code(),
        output.stdout.len(),
    ))
}

/// A closed standard output, a small result written at exit and a large one alike, and a
/// diagnostic to a closed standard error, from the evaluation (`error`) or from the parser, die of
/// SIGPIPE; a run that writes nothing to the closed descriptor ends as it would have.
#[test]
fn yq_dies_of_sigpipe_when_it_was_inherited_blocked_7857() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let doc = dir.path().join("big.yaml");
    let body: String = (0..100_000).map(|i| format!("- {i}\n")).collect();
    std::fs::write(&doc, body)?;
    let doc = doc.display().to_string();
    let rows: [(bool, &[&str], Ended); 8] = [
        (false, &["-n", "\"x\""], (Some(SIGPIPE), None, 0)),
        (false, &[".[]", &doc], (Some(SIGPIPE), None, 0)),
        (false, &["-o=json", ".", &doc], (Some(SIGPIPE), None, 0)),
        (true, &["-n", "error(\"x\")"], (Some(SIGPIPE), None, 0)),
        (true, &["-n", ".a |"], (Some(SIGPIPE), None, 0)),
        (true, &[".", "missing.yaml"], (Some(SIGPIPE), None, 0)),
        (true, &["-n", "\"x\""], (None, Some(0), 2)),
        (true, &[".[]", &doc], (None, Some(0), 588_890)),
    ];
    for (to_stderr, args, want) in rows {
        assert_eq!(run(to_stderr, args)?, want, "stderr={to_stderr} {args:?}");
    }
    Ok(())
}
