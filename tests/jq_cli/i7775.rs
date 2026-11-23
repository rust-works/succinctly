//! #7775: under `--seq`, a program that reads its inputs with `input`/`inputs` read the whole stream
//! before its first record ran, so `-n 'first(inputs)'` took as long as `-n '[inputs]'`, where jq
//! reads a line at a time and stops at the records the program consumes. `succinctly jq` now walks
//! the stream as it reads it, a block at a time, as it does a main loop that can halt over a pipe, and
//! such a main loop over a file reads it a window at a time; none reports a fault past the records the
//! program reads, as jq never reaches one.
//!
//! The literal rows were captured from jq 1.7.1 (`/usr/bin/jq`) and jq 1.8.2
//! (`/opt/homebrew/bin/jq`), which agree on all of them: with the pipe held open after the first two
//! records, both answer at once for every row, and `-n '[inputs]'` waits for the pipe's end and
//! reports the record cut short after it.

#![cfg(unix)]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use crate::small_window;

/// (program arguments, stdout, stderr, exit status) over `RS {"a":1} LF RS {"a":2} LF`, with the pipe
/// held open after it.
const EARLY: &[(&[&str], &str, &str, i32)] = &[
    (
        &["--seq", "-c", "-n", "first(inputs)"],
        "\x1e{\"a\":1}\n",
        "",
        0,
    ),
    (&["--seq", "-c", "-n", "input"], "\x1e{\"a\":1}\n", "", 0),
    (&["--seq", "-c", "-n", "input | halt"], "", "", 0),
    (
        &["--seq", "-c", "-n", "limit(1; inputs)"],
        "\x1e{\"a\":1}\n",
        "",
        0,
    ),
    (
        &["--seq", "-c", "-n", "first(inputs | select(.a == 2))"],
        "\x1e{\"a\":2}\n",
        "",
        0,
    ),
    (
        &["--seq", "-c", "-n", "[limit(2; inputs)]"],
        "\x1e[{\"a\":1},{\"a\":2}]\n",
        "",
        0,
    ),
    (
        &["--seq", "-c", "-n", "input, input"],
        "\x1e{\"a\":1}\n\x1e{\"a\":2}\n",
        "",
        0,
    ),
    // The main loop, halting at its first record and at its second.
    (&["--seq", "-c", ".a, halt"], "\x1e1\n", "", 0),
    (
        &["--seq", "-c", "select(.a == 2) | ., halt"],
        "\x1e{\"a\":2}\n",
        "",
        0,
    ),
    (
        &["--seq", "-c", ".a | if . == 2 then halt_error else . end"],
        "\x1e1\n",
        "2\n",
        5,
    ),
];

/// How long a run that should answer at once may take; the pipe is held open past it.
const PATIENCE: Duration = Duration::from_secs(60);

/// A program that reads only its first records answers while the pipe it reads is still open, as jq
/// does: the run that read the whole stream first waited for the writer to close it.
#[test]
fn an_early_exit_answers_before_the_pipe_ends_7775() -> Result<()> {
    for &(args, stdout, stderr, code) in EARLY {
        let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .arg("jq")
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().expect("stdin is piped");
        stdin.write_all(b"\x1e{\"a\":1}\n\x1e{\"a\":2}\n")?;
        stdin.flush()?;
        let start = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if start.elapsed() > PATIENCE {
                child.kill()?;
                bail!("{args:?} still running {PATIENCE:?} after its records, with the pipe open");
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        // A record cut short, which the run never reads.
        let _ = stdin.write_all(b"\x1e[3\n");
        drop(stdin);
        let output = child.wait_with_output()?;
        assert_eq!(
            (
                String::from_utf8_lossy(&output.stdout).into_owned(),
                String::from_utf8_lossy(&output.stderr).into_owned(),
                status.code(),
            ),
            (stdout.to_string(), stderr.to_string(), Some(code)),
            "{args:?}"
        );
    }
    Ok(())
}

/// What a run answered, and whether it walked the stream as it read it.
type Answer = (Vec<u8>, String, i32, bool);

/// A file of its own for each run to log its routes to (`SUCCINCTLY_DEBUG_JQ_ROUTE_LOG`, #7688): the
/// tests of this file run at once, in one process.
fn route_log() -> std::path::PathBuf {
    static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let run = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "succinctly-7775-routes-{}-{run}",
        std::process::id()
    ))
}

fn command(args: &[&str], whole: bool, log: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command
        .arg("jq")
        .args(args)
        .env("LC_ALL", "C")
        .env("SUCCINCTLY_DEBUG_JQ_ROUTE_LOG", log)
        .env_remove("SUCCINCTLY_DEBUG_JQ_WHOLE_FILE");
    small_window::apply(&mut command);
    if whole {
        command.env("SUCCINCTLY_DEBUG_JQ_WHOLE_FILE", "1");
    }
    command
}

fn answer(output: std::process::Output, log: &Path) -> Answer {
    let walked = std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .any(|route| route == "seq-walk");
    let _ = std::fs::remove_file(log);
    (
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
        walked,
    )
}

/// `succinctly jq ARGS` with `bytes` on an anonymous pipe, read whole when `dash` (`-`).
fn over_pipe(args: &[&str], bytes: &[u8], dash: bool) -> Result<Answer> {
    let mut args = args.to_vec();
    if dash {
        args.push("-");
    }
    let log = route_log();
    let mut child = command(&args, false, &log)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let owned = bytes.to_vec();
    let writer = std::thread::spawn(move || {
        // A run that stops reading early closes its end; that is not this test's failure.
        let _ = stdin.write_all(&owned);
    });
    let output = child.wait_with_output()?;
    writer.join().expect("the writer does not panic");
    Ok(answer(output, &log))
}

/// `succinctly jq ARGS FILES...`, read whole when `whole` (`SUCCINCTLY_DEBUG_JQ_WHOLE_FILE`).
fn over_files(args: &[&str], files: &[&Path], whole: bool) -> Result<Answer> {
    let log = route_log();
    let output = command(args, whole, &log)
        .args(files)
        .stdin(Stdio::null())
        .output()?;
    Ok(answer(output, &log))
}

/// `count` records `RS {"id":N,"pad":"..."} LF`, then `tail`.
fn stream(count: usize, tail: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for id in 0..count {
        bytes.extend_from_slice(
            format!("\x1e{{\"id\":{id},\"pad\":\"{}\"}}\n", "x".repeat(id % 90)).as_bytes(),
        );
    }
    bytes.extend_from_slice(tail);
    bytes
}

/// A fault past the records an early exit reads is never reported, on a file, on two files and on a
/// pipe, where the program that reads every record reports it; each answer is the whole read's
/// (the file read whole, standard input read whole as `-`). The stream is past the walk's first
/// read, so it is read in several.
#[test]
fn a_fault_past_the_records_read_is_not_reached_7775() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let records = 4000;
    for tail in [
        &b"\x1e[3\n"[..],
        b"\x1e{\"a\":",
        b"\x1e1 2\n\x1e\xff\n",
        b"\x1e\"x\x00y\n",
    ] {
        let bytes = stream(records, tail);
        let file = dir.path().join("all.seq");
        std::fs::write(&file, &bytes)?;
        let (head, rest) = bytes.split_at(bytes.len() / 2);
        let first = dir.path().join("first.seq");
        let second = dir.path().join("second.seq");
        std::fs::write(&first, head)?;
        std::fs::write(&second, rest)?;
        for (args, early) in [
            (
                &["--seq", "-c", "-n", "first(inputs) | .id"][..],
                Some("\x1e0\n"),
            ),
            (&["--seq", "-c", "-n", "input | .id"], Some("\x1e0\n")),
            (
                &["--seq", "-c", "-n", "[limit(3; inputs) | .id]"],
                Some("\x1e[0,1,2]\n"),
            ),
            (
                &[
                    "--seq",
                    "-c",
                    "-n",
                    "first(inputs | select(.id == 3999)) | .id",
                ],
                Some("\x1e3999\n"),
            ),
            (&["--seq", "-c", "-n", "[inputs] | length"], None),
            (
                &["--seq", "-c", "-n", "reduce inputs as $x (0; . + 1)"],
                None,
            ),
            (&["--seq", "-c", "[., input] | map(.id)"], None),
        ] {
            let walked = over_files(args, &[&file], false)?;
            let whole = over_files(args, &[&file], true)?;
            let split = over_files(args, &[&first, &second], false)?;
            let split_whole = over_files(args, &[&first, &second], true)?;
            let piped = over_pipe(args, &bytes, false)?;
            let dash = over_pipe(args, &bytes, true)?;
            assert!(walked.3 && split.3 && piped.3, "{args:?} {tail:?}: walked");
            assert!(
                !whole.3 && !split_whole.3 && !dash.3,
                "{args:?} {tail:?}: read whole"
            );
            let unwalked = |answer: &Answer| (answer.0.clone(), answer.1.clone(), answer.2);
            assert_eq!(
                unwalked(&walked),
                unwalked(&whole),
                "{args:?} {tail:?}: a file"
            );
            assert_eq!(
                unwalked(&split),
                unwalked(&split_whole),
                "{args:?} {tail:?}: two files"
            );
            assert_eq!(
                unwalked(&piped),
                unwalked(&dash),
                "{args:?} {tail:?}: a pipe"
            );
            if let Some(stdout) = early {
                assert_eq!(
                    unwalked(&walked),
                    (stdout.as_bytes().to_vec(), String::new(), 0),
                    "{args:?} {tail:?}"
                );
            }
        }
    }
    Ok(())
}

/// A main loop that halts at its first record over a file of plain records reads it a window at a
/// time and stops there, so a fault in a later window is not met: the answer is jq's, `0` alone.
#[test]
fn a_halting_main_loop_stops_at_its_first_window_7775() -> Result<()> {
    let dir = tempfile::tempdir()?;
    // Past the four windows a file needs to be read a window at a time, in every build.
    let records = 2 * small_window::MIN_BYTES / 20;
    for tail in [&b""[..], b"\x1e[3\n", b"\x1e{\"a\":"] {
        let bytes = stream(records, tail);
        let file = dir.path().join("all.seq");
        std::fs::write(&file, &bytes)?;
        for args in [
            &["--seq", "-c", ".id, halt"][..],
            &["--seq", "-c", "if .id == 1 then halt else .id end"],
            &["--seq", "-c", ".id | halt_error(0)"],
        ] {
            let got = over_files(args, &[&file], false)?;
            let whole = over_files(args, &[&file], true)?;
            assert_eq!(
                (got.0.clone(), got.1.clone(), got.2),
                (whole.0, whole.1, whole.2),
                "{args:?} {tail:?}"
            );
            assert_eq!(got.2, 0, "{args:?} {tail:?}: {}", got.1);
            assert!(
                got.1.is_empty() || args[2].contains("halt_error"),
                "{args:?} {tail:?}: {}",
                got.1
            );
        }
    }
    Ok(())
}

/// Runs `succinctly jq ARGS` in `dir`, standard input empty, and waits for it at most [`PATIENCE`]:
/// a run blocked in an open, say, fails the test instead of hanging it.
fn within_patience(dir: &Path, args: &[&str]) -> Result<(String, String, Option<i32>)> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .current_dir(dir)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let start = Instant::now();
    while child.try_wait()?.is_none() {
        if start.elapsed() > PATIENCE {
            child.kill()?;
            bail!("{args:?} still running after {PATIENCE:?}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child.wait_with_output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    ))
}

/// A file is opened only when the reads reach it, as jq's reader opens each file only once it has
/// used every byte of the one before: a run that stops at the records of the first file never opens
/// a named pipe after it that has no writer, where opening it would wait for one for ever.
#[test]
fn an_early_exit_never_opens_the_files_after_its_records_7775() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.seq"), "\x1e{\"a\":1}\n\x1e{\"a\":2}\n")?;
    let made = Command::new("mkfifo")
        .arg(dir.path().join("b.fifo"))
        .status()?;
    assert!(made.success(), "mkfifo");
    for (program, stdout) in [
        ("first(inputs)", "\x1e{\"a\":1}\n"),
        ("input", "\x1e{\"a\":1}\n"),
        ("[limit(2; inputs)]", "\x1e[{\"a\":1},{\"a\":2}]\n"),
        ("input | halt", ""),
    ] {
        let got = within_patience(
            dir.path(),
            &["--seq", "-c", "-n", program, "a.seq", "b.fifo"],
        )?;
        assert_eq!(
            got,
            (stdout.to_string(), String::new(), Some(0)),
            "{program}"
        );
    }
    Ok(())
}

/// (program, stdout, stderr, exit status) over `a.seq` (two records), a second file that is missing
/// or will not open, and `c.seq` (one record), from jq 1.7.1 and 1.8.2, which agree: the failure is
/// reported only by a run whose reads reach it, and the run then exits 2.
const SECOND_FILE_FAILS: &[(&str, &str, &str, i32)] = &[
    ("first(inputs)", "\x1e{\"a\":1}\n", "", 0),
    (
        "[inputs]",
        "\x1e[{\"a\":1},{\"a\":2},{\"b\":3}]\n",
        "jq: error: Could not open file @FILE@: @WHY@\n",
        2,
    ),
    (
        "input, input, input",
        "\x1e{\"a\":1}\n\x1e{\"a\":2}\n\x1e{\"b\":3}\n",
        "jq: error: Could not open file @FILE@: @WHY@\n",
        2,
    ),
    (
        "reduce inputs as $x (0; .+1)",
        "\x1e3\n",
        "jq: error: Could not open file @FILE@: @WHY@\n",
        2,
    ),
    (
        "[inputs|input_filename]",
        "\x1e[\"a.seq\",\"a.seq\",\"c.seq\"]\n",
        "jq: error: Could not open file @FILE@: @WHY@\n",
        2,
    ),
    (
        "[inputs], input_line_number",
        "\x1e[{\"a\":1},{\"a\":2},{\"b\":3}]\n\x1e1\n",
        "jq: error: Could not open file @FILE@: @WHY@\n",
        2,
    ),
];

/// A second file that is missing, or that is there and will not open (its mode 000), is reported
/// only by a run whose reads reach it, as jq reports it.
#[test]
fn a_file_that_will_not_open_is_reported_only_once_reached_7775() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.seq"), "\x1e{\"a\":1}\n\x1e{\"a\":2}\n")?;
    std::fs::write(dir.path().join("c.seq"), "\x1e{\"b\":3}\n")?;
    let locked = dir.path().join("locked.seq");
    std::fs::write(&locked, "\x1e{\"x\":9}\n")?;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000))?;
    // A run as root opens it anyway, and has nothing to show.
    let refused = std::fs::File::open(&locked).is_err();
    for (file, why) in [
        ("missing.seq", "No such file or directory"),
        ("locked.seq", "Permission denied"),
    ] {
        if file == "locked.seq" && !refused {
            continue;
        }
        for &(program, stdout, stderr, status) in SECOND_FILE_FAILS {
            let got = within_patience(
                dir.path(),
                &["--seq", "-c", "-n", program, "a.seq", file, "c.seq"],
            )?;
            let stderr = stderr.replace("@FILE@", file).replace("@WHY@", why);
            assert_eq!(
                got,
                (stdout.to_string(), stderr, Some(status)),
                "{program} with {file}"
            );
        }
    }
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644))?;
    Ok(())
}
