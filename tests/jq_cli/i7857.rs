//! #7857: started with SIGPIPE blocked in its signal mask (the disposition at its default), jq's
//! write to a closed pipe fails `EPIPE`, as it does under an ignored disposition (#7732): jq reports
//! `jq: error: writing output failed: Broken pipe` and exits 2, and ignores a failed write to
//! standard error. `succinctly jq` took the die-of-SIGPIPE path instead, whose signal stays pending
//! under the block, and exited 1 with `succinctly: failed to flush output: ...` and `Error: ...`.
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`) on macOS arm64; jq 1.8.2 agrees on all of
//! them. Unix only: Windows has no SIGPIPE.

#![cfg(unix)]
#![allow(unsafe_code)] // STYLE-0004: `pre_exec` making the closed pipes, and blocking SIGPIPE, in the child.

use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};

use anyhow::Result;

/// What jq 1.7.1 writes to standard error when its write to standard output fails `EPIPE`.
const BROKEN: &str = "jq: error: writing output failed: Broken pipe\n";

/// Which of the run's output descriptors is a pipe with no reader.
#[derive(Clone, Copy, Debug)]
enum Closed {
    Stdout,
    Stderr,
    Both,
}

/// The status of a run (`None` for a signal, and the other way round), its standard output's
/// length and its standard error, whichever of the two is not closed.
#[derive(Debug, PartialEq)]
struct Run {
    signal: Option<i32>,
    code: Option<i32>,
    stdout_len: usize,
    stderr: String,
}

/// `program <args>` with SIGPIPE at its default (`Command` resets it in the child) and blocked in the
/// mask the process starts with, and `closed` a pipe whose reader is gone.
///
/// The pipes are made in the child, after `fork`: one made here could have its read end inherited
/// by a process another test thread forks in the moment before the descriptor is marked
/// close-on-exec (macOS has no `pipe2`), and a pipe with a reader somewhere does not fail the write.
fn run_program(program: &str, closed: Closed, args: &[&str]) -> Result<Run> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (stdout, stderr) = match closed {
        Closed::Stdout => (true, false),
        Closed::Stderr => (false, true),
        Closed::Both => (true, true),
    };
    // SAFETY: runs in the forked child before `exec` and calls only async-signal-safe functions on
    // descriptors and a signal set of its own; the pipes' write ends replace descriptors 1 and 2,
    // which `exec` keeps, as it keeps the one thread's signal mask.
    unsafe {
        command.pre_exec(move || {
            for (close, target) in [(stdout, libc::STDOUT_FILENO), (stderr, libc::STDERR_FILENO)] {
                if !close {
                    continue;
                }
                let mut fds = [0; 2];
                if libc::pipe(fds.as_mut_ptr()) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                libc::close(fds[0]);
                if libc::dup2(fds[1], target) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                libc::close(fds[1]);
            }
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
    Ok(Run {
        signal: output.status.signal(),
        code: output.status.code(),
        stdout_len: output.stdout.len(),
        stderr: String::from_utf8(output.stderr)?,
    })
}

/// `succinctly jq <args>`; see [`run_program`].
fn run(closed: Closed, args: &[&str]) -> Result<Run> {
    let mut argv = vec!["jq"];
    argv.extend_from_slice(args);
    run_program(env!("CARGO_BIN_EXE_succinctly"), closed, &argv)
}

fn exited(code: i32, stdout_len: usize, stderr: &str) -> Run {
    Run {
        signal: None,
        code: Some(code),
        stdout_len,
        stderr: stderr.to_string(),
    }
}

/// A JSON file the `--stream`, whole-file and multi-file rows read: `[0,1,...,99999]`.
fn big_array_file(dir: &tempfile::TempDir) -> Result<String> {
    let path = dir.path().join("big.json");
    let body: Vec<String> = (0..100_000).map(|i| i.to_string()).collect();
    std::fs::write(&path, format!("[{}]\n", body.join(",")))?;
    Ok(path.display().to_string())
}

/// Standard output closed: every route that writes reports the failed write and exits 2, a large
/// output that fills the buffer and a small one written at exit alike; a run that writes nothing to
/// it keeps its status. Whatever went to standard error first comes first, as in jq.
#[test]
fn a_closed_stdout_is_jqs_failed_write_when_sigpipe_is_inherited_blocked_7857() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let big = big_array_file(&dir)?;
    let rows: Vec<(Vec<&str>, i32, String)> = vec![
        (vec!["-n", "range(100000)"], 2, BROKEN.into()),
        (vec!["-n", "-r", "range(100000)|tostring"], 2, BROKEN.into()),
        (vec!["-n", "-j", "range(100000)|tostring"], 2, BROKEN.into()),
        (vec!["--stream", "-c", ".", &big], 2, BROKEN.into()),
        (vec!["--seq", "-n", "range(100000)"], 2, BROKEN.into()),
        (vec!["-n", "[range(300000)]"], 2, BROKEN.into()),
        (vec![".", &big], 2, BROKEN.into()),
        (vec!["-c", ".", &big, &big], 2, BROKEN.into()),
        (vec!["-n", "input", &big], 2, BROKEN.into()),
        (vec!["-n", "\"x\""], 2, BROKEN.into()),
        (vec!["-n", "-c", "[range(3)]"], 2, BROKEN.into()),
        (vec!["--unbuffered", "-n", "1,2"], 2, BROKEN.into()),
        (vec!["-e", "-n", "null"], 2, BROKEN.into()),
        (
            vec!["-n", "1, error(\"x\")"],
            2,
            format!("jq: error (at <unknown>): x\n{BROKEN}"),
        ),
        (
            vec!["-n", "error(\"x\")"],
            5,
            "jq: error (at <unknown>): x\n".into(),
        ),
        (vec!["-n", "\"bye\\n\"|halt_error"], 5, "bye\n".into()),
        (
            vec!["-n", "1, (\"bye\\n\"|halt_error)"],
            2,
            format!("bye\n{BROKEN}"),
        ),
        (
            vec!["-n", "\"x\"|debug"],
            2,
            format!("[\"DEBUG:\",\"x\"]\n{BROKEN}"),
        ),
        (vec!["-n", "\"x\"|stderr"], 2, format!("x{BROKEN}")),
        (vec!["-n", "halt"], 0, String::new()),
        (vec!["-n", "\"x\"|halt_error(1)"], 1, "x".into()),
    ];
    for (args, code, stderr) in &rows {
        assert_eq!(
            run(Closed::Stdout, args)?,
            exited(*code, 0, stderr),
            "{args:?}"
        );
    }
    Ok(())
}

/// The `jq` and `sjq` multi-call aliases go the same way.
#[test]
fn the_aliases_keep_an_inherited_blocked_sigpipe_too_7857() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for alias in ["sjq", "jq"] {
        let link = dir.path().join(alias);
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_succinctly"), &link)?;
        let got = run_program(
            &link.display().to_string(),
            Closed::Stdout,
            &["-n", "range(100000)"],
        )?;
        assert_eq!(got, exited(2, 0, BROKEN), "{alias}");
    }
    Ok(())
}

/// Standard error closed: a diagnostic that cannot be written is dropped and the run ends as it
/// would have, with all of its output (jq ignores a failed `fprintf(stderr, ...)`).
#[test]
fn a_closed_stderr_is_ignored_when_sigpipe_is_inherited_blocked_7857() -> Result<()> {
    let rows: [(&[&str], i32, usize); 9] = [
        (&["-n", "range(100000)"], 0, 588_890),
        (&["-n", "-c", "[range(3)]"], 0, 8),
        (&["-n", "1, error(\"x\")"], 5, 2),
        (&["-n", "error(\"x\")"], 5, 0),
        (&["-n", "1, (\"bye\\n\"|halt_error)"], 5, 2),
        (&["-n", "\"x\"|debug"], 0, 4),
        (&["-n", "\"x\"|stderr"], 0, 4),
        (&["-e", "-n", "null"], 1, 5),
        (&["-n", "\"x\"|halt_error(1)"], 1, 0),
    ];
    for (args, code, stdout_len) in rows {
        assert_eq!(
            run(Closed::Stderr, args)?,
            exited(code, stdout_len, ""),
            "{args:?}"
        );
    }
    Ok(())
}

/// Both closed: the failed write to standard output still sets exit 2; a run that writes nothing
/// there keeps its status.
#[test]
fn both_closed_keep_jqs_statuses_when_sigpipe_is_inherited_blocked_7857() -> Result<()> {
    let rows: [(&[&str], i32); 6] = [
        (&["-n", "range(100000)"], 2),
        (&["-n", "\"x\"|debug"], 2),
        (&["-n", "1, error(\"x\")"], 2),
        (&["-n", "error(\"x\")"], 5),
        (&["-n", "\"bye\\n\"|halt_error"], 5),
        (&["-n", "halt"], 0),
    ];
    for (args, code) in rows {
        assert_eq!(run(Closed::Both, args)?, exited(code, 0, ""), "{args:?}");
    }
    Ok(())
}
