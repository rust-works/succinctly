//! #7538: a standard output that cannot be written does not turn a report into a panic (exit 101).
//!
//! `json generate`, `dsv generate`, `yaml generate`, `jq-locate`, `yq-locate` and the
//! `--version`/`--build-configuration` reports printed through `std::println!`, which panics when
//! the write fails. They now end as `succinctly jq` ends a failed write to standard output (#7214):
//! `succinctly: error: writing output failed: <strerror>` on standard error and exit 2, where jq
//! 1.7.1 and 1.8.2 print `jq: error: writing output failed: File too large` and exit 2 for
//! `jq --version` and `jq --build-configuration` under the same `ulimit -f 0` with `SIGXFSZ`
//! ignored (captured from `/usr/bin/jq` and `/opt/homebrew/bin/jq`; the generators and locators are
//! succinctly's own, so there is no reference for them).
//!
//! A pipe whose reader has gone is a second shape: the run dies of SIGPIPE, as `succinctly jq` and
//! jq do (#5547), instead of panicking on `EPIPE`.

#![cfg(unix)]

use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

use anyhow::Result;

/// How descriptor 1 is set up for the run.
#[derive(Clone, Copy, Debug)]
enum DeadStdout {
    /// A regular file that cannot grow: `ulimit -f 0` with `SIGXFSZ` ignored, so a write fails `EFBIG`.
    FileSizeLimit,
    /// `/dev/full`: a write fails `ENOSPC`.
    #[cfg(target_os = "linux")]
    DevFull,
}

struct Run {
    stdout: Vec<u8>,
    stderr: String,
    code: Option<i32>,
}

fn run(how: DeadStdout, args: &[&str]) -> Result<Run> {
    let dir = tempfile::tempdir()?;
    let out_file = dir.path().join("stdout.txt");
    let redirect = match how {
        DeadStdout::FileSizeLimit => "trap '' XFSZ; ulimit -f 0; exec \"$0\" \"$@\" >\"$OUT_FILE\"",
        #[cfg(target_os = "linux")]
        DeadStdout::DevFull => "exec \"$0\" \"$@\" >/dev/full",
    };
    let output = Command::new("sh")
        .arg("-c")
        .arg(redirect)
        .arg(env!("CARGO_BIN_EXE_succinctly"))
        .args(args)
        .env("OUT_FILE", &out_file)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    Ok(Run {
        stdout: std::fs::read(&out_file).unwrap_or_default(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    })
}

/// A JSON document and a YAML one for the locators, in a directory that outlives the run.
fn inputs(dir: &std::path::Path) -> Result<(String, String)> {
    let json = dir.join("ok.json");
    std::fs::write(&json, "{\"a\":1}")?;
    let yaml = dir.join("ok.yaml");
    std::fs::write(&yaml, "a: 1\n")?;
    Ok((json.display().to_string(), yaml.display().to_string()))
}

/// Each report, as the arguments after `succinctly`.
fn reports(json: &str, yaml: &str) -> Vec<Vec<String>> {
    let owned = |args: &[&str]| args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();
    vec![
        owned(&["json", "generate", "1kb"]),
        owned(&["dsv", "generate", "1kb"]),
        owned(&["yaml", "generate", "1kb"]),
        owned(&["jq-locate", json, "--offset", "2"]),
        owned(&["jq-locate", json, "--offset", "2", "--format", "json"]),
        owned(&["yq-locate", yaml, "--offset", "3"]),
        owned(&["jq", "--version"]),
        owned(&["jq", "--build-configuration"]),
        owned(&["yq", "--build-configuration"]),
    ]
}

fn assert_reports_the_failed_write(how: DeadStdout) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (json, yaml) = inputs(dir.path())?;
    for args in reports(&json, &yaml) {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let result = run(how, &refs)?;
        assert!(
            !result.stderr.contains("panicked"),
            "{how:?} {args:?}: a panic message: {}",
            result.stderr
        );
        assert_eq!(result.code, Some(2), "{how:?} {args:?}: {}", result.stderr);
        assert!(
            result
                .stderr
                .starts_with("succinctly: error: writing output failed: "),
            "{how:?} {args:?}: stderr was {:?}",
            result.stderr
        );
        assert_eq!(
            result.stderr.matches("writing output failed").count(),
            1,
            "{how:?} {args:?}: reported more than once: {:?}",
            result.stderr
        );
        assert!(result.stdout.is_empty(), "{how:?} {args:?}: wrote output");
    }
    Ok(())
}

#[test]
fn reports_exit_2_when_stdout_is_past_the_file_size_limit_7538() -> Result<()> {
    assert_reports_the_failed_write(DeadStdout::FileSizeLimit)
}

#[cfg(target_os = "linux")]
#[test]
fn reports_exit_2_when_stdout_is_dev_full_7538() -> Result<()> {
    assert_reports_the_failed_write(DeadStdout::DevFull)
}

/// The same commands with a writable standard output succeed, so the rows above are not vacuous.
#[test]
fn the_same_reports_succeed_on_a_writable_stdout_7538() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (json, yaml) = inputs(dir.path())?;
    for args in reports(&json, &yaml) {
        let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .args(&args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .output()?;
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        assert!(!output.stdout.is_empty(), "{args:?}: no output");
        assert!(output.stderr.is_empty(), "{args:?}: wrote to stderr");
    }
    Ok(())
}

/// A pipe whose reader has gone: the run dies of SIGPIPE (`| head`), with no panic and no report.
#[test]
fn a_closed_stdout_pipe_ends_the_generators_quietly_7538() -> Result<()> {
    for args in [["json", "generate", "1mb"], ["dsv", "generate", "1mb"]] {
        let mut reader = Command::new("true").stdin(Stdio::piped()).spawn()?;
        let write_end = reader.stdin.take().expect("piped");
        reader.wait()?;
        let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::from(write_end))
            .stderr(Stdio::piped())
            .output()?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains("panicked"), "{args:?}: {stderr}");
        assert!(stderr.is_empty(), "{args:?}: reported on stderr: {stderr}");
        assert!(
            output.status.signal() == Some(libc::SIGPIPE) || output.status.code() == Some(1),
            "{args:?}: ended {:?}",
            output.status
        );
    }
    Ok(())
}
