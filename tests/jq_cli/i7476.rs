//! #7476: a standard error that cannot be written does not turn a diagnostic into a panic (exit 101).
//!
//! jq ignores a failed `fprintf(stderr, ...)` and exits with the status it would have had, and yq does
//! the same. Every row's status and standard output were captured from `/usr/bin/jq` 1.7.1 and
//! `/opt/homebrew/bin/jq` 1.8.2 (identical) with standard error on a file past `ulimit -f 0` with
//! `SIGXFSZ` ignored, the `EFBIG` shape every Unix has; the issue's `2>/dev/full` (`ENOSPC`, Linux
//! only) gave jq 1.7.1 the same statuses, and so does a closed descriptor. yq v4.53.3 exits 1 on each
//! of its rows under the same redirection.
//!
//! A pipe whose reader has gone is a fourth shape. `succinctly jq` and `succinctly yq` die of SIGPIPE
//! on it, as jq and yq do (#5547); every other subcommand keeps SIGPIPE ignored and sees `EPIPE`,
//! which is where the panic showed on macOS too (`json validate` exited 101).

#![cfg(unix)]

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;

/// How descriptor 2 is set up for the run.
#[derive(Clone, Copy, Debug)]
enum DeadStderr {
    /// A regular file that cannot grow: `ulimit -f 0` with `SIGXFSZ` ignored, so a write fails `EFBIG`.
    FileSizeLimit,
    /// `/dev/full`: a write fails `ENOSPC`.
    #[cfg(target_os = "linux")]
    DevFull,
    /// `2>&-`: closed (Rust's runtime reopens it on `/dev/null` before `main`).
    Closed,
    /// The write end of a pipe whose reader has exited: a write fails `EPIPE` (or raises SIGPIPE).
    BrokenPipe,
}

struct Run {
    stdout: String,
    /// What reached the standard error file, when it is one.
    stderr_bytes: usize,
    code: Option<i32>,
}

/// `succinctly <args>` with `stdin` and standard error set up as `how`.
fn run(how: DeadStderr, args: &[&str], stdin: &str) -> Result<Run> {
    let dir = tempfile::tempdir()?;
    let err_file = dir.path().join("stderr.txt");
    let binary = env!("CARGO_BIN_EXE_succinctly");
    let mut command = match how {
        DeadStderr::BrokenPipe => {
            let mut command = Command::new(binary);
            command.args(args);
            command
        }
        _ => {
            let redirect = match how {
                DeadStderr::FileSizeLimit => {
                    "trap '' XFSZ; ulimit -f 0; exec \"$0\" \"$@\" 2>\"$ERR_FILE\""
                }
                #[cfg(target_os = "linux")]
                DeadStderr::DevFull => "exec \"$0\" \"$@\" 2>/dev/full",
                DeadStderr::Closed => "exec \"$0\" \"$@\" 2>&-",
                DeadStderr::BrokenPipe => unreachable!(),
            };
            let mut command = Command::new("sh");
            command
                .arg("-c")
                .arg(redirect)
                .arg(binary)
                .args(args)
                .env("ERR_FILE", &err_file);
            command
        }
    };
    command
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    match how {
        DeadStderr::BrokenPipe => {
            // A child that exits at once leaves its stdin's write end with no reader.
            let mut reader = Command::new("true").stdin(Stdio::piped()).spawn()?;
            let write_end = reader.stdin.take().expect("piped");
            reader.wait()?;
            command.stderr(Stdio::from(write_end));
        }
        _ => {
            command.stderr(Stdio::null());
        }
    }
    let mut child = command.spawn()?;
    // The program may exit before reading its input; a write that finds the pipe closed is not a failure.
    let _ = child
        .stdin
        .take()
        .expect("piped")
        .write_all(stdin.as_bytes());
    let output = child.wait_with_output()?;
    let stderr_bytes = std::fs::metadata(&err_file).map_or(0, |m| m.len() as usize);
    Ok(Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr_bytes,
        code: output.status.code(),
    })
}

fn missing_path(name: &str) -> String {
    std::env::temp_dir()
        .join(format!(
            "succinctly-7476-no-such-dir-{}",
            std::process::id()
        ))
        .join(name)
        .display()
        .to_string()
}

/// `(args after "jq", stdin, jq's stdout, jq's status)`, each captured from jq 1.7.1 and 1.8.2.
fn jq_rows() -> Vec<(Vec<String>, &'static str, &'static str, i32)> {
    let owned = |args: &[&str]| args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();
    vec![
        (owned(&["-n", "error(\"x\")"]), "", "", 5),
        (owned(&["."]), "[1,", "", 5),
        (owned(&["."]), "1 [", "1\n", 5),
        (owned(&[".", &missing_path("missing.json")]), "", "", 2),
        (owned(&["bad syntax("]), "", "", 3),
        (owned(&["-n", "1,error(\"x\"),2"]), "", "1\n", 5),
        (owned(&["-n", "input"]), "", "", 5),
        (owned(&["-n", "\"x\"|debug"]), "", "\"x\"\n", 0),
        (owned(&["-n", "\"x\"|stderr"]), "", "\"x\"\n", 0),
        (owned(&["-n", "\"x\"|halt_error"]), "", "", 5),
        // A warning: jq ignores the truncated record and exits 0.
        (owned(&["--seq", "."]), "\u{1e}1\n\u{1e}[\n", "\u{1e}1\n", 0),
    ]
}

fn assert_jq_keeps_its_status(how: DeadStderr) -> Result<()> {
    for (args, stdin, stdout, code) in jq_rows() {
        let mut argv = vec!["jq"];
        argv.extend(args.iter().map(String::as_str));
        let got = run(how, &argv, stdin)?;
        assert_eq!(
            (got.stdout.as_str(), got.code, got.stderr_bytes),
            (stdout, Some(code), 0),
            "{how:?} {args:?}"
        );
    }
    Ok(())
}

#[test]
fn jq_keeps_its_status_when_stderr_is_past_the_file_size_limit_7476() -> Result<()> {
    assert_jq_keeps_its_status(DeadStderr::FileSizeLimit)
}

#[cfg(target_os = "linux")]
#[test]
fn jq_keeps_its_status_when_stderr_is_dev_full_7476() -> Result<()> {
    assert_jq_keeps_its_status(DeadStderr::DevFull)
}

#[test]
fn jq_keeps_its_status_when_stderr_is_closed_7476() -> Result<()> {
    assert_jq_keeps_its_status(DeadStderr::Closed)
}

/// Standard output past the limit too: jq's `writing output failed` cannot be written either, and the
/// status is still 2 (jq 1.7.1 and 1.8.2: `-n '1,error("x")'` and `-n '"x"|debug'`, both 2).
#[test]
fn jq_exits_2_when_stdout_and_stderr_both_fail_7476() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let out_file = dir.path().join("stdout.txt");
    let err_file = dir.path().join("stderr.txt");
    for filter in ["1,error(\"x\")", "\"x\"|debug", "1"] {
        let status = Command::new("sh")
            .arg("-c")
            .arg("trap '' XFSZ; ulimit -f 0; exec \"$0\" jq -n \"$1\" >\"$OUT_FILE\" 2>\"$ERR_FILE\"")
            .arg(env!("CARGO_BIN_EXE_succinctly"))
            .arg(filter)
            .env("OUT_FILE", &out_file)
            .env("ERR_FILE", &err_file)
            .stdin(Stdio::null())
            .status()?;
        assert_eq!(status.code(), Some(2), "{filter}");
        assert_eq!(std::fs::metadata(&out_file)?.len(), 0, "{filter}");
        assert_eq!(std::fs::metadata(&err_file)?.len(), 0, "{filter}");
    }
    Ok(())
}

/// A stderr pipe with no reader: jq dies of SIGPIPE at its first diagnostic (jq 1.7.1 and 1.8.2 under
/// the default disposition), and so does `succinctly jq` (#5547), before its buffered `1` is flushed.
#[test]
fn jq_dies_of_sigpipe_on_a_stderr_pipe_with_no_reader_7476() -> Result<()> {
    use std::os::unix::process::ExitStatusExt;
    let mut reader = Command::new("true").stdin(Stdio::piped()).spawn()?;
    let write_end = reader.stdin.take().expect("piped");
    reader.wait()?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["jq", "-n", "1,error(\"x\"),2"])
        .stdin(Stdio::null())
        .stderr(Stdio::from(write_end))
        .output()?;
    assert_eq!(output.status.signal(), Some(13));
    assert_eq!(output.stdout, b"");
    Ok(())
}

#[test]
fn yq_keeps_its_status_when_stderr_cannot_be_written_7476() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let doc = dir.path().join("a.yaml");
    std::fs::write(&doc, "a: 1\n")?;
    let doc = doc.display().to_string();
    let missing = missing_path("missing.yaml");
    let rows: [(&[&str], &str); 4] = [
        (&["yq", ".a |", &doc], ""),
        (&["yq", ".", &missing], ""),
        (&["yq", "error(\"x\")", &doc], ""),
        (&["yq", "."], "a: [\n"),
    ];
    for how in [DeadStderr::FileSizeLimit, DeadStderr::Closed] {
        for (args, stdin) in rows {
            let got = run(how, args, stdin)?;
            assert_eq!(
                (got.code, got.stderr_bytes),
                (Some(1), 0),
                "{how:?} {args:?}"
            );
        }
    }
    Ok(())
}

/// The other subcommands exit as they do with a working standard error. Nothing to capture from jq
/// here: the reference is the same command run with standard error on a pipe that is read.
#[test]
fn other_subcommands_keep_their_status_when_stderr_cannot_be_written_7476() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let bad_json = dir.path().join("bad.json");
    std::fs::write(&bad_json, "{\n")?;
    let bad_yaml = dir.path().join("bad.yaml");
    std::fs::write(&bad_yaml, "a: [\n")?;
    let bad_json = bad_json.display().to_string();
    let bad_yaml = bad_yaml.display().to_string();
    let missing = missing_path("missing.json");
    let rows: [&[&str]; 5] = [
        &["json", "validate", &bad_json],
        &["yaml", "validate", &bad_yaml],
        &["text", "validate", "utf8", &missing],
        &["jq-locate", &missing, "--offset", "1"],
        &["yq-locate", &bad_yaml, "--offset", "99"],
    ];
    for args in rows {
        let live = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .output()?;
        assert!(!live.stderr.is_empty(), "{args:?} reports something");
        assert_ne!(live.status.code(), Some(0), "{args:?} fails");
        let mut shapes = vec![DeadStderr::FileSizeLimit, DeadStderr::BrokenPipe];
        #[cfg(target_os = "linux")]
        shapes.push(DeadStderr::DevFull);
        for how in shapes {
            let got = run(how, args, "")?;
            assert_eq!(
                (got.stdout.as_bytes(), got.code, got.stderr_bytes),
                (live.stdout.as_slice(), live.status.code(), 0),
                "{how:?} {args:?}"
            );
        }
    }
    Ok(())
}
