//! #7214: a write to a standard output that is open but cannot be written ends the run with jq's
//! `jq: error: writing output failed: <strerror>` and exit 2.
//!
//! Rust's `std::io::Stdout` turns `EBADF` into success, so `succinctly jq . 1<file` (a descriptor opened
//! read-only) printed nothing and exited 0. Every row was captured from `/usr/bin/jq` 1.7.1 under the same
//! redirection.
//!
//! Not covered: a closed descriptor (`jq . >&-`, which jq also exits 2 on). Rust's runtime reopens descriptor 1 on
//! `/dev/null` before `main` runs, so a closed standard output cannot be told from `>/dev/null` (recorded in
//! the limitations page).

#![cfg(unix)]

use std::process::Command;

use anyhow::Result;

const FAILED: &str = "jq: error: writing output failed: Bad file descriptor\n";

/// `sh -c 'exec "$0" "$@" 1<file'`: the binary runs with descriptor 1 open for reading only.
fn run(stdin: &str, args: &[&str]) -> Result<(String, String, Option<i32>)> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("ro.txt");
    std::fs::write(&file, b"x")?;
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(format!("exec \"$0\" \"$@\" 1<{}", file.display()))
        .arg(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    {
        use std::io::Write;
        child
            .stdin
            .take()
            .expect("piped")
            .write_all(stdin.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    ))
}

#[test]
fn a_standard_output_that_cannot_be_written_is_reported_with_exit_2_as_in_jq_7214() -> Result<()> {
    for args in [
        &["."][..],
        &["-c", "."],
        &["-r", "."],
        &["--unbuffered", "."],
        &["-e", "false"],
    ] {
        let (out, err, code) = run("1\n", args)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            ("", FAILED, Some(2)),
            "{args:?}"
        );
    }
    Ok(())
}

#[test]
fn a_failed_write_after_a_runtime_error_still_reports_the_error_first_7214() -> Result<()> {
    let (_, err, code) = run("", &["-n", "1, error(\"x\")"])?;
    assert_eq!(
        (err.as_str(), code),
        (
            "jq: error (at <unknown>): x\njq: error: writing output failed: Bad file descriptor\n",
            Some(2)
        )
    );
    Ok(())
}

/// A run that writes nothing has no write to fail: exit 0, as in jq.
#[test]
fn a_run_with_no_output_does_not_fail_on_an_unwritable_standard_output_7214() -> Result<()> {
    let (_, err, code) = run("", &["-n", "empty"])?;
    assert_eq!((err.as_str(), code), ("", Some(0)));
    Ok(())
}

/// A big result fails on the buffer's first flush, not at the end, and is reported the same way.
#[test]
fn a_result_larger_than_the_buffer_is_reported_the_same_way_7214() -> Result<()> {
    let (_, err, code) = run("", &["-n", "range(100000)"])?;
    assert_eq!((err.as_str(), code), (FAILED, Some(2)));
    Ok(())
}

/// A closed standard output reads as `/dev/null`: Rust reopens descriptor 1 before `main`, so jq's exit 2 for
/// it is not reproducible (#7214, recorded divergence).
#[test]
fn a_closed_standard_output_is_indistinguishable_from_dev_null_7214() -> Result<()> {
    let output = Command::new("sh")
        .arg("-c")
        .arg("exec \"$0\" jq . >&-")
        .arg(env!("CARGO_BIN_EXE_succinctly"))
        .stdin(std::process::Stdio::null())
        .output()?;
    assert_eq!(output.status.code(), Some(0));
    Ok(())
}
