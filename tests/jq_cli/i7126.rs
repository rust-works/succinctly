//! #7126: a standard input that cannot be read is an empty one that ends the run with exit 2.
//!
//! When standard input is a descriptor that was not opened for reading (`jq . 0>>file`, the write end of
//! a pipe) or is a directory (`jq . < /`), jq 1.7.1 reads it as empty, says nothing, and exits 2 once the
//! run is over, whatever the program left: `-e`'s 4, `input`'s 5. A program that never reads standard
//! input (`-n 1`, `-n halt`, `halt_error`) is unaffected. Every row was captured from `/usr/bin/jq` 1.7.1
//! under the same shell redirections and is the same for all three shapes.
//!
//! Not covered: a closed descriptor (`jq . 0<&-`, which jq also exits 2 on). Rust's runtime reopens
//! descriptor 0 on `/dev/null` before `main` runs, so a closed standard input cannot be told from an
//! empty one (recorded in the limitations page).

#![cfg(unix)]

use std::process::Command;

use anyhow::Result;

/// `(program arguments, stdout, stderr, exit status)`.
const ROWS: &[(&[&str], &str, &str, i32)] = &[
    (&["."], "", "", 2),
    (&["-R", "."], "", "", 2),
    (&["-n", "input"], "", "jq: error (at <stdin>:0): break\n", 2),
    (&["-n", "[inputs]"], "[]\n", "", 2),
    (&["-e", "."], "", "", 2),
    (&["-s", "."], "[]\n", "", 2),
    (&["-c", ".a"], "", "", 2),
    (
        &["--seq", "."],
        "",
        "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0\n",
        2,
    ),
    (&["-n", "1"], "1\n", "", 0),
    (&["-n", "[inputs] | length"], "0\n", "", 2),
    (&["-r", ".a"], "", "", 2),
    (&[".", "--arg", "x", "1"], "", "", 2),
    (&["-n", "halt"], "", "", 0),
    (
        &["-n", "input | halt"],
        "",
        "jq: error (at <stdin>:0): break\n",
        2,
    ),
    (&["-n", "\"x\" | halt_error"], "", "x", 5),
    (
        &["-n", "input, error(\"boom\")"],
        "",
        "jq: error (at <stdin>:0): break\n",
        2,
    ),
    (&["--stream", "."], "", "", 2),
    (&["-c", "--stream", "-n", "[inputs]"], "[]\n", "", 2),
    (&["--stream-errors", "."], "", "", 2),
];

/// `sh -c 'exec "$0" "$@" REDIRECTION'`: the binary runs with descriptor 0 as the redirection makes it.
/// `0>&1` is the write end of the pipe the test reads the output from.
fn run(redirection: &str, args: &[&str]) -> Result<(String, String, Option<i32>)> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("exec \"$0\" \"$@\" {redirection}"))
        .arg(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    ))
}

#[test]
fn a_standard_input_that_cannot_be_read_is_empty_and_exits_2_as_in_jq_7126() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let appended = dir.path().join("w.json");
    std::fs::write(&appended, b"{\"a\":1}\n")?;
    // A write-only file past the windowed routes' 4 MiB threshold, which they decline for it.
    let big = dir.path().join("big.json");
    std::fs::write(&big, "{\"a\":1}\n".repeat(700_000))?;
    for redirection in [
        format!("0>>{}", appended.display()),
        format!("0>>{}", big.display()),
        "0>&1".to_string(),
        "0</".to_string(),
    ] {
        for &(args, stdout, stderr, status) in ROWS {
            let (out, err, code) = run(&redirection, args)?;
            assert_eq!(
                (out.as_str(), err.as_str(), code),
                (stdout, stderr, Some(status)),
                "{args:?} with {redirection}"
            );
        }
    }
    Ok(())
}

/// A standard input that is empty and readable (`< /dev/null`) is unchanged: exit 0, or `-e`'s 4.
#[test]
fn an_empty_readable_standard_input_is_not_a_failure_7126() -> Result<()> {
    for (args, status) in [(&["."][..], 0), (&["-e", "."], 4), (&["-n", "[inputs]"], 0)] {
        let (_, err, code) = run("0</dev/null", args)?;
        assert_eq!((err.as_str(), code), ("", Some(status)), "{args:?}");
    }
    Ok(())
}

/// A closed standard input reads as empty and exits as an empty one does: Rust reopens descriptor 0 on
/// `/dev/null` before `main`, so jq's exit 2 for it is not reproducible (#7126, recorded divergence).
#[test]
fn a_closed_standard_input_is_indistinguishable_from_an_empty_one_7126() -> Result<()> {
    let (out, err, code) = run("0<&-", &["."])?;
    assert_eq!((out.as_str(), err.as_str(), code), ("", "", Some(0)));
    Ok(())
}

/// A `-` the stream reader never gets to does not end the run with exit 2: jq reads standard input
/// when its reader reaches the source, so `--stream -n input f.json -` stops after the file's first
/// event and exits 0, where `[inputs]` goes on to the failed `-` and exits 2. The stream and raw
/// routes note the failure when they reach the source, not when they open it (#7126).
#[test]
fn a_failed_stdin_is_noted_when_the_stream_reader_reaches_it_7126() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("f.json");
    std::fs::write(&file, b"{\"a\":2}")?;
    let appended = dir.path().join("w.json");
    std::fs::write(&appended, b"")?;
    let file = file.display().to_string();
    for (args, stdout, status) in [
        (
            vec!["--stream", "-n", "input", file.as_str(), "-"],
            "[\n  [\n    \"a\"\n  ],\n  2\n]\n",
            0,
        ),
        (
            vec!["--stream", "-n", "[inputs]", file.as_str(), "-"],
            "[\n  [\n    [\n      \"a\"\n    ],\n    2\n  ],\n  [\n    [\n      \"a\"\n    ]\n  ]\n]\n",
            2,
        ),
    ] {
        let (out, err, code) = run(&format!("0>>{}", appended.display()), &args)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout, "", Some(status)),
            "{args:?}"
        );
    }
    Ok(())
}
