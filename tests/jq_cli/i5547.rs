//! #5547: a reader that stops early (`succinctly jq ... | head -n1`) ends the process with
//! SIGPIPE, as it does jq's: no message on stderr and status 141 under a shell, not
//! `failed to flush output: Broken pipe` and exit 1. Unix only: Windows has no SIGPIPE.

#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

use anyhow::Result;

/// SIGPIPE's number on every Unix this binary builds for.
const SIGPIPE: i32 = 13;

/// Runs `succinctly <args>`, reads its first stdout line, closes the pipe and reports how the
/// process ended and what it said on stderr.
fn first_line_then_close(args: &[&str]) -> Result<(String, Option<i32>, Option<i32>, String)> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let mut first = String::new();
    BufReader::new(stdout).read_line(&mut first)?;
    // The reader is gone with the `BufReader`: the child's next write finds the pipe closed.
    let output = child.wait_with_output()?;
    Ok((
        first,
        output.status.code(),
        output.status.signal(),
        String::from_utf8(output.stderr)?,
    ))
}

#[test]
fn test_a_closed_stdout_pipe_ends_the_process_with_sigpipe_5547() -> Result<()> {
    for args in [
        vec!["jq", "-n", "range(10000000)"],
        vec!["jq", "-n", "repeat(1)"],
        vec!["jq", "-n", "[range(1000000)] | .[]"],
        vec!["jq", "-n", "-r", "range(10000000) | tostring"],
        vec!["jq", "-n", "-c", "range(10000000) | [., .]"],
        vec!["jq", "-n", "--seq", "range(10000000)"],
    ] {
        let filter = args.join(" ");
        let (first, code, signal, stderr) = first_line_then_close(&args)?;
        assert!(!first.trim_end().is_empty(), "{filter}: first line");
        assert_eq!(
            signal,
            Some(SIGPIPE),
            "{filter}: code={code:?} stderr={stderr:?}"
        );
        assert_eq!(stderr, "", "{filter}");
    }
    Ok(())
}

#[test]
fn test_a_reader_that_takes_everything_is_unaffected_5547() -> Result<()> {
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["jq", "-n", "-c", "[range(3)] | .[]"])
        .output()?;
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stdout)?, "0\n1\n2\n");
    Ok(())
}

#[test]
fn test_the_sjq_alias_dies_of_sigpipe_too_5547() -> Result<()> {
    use std::os::unix::process::CommandExt;

    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg0("sjq")
        .args(["-n", "range(10000000)"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let mut first = String::new();
    BufReader::new(stdout).read_line(&mut first)?;
    let output = child.wait_with_output()?;
    assert_eq!(first, "0\n");
    assert_eq!(output.status.signal(), Some(SIGPIPE));
    assert_eq!(output.stderr, b"");
    Ok(())
}
