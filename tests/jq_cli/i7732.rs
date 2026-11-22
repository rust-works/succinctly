//! #7732: a process started with SIGPIPE ignored keeps it ignored, as jq does.
//!
//! jq never sets the disposition. A parent that ignores SIGPIPE (a systemd unit, Python's
//! `subprocess` with `restore_signals=False`, `trap '' PIPE`) hands that on through `exec`, so jq's
//! write to a pipe whose reader has gone fails `EPIPE`: jq carries on, then reports `jq: error:
//! writing output failed: Broken pipe` and exits 2, and a failed write to standard error is ignored
//! like any other. `succinctly jq` set SIGPIPE to its default at the start of every run (#5547) and
//! died of it (status 141 under a shell) on every row below.
//!
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq` on macOS arm64, `~/.local/bin/jq` on
//! Linux x86_64) and jq 1.8.2 (`/opt/homebrew/bin/jq`, macOS arm64), which agree on all of them, with
//! the reader of the closed pipe gone before the run starts. With SIGPIPE inherited at its default
//! the three tools die of it (#5547, `i5547.rs`). yq v4.53.3 dies of SIGPIPE whatever it inherited,
//! and so does `succinctly yq`. Unix only: Windows has no SIGPIPE.

#![cfg(unix)]

use std::os::unix::process::ExitStatusExt;
use std::process::{ChildStdin, Command, Stdio};

use anyhow::Result;

/// SIGPIPE's number on every Unix this binary builds for.
const SIGPIPE: i32 = 13;

/// jq's report of the failed write, in jq 1.7.1 and 1.8.2 on both platforms.
const BROKEN: &str = "jq: error: writing output failed: Broken pipe\n";

/// The write end of a pipe whose reader has exited: a write to it fails `EPIPE` (or raises SIGPIPE).
fn dead_pipe() -> Result<ChildStdin> {
    let mut reader = Command::new("true").stdin(Stdio::piped()).spawn()?;
    let write_end = reader.stdin.take().expect("piped");
    reader.wait()?;
    Ok(write_end)
}

/// Which of the run's output descriptors is a pipe with no reader.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Closed {
    Stdout,
    Stderr,
    Both,
}

struct Run {
    code: Option<i32>,
    signal: Option<i32>,
    stdout: Vec<u8>,
    stderr: String,
}

/// `program <args>`, started with SIGPIPE ignored when `ignored` (through `sh`'s `trap '' PIPE`,
/// which `exec` keeps) and at its default otherwise (`std::process::Command` resets it in the child).
fn run_program(program: &str, ignored: bool, closed: Closed, args: &[&str]) -> Result<Run> {
    let mut command = if ignored {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("trap '' PIPE; exec \"$0\" \"$@\"")
            .arg(program);
        command
    } else {
        Command::new(program)
    };
    command.args(args).stdin(Stdio::null());
    if matches!(closed, Closed::Stdout | Closed::Both) {
        command.stdout(Stdio::from(dead_pipe()?));
    }
    if matches!(closed, Closed::Stderr | Closed::Both) {
        command.stderr(Stdio::from(dead_pipe()?));
    }
    let output = command.output()?;
    Ok(Run {
        code: output.status.code(),
        signal: output.status.signal(),
        stdout: output.stdout,
        stderr: String::from_utf8(output.stderr)?,
    })
}

/// `succinctly <args>`; see [`run_program`].
fn run(ignored: bool, closed: Closed, args: &[&str]) -> Result<Run> {
    run_program(env!("CARGO_BIN_EXE_succinctly"), ignored, closed, args)
}

/// A JSON file the `--stream` and whole-file rows read: `[0,1,...,99999]`.
fn big_array_file(dir: &tempfile::TempDir) -> Result<String> {
    let path = dir.path().join("big.json");
    let body: Vec<String> = (0..100_000).map(|i| i.to_string()).collect();
    std::fs::write(&path, format!("[{}]\n", body.join(",")))?;
    Ok(path.display().to_string())
}

/// Standard output closed: every route that writes reports the failed write and exits 2; a run that
/// writes nothing to it keeps its status (`error`, `halt_error`: 5), as jq's `fclose` finds nothing
/// left to flush. Whatever went to standard error first comes first, as in jq.
#[test]
fn a_closed_stdout_is_jqs_failed_write_when_sigpipe_is_inherited_ignored_7732() -> Result<()> {
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
        (vec!["-n", "1"], 2, BROKEN.into()),
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
            vec!["-n", "1|debug"],
            2,
            format!("[\"DEBUG:\",1]\n{BROKEN}"),
        ),
        (vec!["-n", "\"x\"|stderr"], 2, format!("x{BROKEN}")),
    ];
    for (args, code, stderr) in &rows {
        let mut argv = vec!["jq"];
        argv.extend(args.iter().copied());
        let got = run(true, Closed::Stdout, &argv)?;
        assert_eq!(
            (got.signal, got.code, got.stderr.as_str()),
            (None, Some(*code), stderr.as_str()),
            "{args:?}"
        );
    }
    Ok(())
}

/// The `jq` and `sjq` multi-call aliases go the same way.
#[test]
fn the_aliases_keep_an_inherited_ignored_sigpipe_too_7732() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for alias in ["sjq", "jq"] {
        let link = dir.path().join(alias);
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_succinctly"), &link)?;
        let got = run_program(
            &link.display().to_string(),
            true,
            Closed::Stdout,
            &["-n", "range(100000)"],
        )?;
        assert_eq!(
            (got.signal, got.code, got.stderr.as_str()),
            (None, Some(2), BROKEN),
            "{alias}"
        );
    }
    Ok(())
}

/// Standard error closed: a diagnostic that cannot be written is dropped and the run ends as it
/// would have, with all of its output (jq ignores a failed `fprintf(stderr, ...)`).
#[test]
fn a_closed_stderr_is_ignored_when_sigpipe_is_inherited_ignored_7732() -> Result<()> {
    let rows: [(&[&str], i32, &str); 7] = [
        (&["-n", "-c", "[range(3)]"], 0, "[0,1,2]\n"),
        (&["-n", "1, error(\"x\")"], 5, "1\n"),
        (&["-n", "error(\"x\")"], 5, ""),
        (&["-n", "1, (\"bye\\n\"|halt_error)"], 5, "1\n"),
        (&["-n", "1|debug"], 0, "1\n"),
        (&["-n", "\"x\"|stderr"], 0, "\"x\"\n"),
        (&["-e", "-n", "null"], 1, "null\n"),
    ];
    for (args, code, stdout) in rows {
        let mut argv = vec!["jq"];
        argv.extend_from_slice(args);
        let got = run(true, Closed::Stderr, &argv)?;
        assert_eq!(
            (
                got.signal,
                got.code,
                String::from_utf8(got.stdout)?.as_str()
            ),
            (None, Some(code), stdout),
            "{args:?}"
        );
    }
    // A large output still goes out whole.
    let got = run(true, Closed::Stderr, &["jq", "-n", "range(100000)"])?;
    assert_eq!((got.code, got.stdout.len()), (Some(0), 588_890));
    Ok(())
}

/// Both closed: the failed write to standard output still sets exit 2; a run that writes nothing
/// there keeps its status.
#[test]
fn both_closed_keep_jqs_statuses_when_sigpipe_is_inherited_ignored_7732() -> Result<()> {
    let rows: [(&[&str], i32); 5] = [
        (&["-n", "range(100000)"], 2),
        (&["-n", "1|debug"], 2),
        (&["-n", "1, error(\"x\")"], 2),
        (&["-n", "error(\"x\")"], 5),
        (&["-n", "\"bye\\n\"|halt_error"], 5),
    ];
    for (args, code) in rows {
        let mut argv = vec!["jq"];
        argv.extend_from_slice(args);
        let got = run(true, Closed::Both, &argv)?;
        assert_eq!((got.signal, got.code), (None, Some(code)), "{args:?}");
    }
    Ok(())
}

/// SIGPIPE inherited at its default still ends the run with it (#5547), through the same harness.
#[test]
fn a_default_sigpipe_still_ends_the_run_7732() -> Result<()> {
    let got = run(false, Closed::Stdout, &["jq", "-n", "range(100000)"])?;
    assert_eq!(
        (got.signal, got.code, got.stderr.as_str()),
        (Some(SIGPIPE), None, "")
    );
    Ok(())
}

/// `succinctly yq` dies of SIGPIPE on a closed standard output whatever it inherited, as yq v4.53.3
/// does (Go raises SIGPIPE for `EPIPE` on descriptors 1 and 2).
#[test]
fn yq_dies_of_sigpipe_even_when_it_was_inherited_ignored_7732() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let doc = dir.path().join("big.yaml");
    let body: String = (0..100_000).map(|i| format!("- {i}\n")).collect();
    std::fs::write(&doc, body)?;
    let doc = doc.display().to_string();
    for ignored in [false, true] {
        let got = run(ignored, Closed::Stdout, &["yq", ".[]", &doc])?;
        assert_eq!(
            (got.signal, got.stderr.as_str()),
            (Some(SIGPIPE), ""),
            "ignored={ignored}"
        );
    }
    Ok(())
}

/// The route that writes a whole-file result straight from the document's index lost the write's
/// own error (`Error: write error`, exit 1), so a closed pipe there was not jq's failed write, and
/// neither was a full disk (#7214's report, which jq 1.7.1 on Linux x86_64 gives as
/// `jq: error: writing output failed: No space left on device`, exit 2).
#[cfg(target_os = "linux")]
#[test]
fn the_whole_file_route_reports_a_full_disk_as_jq_does_7732() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let big = big_array_file(&dir)?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["jq", ".", &big])
        .stdin(Stdio::null())
        .stdout(std::fs::OpenOptions::new().write(true).open("/dev/full")?)
        .output()?;
    assert_eq!(
        (
            output.status.code(),
            String::from_utf8(output.stderr)?.as_str()
        ),
        (
            Some(2),
            "jq: error: writing output failed: No space left on device\n"
        )
    );
    Ok(())
}
