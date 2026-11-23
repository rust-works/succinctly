//! #7889: `--seq` over standard input redirected from a regular file that holds no RS, with a
//! program that can halt, names the end of the file in `Unfinished abandoned text at EOF`.
//!
//! jq reads a `--seq` stream with no RS as one abandoned text, runs the program on nothing and
//! reports where the stream ended. Past its first read (16,384 bytes), a redirected file with no RS
//! is scanned to its end without being held, and handed on as no bytes with the place the scan
//! found (#7260). Since #7775 a program holding `halt` or `halt_error` walks standard input as it
//! is read, and that walk was given the empty stream and not the place: it reported `line 1,
//! column 0` whatever the file held.
//!
//! Every expected line was captured from jq 1.7.1 (`/usr/bin/jq`) and jq 1.8.2
//! (`/opt/homebrew/bin/jq`) on macOS arm64, which agree on every row.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Result;

use crate::succinctly_bin;

/// The files the rows read: (name, the text repeated, how many times, the text after it).
const FILES: &[(&str, &str, usize, &str)] = &[
    // 18,000 bytes, the issue's file.
    ("lines.json", "1\n", 9000, ""),
    // The last line has no line break: the column is that line's.
    ("noeol.json", "1\n", 9000, "abc"),
    // Every line opens an array that never closes.
    ("open.json", "[1,\n", 6000, ""),
    // One line of 80,003 bytes and its line break.
    ("wide.json", "1,", 40000, "1]\n"),
    ("crlf.json", "1\r\n", 7000, ""),
    // 16,382 bytes: inside the first read, which never reported the wrong line.
    ("small.json", "1\n", 8191, ""),
    // An RS at the very end: the one record after it is read, and nothing is abandoned.
    ("rslast.json", "1\n", 9000, "\u{1e}2\n"),
];

fn files(dir: &Path) -> Result<()> {
    for &(name, unit, count, tail) in FILES {
        let mut text = unit.repeat(count);
        text.push_str(tail);
        std::fs::write(dir.join(name), text)?;
    }
    Ok(())
}

/// How a row hands its file to the run.
#[derive(Clone, Copy, Debug)]
enum Entry {
    /// Standard input redirected from the file (`jq ... < file`).
    Redirected,
    /// The file named as an argument.
    Named,
}

/// Runs `succinctly jq --seq <program>` in `dir` over `file`.
fn run(
    dir: &Path,
    program: &str,
    file: &str,
    entry: Entry,
) -> Result<(String, String, Option<i32>)> {
    let mut command = Command::new(succinctly_bin());
    command
        .arg("jq")
        .arg("--seq")
        .arg(program)
        .current_dir(dir)
        .env("LC_ALL", "C");
    match entry {
        Entry::Redirected => {
            command.stdin(Stdio::from(std::fs::File::open(dir.join(file))?));
        }
        Entry::Named => {
            command.arg(file).stdin(Stdio::null());
        }
    }
    let output = command.output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code(),
    ))
}

/// jq's warning for a stream that ends at `line`, `column` with no RS in it.
fn abandoned(line: usize, column: usize) -> String {
    format!(
        "jq: ignoring parse error: Unfinished abandoned text at EOF at line {line}, column {column}\n"
    )
}

/// (program, file, the line and column of the warning).
const ABANDONED: &[(&str, &str, usize, usize)] = &[
    ("halt", "lines.json", 9001, 0),
    ("1 | halt", "lines.json", 9001, 0),
    ("first(halt)", "lines.json", 9001, 0),
    ("halt, 1", "lines.json", 9001, 0),
    ("if . == 2 then halt else . end", "lines.json", 9001, 0),
    ("\"bye\\n\" | halt_error(0)", "lines.json", 9001, 0),
    ("halt_error", "lines.json", 9001, 0),
    ("halt", "noeol.json", 9001, 3),
    ("\"bye\\n\" | halt_error(0)", "noeol.json", 9001, 3),
    ("halt", "open.json", 6001, 0),
    ("halt", "wide.json", 2, 0),
    ("halt", "crlf.json", 7001, 0),
    ("halt", "small.json", 8192, 0),
    // A program that cannot halt never reported the wrong line.
    (".", "lines.json", 9001, 0),
    (".", "noeol.json", 9001, 3),
];

#[test]
fn test_seq_halting_program_names_the_end_of_a_redirected_file_with_no_rs_7889() -> Result<()> {
    let dir = tempfile::tempdir()?;
    files(dir.path())?;
    for &(program, file, line, column) in ABANDONED {
        for entry in [Entry::Redirected, Entry::Named] {
            let (stdout, stderr, code) = run(dir.path(), program, file, entry)?;
            let row = format!("--seq {program:?} over {file} ({entry:?})");
            assert_eq!(stdout, "", "stdout: {row}");
            assert_eq!(stderr, abandoned(line, column), "stderr: {row}");
            assert_eq!(code, Some(0), "exit status: {row}");
        }
    }
    Ok(())
}

/// An RS anywhere makes the stream one that is read: the record after it runs, and nothing is
/// reported as abandoned.
#[test]
fn test_seq_halting_program_reads_the_record_after_a_late_rs_7889() -> Result<()> {
    let dir = tempfile::tempdir()?;
    files(dir.path())?;
    // (program, stdout, stderr)
    let rows: &[(&str, &str, &str)] = &[
        ("halt", "", ""),
        ("if . == 2 then halt else . end", "", ""),
        ("\"bye\\n\" | halt_error(0)", "", "bye\n"),
        (".", "\u{1e}2\n", ""),
    ];
    for &(program, want_stdout, want_stderr) in rows {
        for entry in [Entry::Redirected, Entry::Named] {
            let (stdout, stderr, code) = run(dir.path(), program, "rslast.json", entry)?;
            let row = format!("--seq {program:?} over rslast.json ({entry:?})");
            assert_eq!(stdout, want_stdout, "stdout: {row}");
            assert_eq!(stderr, want_stderr, "stderr: {row}");
            assert_eq!(code, Some(0), "exit status: {row}");
        }
    }
    Ok(())
}
