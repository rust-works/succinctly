//! #7718: under `--seq`, jq 1.7.1 ended the stream silently at an empty record that was the first
//! thing its last `fgets` buffer held (#2998): `printf '\x1e1\n\x1etrue' | jq --seq -c .` printed
//! `1` alone, exit 0. jq 1.8.2 reports the end only once the last file is closed, one refill later
//! (jqlang/jq#3279, commit 4e3088f), so it reads every record, and `succinctly jq` follows it. The
//! same refill makes a byte-order mark that breaks off wipe the EOF branch's report, and makes `-s`
//! report that branch's warning before the program runs (#3201 deferred it). What is named as the
//! position after the last input stays jq 1.7.1's (#7607).
//!
//! The literal rows were captured from jq 1.8.2 (`/opt/homebrew/bin/jq`); jq 1.7.1's answers are in
//! the comments. The route rows hold every `--seq` reader to one answer over the same bytes: a pipe
//! read in windows, standard input read whole (`-`), a regular file, the file read whole
//! (`SUCCINCTLY_DEBUG_JQ_WHOLE_FILE`), and the stream split across two files.

#![cfg(unix)]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Result;

use crate::{run_jq_full, small_window};

/// (program arguments, stdin, stdout, stderr, exit status), all jq 1.8.2's.
const ROWS: &[(&[&str], &str, &str, &str, i32)] = &[
    // jq 1.7.1: `"a"` alone.
    (&["--seq", "-c", "."], "\x1e\"a\"\n\x1e\"b\"", "\x1e\"a\"\n\x1e\"b\"\n", "", 0),
    // jq 1.7.1: `[1]` alone.
    (&["--seq", "-c", "."], "\x1e[1]\n\x1e[2]", "\x1e[1]\n\x1e[2]\n", "", 0),
    // jq 1.7.1: `1` alone.
    (&["--seq", "-c", "."], "\x1e1\n\x1etrue", "\x1e1\n\x1etrue\n", "", 0),
    // jq 1.7.1: nothing.
    (&["--seq", "-c", "."], "\x1e\x1etrue", "\x1etrue\n", "", 0),
    (&["--seq", "-c", "."], "\x1e1\n\x1e\x1e2 ", "\x1e1\n\x1e2\n", "", 0),
    // jq 1.7.1: `1`, and no warning.
    (
        &["--seq", "-c", "."],
        "\x1e1\n\x1e[2",
        "\x1e1\n",
        "jq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 3\n",
        0,
    ),
    (
        &["--seq", "-c", "."],
        "\x1e1\n\x1e2",
        "\x1e1\n",
        "jq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 2, column 2\n",
        0,
    ),
    // jq 1.7.1: `[[1]]`.
    (&["--seq", "-n", "-c", "[inputs]"], "\x1e[1]\n\x1e[2]", "\x1e[[1],[2]]\n", "", 0),
    // jq 1.7.1: `[]` then `[true]` (#5662's `break` ended the first `inputs`).
    (
        &["--seq", "-n", "-c", "[inputs], [inputs]"],
        "\x1e\x1etrue",
        "\x1e[true]\n\x1e[]\n",
        "",
        0,
    ),
    (&["--seq", "-n", "-c", "input"], "\x1e1\n\x1etrue", "\x1e1\n", "", 0),
    // `-s`: the EOF warning before the program's error. jq 1.7.1 printed them the other way round.
    (
        &["--seq", "-s", "-c", "error(\"x\")"],
        "\x1e1{",
        "",
        "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 3\njq: error (at <stdin>:0): x\n",
        5,
    ),
    // jq 1.7.1: `[[1],"Unfinished JSON term at EOF at line 1, column 3"]`.
    (
        &["--seq", "-s", "-c", "[., (try input catch .)]"],
        "\x1e1{",
        "\x1e[[1],\"break\"]\n",
        "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 3\n",
        0,
    ),
    // jq 1.7.1 named `<unknown>`: it raised the report in the call after the array.
    (
        &["--seq", "-n", "-s", "-c", "[inputs]"],
        "\x1etrue\"",
        "",
        "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 6\n",
        5,
    ),
    // `--seq --stream`: every record's events, and under `-s` every event in the array.
    // jq 1.7.1: `[[0],1]` and `[[0]]`.
    (
        &["--seq", "--stream", "-c", "."],
        "\x1e[1]\n\x1e[2]",
        "\x1e[[0],1]\n\x1e[[0]]\n\x1e[[0],2]\n\x1e[[0]]\n",
        "",
        0,
    ),
    // jq 1.7.1: `[[[0],1],[[0]],[[0],2]]` and then `[[0]]`.
    (
        &["--seq", "--stream", "-s", "-c", "."],
        "\x1e[1]\n\x1e[2]",
        "\x1e[[[0],1],[[0]],[[0],2],[[0]]]\n",
        "",
        0,
    ),
];

#[test]
fn every_record_is_read_as_jq_1_8_2_reads_it_7718() -> Result<()> {
    for &(args, input, stdout, stderr, status) in ROWS {
        let got = run_jq_full(args, Some(input))?;
        assert_eq!(
            got,
            (stdout.to_string(), stderr.to_string(), status),
            "{args:?} over {input:?}"
        );
    }
    Ok(())
}

/// A byte-order mark that breaks off (`EF BB`, not UTF-8, so through a file): the empty last call's
/// `parser_reset` wipes what the EOF branch would report and a number still waiting for the byte
/// after it. jq 1.7.1 printed `1` and warned `Potentially truncated top-level numeric value at EOF at
/// line 1, column 3` for the first row, and ended the second and third at the RS after the newline
/// (#2998), printing `1` alone; a last line that ends in a newline reads alike in both.
#[test]
fn a_mark_that_breaks_off_wipes_the_eof_report_7718() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for (bytes, stdout, stderr) in [
        (&b"\xef\xbb1 2"[..], "\x1e1\n", ""),
        (b"\xef\xbb\x1e1\n\x1e2 ", "\x1e1\n\x1e2\n", ""),
        (b"\xef\xbb\x1e1\n\x1e\"a", "\x1e1\n", ""),
        (b"\xef\xbb1 2\n", "\x1e1\n\x1e2\n", ""),
    ] {
        let path = dir.path().join("m.seq");
        std::fs::write(&path, bytes)?;
        let got = run_jq_full(&["--seq", "-c", ".", path.to_str().unwrap()], None)?;
        assert_eq!(
            got,
            (stdout.to_string(), stderr.to_string(), 0),
            "{bytes:?}"
        );
    }
    Ok(())
}

/// What a run answered.
type Answer = (Vec<u8>, String, i32);

/// The file each run logs the windowed routes it takes to (`SUCCINCTLY_DEBUG_JQ_ROUTE_LOG`, #7688).
fn route_log() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("succinctly-7718-routes-{}", std::process::id()))
}

/// The routes the last run logged, cleared for the next.
fn routes_taken() -> Vec<String> {
    let log = route_log();
    let routes = std::fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_file(&log);
    routes
}

fn command(args: &[&str], whole: bool) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command
        .arg("jq")
        .args(args)
        .env("LC_ALL", "C")
        .env("SUCCINCTLY_DEBUG_JQ_ROUTE_LOG", route_log())
        .env_remove("SUCCINCTLY_DEBUG_JQ_WHOLE_FILE");
    small_window::apply(&mut command);
    if whole {
        command.env("SUCCINCTLY_DEBUG_JQ_WHOLE_FILE", "1");
    }
    command
}

fn answer(output: std::process::Output) -> Answer {
    (
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

/// `succinctly jq ARGS` with `bytes` on an anonymous pipe.
fn over_pipe(args: &[&str], bytes: &[u8]) -> Result<Answer> {
    let mut child = command(args, false)
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
    Ok(answer(output))
}

/// `succinctly jq ARGS FILES...`.
fn over_files(args: &[&str], files: &[&Path], whole: bool) -> Result<Answer> {
    let output = command(args, whole)
        .args(files)
        .stdin(Stdio::null())
        .output()?;
    Ok(answer(output))
}

/// Every route over `bytes` answers alike: the answer, after each is checked against the first, and
/// whether the pipe and the file were read in windows.
fn every_route(dir: &Path, args: &[&str], bytes: &[u8]) -> Result<(Answer, bool)> {
    let file = dir.join("all.seq");
    std::fs::write(&file, bytes)?;
    routes_taken();
    let piped = over_pipe(args, bytes)?;
    let mut windowed = !routes_taken().is_empty();
    let mut dash = args.to_vec();
    dash.push("-");
    let whole_stdin = over_pipe(&dash, bytes)?;
    assert_eq!(routes_taken(), Vec::<String>::new(), "`-` is read whole");
    let in_file = over_files(args, &[&file], false)?;
    windowed &= !routes_taken().is_empty();
    let whole_file = over_files(args, &[&file], true)?;
    assert_eq!(
        routes_taken(),
        Vec::<String>::new(),
        "the oracle is read whole"
    );
    let routes = [
        ("standard input read whole", whole_stdin),
        ("a file", in_file),
        ("a file read whole", whole_file),
    ];
    // A file's errors name it where standard input's name `<stdin>`.
    let name = file.to_string_lossy().into_owned();
    for (route, got) in &routes {
        let stderr = got.1.replace(&name, "<stdin>");
        assert!(
            got.0 == piped.0,
            "{args:?}: stdout over {route} differs from the pipe's ({} and {} bytes)",
            got.0.len(),
            piped.0.len()
        );
        assert_eq!((&stderr, got.2), (&piped.1, piped.2), "{args:?}: {route}");
    }
    // Split at the last RS: the last file holds the last record.
    if let Some(at) = bytes.iter().rposition(|&b| b == 0x1e).filter(|&at| at > 0) {
        let (a, b) = (dir.join("a.seq"), dir.join("b.seq"));
        std::fs::write(&a, &bytes[..at])?;
        std::fs::write(&b, &bytes[at..])?;
        let split = over_files(args, &[&a, &b], false)?;
        assert!(
            split.0 == piped.0,
            "{args:?}: stdout over two files differs from the pipe's"
        );
        // The second file's line count starts again, which only a runtime error's `(at f:N)` names.
        assert_eq!(split.2, piped.2, "{args:?}: two files");
        if !piped.1.contains("(at ") {
            assert_eq!(split.1, piped.1, "{args:?}: two files");
        }
    }
    Ok((piped, windowed))
}

/// `RS {"id":N} LF` records, about `bytes` of them.
fn records(bytes: usize) -> (Vec<u8>, usize) {
    let mut out = Vec::with_capacity(bytes + 64);
    let mut count = 0;
    while out.len() < bytes {
        out.extend_from_slice(format!("\x1e{{\"id\":{count}}}\n").as_bytes());
        count += 1;
    }
    (out, count)
}

/// (tail after the records, what `--seq -c .` prints of the tail, the warning jq 1.8.2 prints for it
/// with `@LINE@` for its line, which is one past the records').
const TAILS: &[(&str, &str, &str)] = &[
    ("\x1e\"b\"", "\x1e\"b\"\n", ""),
    ("\x1e\x1etrue", "\x1etrue\n", ""),
    ("\x1e{\"id\":-1}", "\x1e{\"id\":-1}\n", ""),
    ("\x1e\x1e{\"id\":-2}\n", "\x1e{\"id\":-2}\n", ""),
    (
        "\x1e[2",
        "",
        "jq: ignoring parse error: Unfinished JSON term at EOF at line @LINE@, column 3\n",
    ),
    (
        "\x1e7",
        "",
        "jq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line @LINE@, column 2\n",
    ),
];

/// The tails over a few records, which every reader reads whole, and over enough records for the
/// windowed routes (a pipe, a file and two files of four windows or more), with `.`, a program that
/// reads every input, and a slurp.
#[test]
fn every_seq_route_reads_past_the_last_newline_alike_7718() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for size in [16, small_window::MIN_BYTES + small_window::WINDOW] {
        let (head, count) = records(size);
        for &(tail, printed, warning) in TAILS {
            let mut bytes = head.clone();
            bytes.extend_from_slice(tail.as_bytes());
            let line = (count + 1).to_string();
            let warning = warning.replace("@LINE@", &line);
            let ((stdout, stderr, status), windowed) =
                every_route(dir.path(), &["--seq", "-c", "."], &bytes)?;
            // The large stream is read in windows over the pipe and the file, or the rows compare
            // one route with itself.
            assert_eq!(windowed, size > 16, "{size} bytes + {tail:?}: windowed");
            let mut want = head.clone();
            want.extend_from_slice(printed.as_bytes());
            assert!(
                stdout == want,
                "{size} bytes + {tail:?}: stdout ({} bytes, {} wanted)",
                stdout.len(),
                want.len()
            );
            assert_eq!((stderr.as_str(), status), (warning.as_str(), 0), "{tail:?}");
            let values = count + usize::from(!printed.is_empty());
            let ((stdout, _, _), _) =
                every_route(dir.path(), &["--seq", "-n", "[inputs] | length"], &bytes)?;
            if warning.is_empty() {
                assert_eq!(stdout, format!("\x1e{values}\n").into_bytes(), "{tail:?}");
            }
            let ((stdout, _, status), _) =
                every_route(dir.path(), &["--seq", "-s", "length"], &bytes)?;
            assert_eq!(
                (stdout, status),
                (format!("\x1e{values}\n").into_bytes(), 0),
                "{tail:?}"
            );
        }
    }
    Ok(())
}
