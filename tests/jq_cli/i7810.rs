//! #7810: a number or literal directly before the input's last byte, an unterminated string's `"`
//! (`2"`, `true"`), is completed by that byte, so it is a value by the time jq's parser meets the
//! end of input, as it is before a final `[` or `{` (#6004). The read that completed it is the read
//! that reached the end, so jq 1.7.1's `input` closes the file before it meets the fault and names
//! it `(at <unknown>)`. `succinctly jq` keeps jq 1.7.1's positions after the last input (#7607), and
//! named `(at <stdin>:0)` here, which is jq 1.8.2's answer (each such row says so above it).
//!
//! The rule takes its read from jq's 4095-byte `fgets` chunks: a last line that fills its reads
//! exactly leaves an empty final read, which the file is still open for, so 4,093 spaces then `2"`
//! or `2[` keep the file's name where 4,092 and 4,094 lose it (the `[` rows answered `<unknown>`
//! for all three before). A space before the quote (`2 "`), a string or container before it
//! (`"a""`), or text after it (`2"abc`) keeps the name too. The reader `--stream -s` and `--seq
//! --stream` read with named `<unknown>` where an empty file or an empty read followed the last
//! bytes; jq names that file (`e.json:0`). Every row was captured from jq 1.7.1 (`/usr/bin/jq`).

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Result;

use crate::small_window;

/// One run: the arguments, the inputs, and jq 1.7.1's answer.
struct Row {
    args: &'static [&'static str],
    /// `(name, lead, spaces, tail)`: each input is `lead`, then that many spaces, then `tail`. The
    /// name `-` is standard input; every other input is a file in the run's directory, named on the
    /// command line in this order.
    files: &'static [(&'static str, &'static [u8], usize, &'static [u8])],
    stdout: &'static str,
    stderr: &'static str,
    code: i32,
}

/// `succinctly jq ARGS FILES...` run in `dir`, with `stdin` on an anonymous pipe.
fn run(dir: &Path, args: &[&str], files: &[String], stdin: &[u8]) -> Result<(String, String, i32)> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command
        .arg("jq")
        .args(args)
        .args(files)
        .current_dir(dir)
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    small_window::apply(&mut command);
    let mut child = command.spawn()?;
    let mut pipe = child.stdin.take().expect("stdin is piped");
    let bytes = stdin.to_vec();
    // A run that stops reading early closes its end; that is not this test's failure.
    let writer = std::thread::spawn(move || {
        let _ = pipe.write_all(&bytes);
    });
    let output = child.wait_with_output()?;
    writer.join().expect("the writer does not panic");
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    ))
}

fn check(rows: &[Row]) -> Result<()> {
    for row in rows {
        let dir = tempfile::tempdir()?;
        let (mut names, mut stdin) = (Vec::new(), Vec::new());
        for &(name, lead, spaces, tail) in row.files {
            let mut bytes = lead.to_vec();
            bytes.resize(lead.len() + spaces, b' ');
            bytes.extend_from_slice(tail);
            if name == "-" {
                stdin = bytes;
            } else {
                std::fs::write(dir.path().join(name), bytes)?;
                names.push(name.to_string());
            }
        }
        let shape: Vec<_> = row
            .files
            .iter()
            .map(|&(name, lead, spaces, tail)| {
                format!(
                    "{name}: {:?} + {spaces} spaces + {:?}",
                    String::from_utf8_lossy(lead),
                    String::from_utf8_lossy(tail)
                )
            })
            .collect();
        assert_eq!(
            run(dir.path(), row.args, &names, &stdin)?,
            (row.stdout.to_string(), row.stderr.to_string(), row.code),
            "{:?} over {shape:?}",
            row.args
        );
    }
    Ok(())
}

const PIPE: &[Row] = &[
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 2
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 2\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 5
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"true\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 5\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 5
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"null\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 5\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 6
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"false\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 6\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 4
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"0.5\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 4\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 3
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"-1\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 4
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"1e5\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 4\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 4
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"1 2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 4\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:1): Unfinished string at EOF at line 2, column 2
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"1\n2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 2, column 2\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 5
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"\"a\"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 5\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"2\"abc")],
        stdout: "",
        stderr: "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 5\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"2 \"")],
        stdout: "",
        stderr: "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"\"a\"\"")],
        stdout: "",
        stderr: "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 4\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"2\n\"")],
        stdout: "",
        stderr: "jq: error (at <stdin>:1): Unfinished string at EOF at line 2, column 1\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"[1]\"")],
        stdout: "",
        stderr: "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 4\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"2\"\n")],
        stdout: "",
        stderr: "jq: error (at <stdin>:1): Unfinished string at EOF at line 2, column 0\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"2\t\"")],
        stdout: "",
        stderr: "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"{}\"")],
        stdout: "",
        stderr: "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 0, b"2\"\\")],
        stdout: "",
        stderr: "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 2
    Row {
        args: &["-c", "input"],
        files: &[("-", b"", 0, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 2\n",
        code: 5,
    },
    Row {
        args: &["-c", "input"],
        files: &[("-", b"", 0, b"2 \"")],
        stdout: "",
        stderr: "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 2
    Row {
        args: &["-c", "[., input]"],
        files: &[("-", b"", 0, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 2\n",
        code: 5,
    },
    Row {
        args: &["-c", "[., input]"],
        files: &[("-", b"", 0, b"2 \"")],
        stdout: "",
        stderr: "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 2
    Row {
        args: &["-snc", "[inputs]"],
        files: &[("-", b"", 0, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 2\n",
        code: 5,
    },
    Row {
        args: &["-snc", "[inputs]"],
        files: &[("-", b"", 0, b"2 \"")],
        stdout: "",
        stderr: "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 2
    Row {
        args: &["-nc", "first(inputs), input"],
        files: &[("-", b"", 0, b"2\"")],
        stdout: "2\n",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 2\n",
        code: 5,
    },
    Row {
        args: &["-nc", "first(inputs), input"],
        files: &[("-", b"", 0, b"2 \"")],
        stdout: "2\n",
        stderr: "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 2
    Row {
        args: &["-c", "input_filename, input"],
        files: &[("-", b"", 0, b"2\"")],
        stdout: "\"<stdin>\"\n",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 2\n",
        code: 5,
    },
    Row {
        args: &["-c", "input_filename, input"],
        files: &[("-", b"", 0, b"2 \"")],
        stdout: "\"<stdin>\"\n",
        stderr: "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
];

const FILES: &[Row] = &[
    // jq 1.8.2: jq: error (at f.json:0): Unfinished string at EOF at line 1, column 2
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("f.json", b"", 0, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 2\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished string at EOF at line 1, column 5
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("f.json", b"", 0, b"true\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 5\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("f.json", b"", 0, b"2 \"")],
        stdout: "",
        stderr: "jq: error (at f.json:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished string at EOF at line 2, column 2
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("a.json", b"", 0, b"1\n"), ("f.json", b"", 0, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 2, column 2\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("a.json", b"", 0, b"1\n"), ("f.json", b"", 0, b"\"a\"\"")],
        stdout: "",
        stderr: "jq: error (at f.json:0): Unfinished string at EOF at line 2, column 4\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("f.json", b"", 0, b"2\""), ("e.json", b"", 0, b"")],
        stdout: "",
        stderr: "jq: error (at e.json:0): Unfinished string at EOF at line 1, column 2\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("f.json", b"", 0, b"2\""), ("t.json", b"", 0, b"3")],
        stdout: "",
        stderr: "jq: error (at t.json:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished string at EOF at line 1, column 2
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("a.json", b"", 0, b"2"), ("f.json", b"", 0, b"\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 2\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished string at EOF at line 1, column 5
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("a.json", b"", 0, b"tru"), ("f.json", b"", 0, b"e\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 5\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("a.json", b"", 0, b"2"), ("f.json", b"", 0, b"\"x")],
        stdout: "",
        stderr: "jq: error (at f.json:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished JSON term at EOF at line 1, column 2
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("a.json", b"", 0, b"2"), ("f.json", b"", 0, b"[")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 2\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished string at EOF at line 1, column 2
    Row {
        args: &["-snc", "[inputs]"],
        files: &[("f.json", b"", 0, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 2\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished string at EOF at line 1, column 5
    Row {
        args: &["-snc", "[inputs]"],
        files: &[("f.json", b"", 0, b"true\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 5\n",
        code: 5,
    },
    Row {
        args: &["-snc", "[inputs]"],
        files: &[("f.json", b"", 0, b"2 \"")],
        stdout: "",
        stderr: "jq: error (at f.json:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished string at EOF at line 2, column 2
    Row {
        args: &["-snc", "[inputs]"],
        files: &[("a.json", b"", 0, b"1\n"), ("f.json", b"", 0, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 2, column 2\n",
        code: 5,
    },
    Row {
        args: &["-snc", "[inputs]"],
        files: &[("a.json", b"", 0, b"1\n"), ("f.json", b"", 0, b"\"a\"\"")],
        stdout: "",
        stderr: "jq: error (at f.json:0): Unfinished string at EOF at line 2, column 4\n",
        code: 5,
    },
    Row {
        args: &["-snc", "[inputs]"],
        files: &[("f.json", b"", 0, b"2\""), ("e.json", b"", 0, b"")],
        stdout: "",
        stderr: "jq: error (at e.json:0): Unfinished string at EOF at line 1, column 2\n",
        code: 5,
    },
    Row {
        args: &["-snc", "[inputs]"],
        files: &[("f.json", b"", 0, b"2\""), ("t.json", b"", 0, b"3")],
        stdout: "",
        stderr: "jq: error (at t.json:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished string at EOF at line 1, column 2
    Row {
        args: &["-snc", "[inputs]"],
        files: &[("a.json", b"", 0, b"2"), ("f.json", b"", 0, b"\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 2\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished string at EOF at line 1, column 5
    Row {
        args: &["-snc", "[inputs]"],
        files: &[("a.json", b"", 0, b"tru"), ("f.json", b"", 0, b"e\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 5\n",
        code: 5,
    },
    Row {
        args: &["-snc", "[inputs]"],
        files: &[("a.json", b"", 0, b"2"), ("f.json", b"", 0, b"\"x")],
        stdout: "",
        stderr: "jq: error (at f.json:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished JSON term at EOF at line 1, column 2
    Row {
        args: &["-snc", "[inputs]"],
        files: &[("a.json", b"", 0, b"2"), ("f.json", b"", 0, b"[")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 2\n",
        code: 5,
    },
];

const READS: &[Row] = &[
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 4094
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 4092, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 4094\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished JSON term at EOF at line 1, column 4094
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 4092, b"2[")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 4094\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 4093, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 4095\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 4093, b"2[")],
        stdout: "",
        stderr: "jq: error (at <stdin>:0): Unfinished JSON term at EOF at line 1, column 4095\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 4096
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 4094, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 4096\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished JSON term at EOF at line 1, column 4096
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 4094, b"2[")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 4096\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"1\n", 4093, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <stdin>:1): Unfinished string at EOF at line 2, column 4095\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:1): Unfinished string at EOF at line 2, column 4094
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"1\n", 4092, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 2, column 4094\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 8189
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 8187, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 8189\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("-", b"", 8188, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 8190\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished string at EOF at line 1, column 4093
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("f.json", b"\"\xff\"", 4088, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 4093\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished string at EOF at line 1, column 4094
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("f.json", b"\"\xff\"", 4089, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 4094\n",
        code: 5,
    },
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("f.json", b"\"\xff\"", 4090, b"2\"")],
        stdout: "",
        stderr: "jq: error (at f.json:0): Unfinished string at EOF at line 1, column 4095\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished JSON term at EOF at line 1, column 4094
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("f.json", b"\"\xff\"", 4089, b"2[")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 4094\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished string at EOF at line 1, column 5
    Row {
        args: &["-nc", "[inputs]"],
        files: &[("f.json", b"\"\xff\"", 0, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 5\n",
        code: 5,
    },
];

const STREAM: &[Row] = &[
    Row {
        args: &["--stream", "-snc", "[inputs]"],
        files: &[("f.json", b"", 0, b"2\""), ("e.json", b"", 0, b"")],
        stdout: "",
        stderr: "jq: error (at e.json:0): Unfinished string at EOF at line 1, column 2\n",
        code: 5,
    },
    Row {
        args: &["--stream", "-snc", "[inputs]"],
        files: &[("f.json", b"", 0, b"2["), ("e.json", b"", 0, b"")],
        stdout: "",
        stderr: "jq: error (at e.json:0): Unfinished JSON term at EOF at line 1, column 2\n",
        code: 5,
    },
    Row {
        args: &["--stream", "-snc", "[inputs]"],
        files: &[("f.json", b"", 0, b"[1,"), ("e.json", b"", 0, b"")],
        stdout: "",
        stderr: "jq: error (at e.json:0): Unfinished JSON term at EOF at line 1, column 3\n",
        code: 5,
    },
    Row {
        args: &["--stream", "-snc", "[inputs]"],
        files: &[("-", b"", 4093, b"2[")],
        stdout: "",
        stderr: "jq: error (at <stdin>:0): Unfinished JSON term at EOF at line 1, column 4095\n",
        code: 5,
    },
    Row {
        args: &["--stream", "-snc", "[inputs]"],
        files: &[("-", b"", 4093, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 4095\n",
        code: 5,
    },
    Row {
        args: &["--stream", "-snc", "[inputs]"],
        files: &[("-", b"", 4092, b"[1,")],
        stdout: "",
        stderr: "jq: error (at <stdin>:0): Unfinished JSON term at EOF at line 1, column 4095\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 2
    Row {
        args: &["--stream", "-snc", "[inputs]"],
        files: &[("-", b"", 0, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 2\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at <stdin>:0): Unfinished string at EOF at line 1, column 4096
    Row {
        args: &["--stream", "-snc", "[inputs]"],
        files: &[("-", b"", 4094, b"2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 4096\n",
        code: 5,
    },
    Row {
        args: &["--seq", "--stream", "-snc", "[inputs]"],
        files: &[("f.json", b"", 0, b"\x1e2\""), ("e.json", b"", 0, b"")],
        stdout: "",
        stderr: "jq: error (at e.json:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    Row {
        args: &["--seq", "--stream", "-snc", "[inputs]"],
        files: &[("f.json", b"", 0, b"\x1e2["), ("e.json", b"", 0, b"")],
        stdout: "",
        stderr: "jq: error (at e.json:0): Unfinished JSON term at EOF at line 1, column 3\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished string at EOF at line 1, column 3
    Row {
        args: &["--seq", "--stream", "-snc", "[inputs]"],
        files: &[("f.json", b"", 0, b"\x1e2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    Row {
        args: &["--seq", "--stream", "-nc", "[inputs]"],
        files: &[("f.json", b"", 0, b"\x1e2\""), ("e.json", b"", 0, b"")],
        stdout: "",
        stderr: "jq: error (at e.json:0): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
    Row {
        args: &["--seq", "--stream", "-nc", "[inputs]"],
        files: &[("f.json", b"", 0, b"\x1e2["), ("e.json", b"", 0, b"")],
        stdout: "",
        stderr: "jq: error (at e.json:0): Unfinished JSON term at EOF at line 1, column 3\n",
        code: 5,
    },
    // jq 1.8.2: jq: error (at f.json:0): Unfinished string at EOF at line 1, column 3
    Row {
        args: &["--seq", "--stream", "-nc", "[inputs]"],
        files: &[("f.json", b"", 0, b"\x1e2\"")],
        stdout: "",
        stderr: "jq: error (at <unknown>): Unfinished string at EOF at line 1, column 3\n",
        code: 5,
    },
];

#[test]
fn the_fault_after_a_scalar_before_a_final_quote_is_named_unknown_7810() -> Result<()> {
    check(PIPE)
}

#[test]
fn every_file_route_names_it_alike_7810() -> Result<()> {
    check(FILES)
}

/// The final read is the last of jq's 4095-byte `fgets` reads of the last line, counted over the
/// bytes as read: an invalid byte earlier on the line is one byte there, where the text holds a
/// three-byte U+FFFD for it.
#[test]
fn the_final_read_is_jqs_last_4095_byte_read_7810() -> Result<()> {
    check(READS)
}

/// The reader of `--stream -s` and `--seq --stream` reads jq 1.8.2's empty closing refill (#7718,
/// #7807) and names what that refill meets as jq 1.7.1 did. It is the call after jq 1.7.1's last
/// refill only when the refill with the last bytes was that last refill: after an empty file, or a
/// last line that filled its reads, jq 1.7.1's last refill was an empty one, read with the file
/// still open.
#[test]
fn the_stream_reader_names_the_file_its_last_refill_read_7810() -> Result<()> {
    check(STREAM)
}

/// The routes that read a long input a window at a time: a lone file and standard input past their
/// front, and several files in turn (the `inputs` route), which hand the rest of the text to the
/// eager reader at the fault.
#[test]
fn the_windowed_routes_name_it_alike_7810() -> Result<()> {
    let lines = small_window::MIN_BYTES / 2 + 1;
    let front = "1\n".repeat(lines);
    assert!(front.len() >= small_window::MIN_BYTES);
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.json"), &front)?;
    let log = dir.path().join("routes.log");
    // (tail, message, column, whether jq 1.7.1 keeps the file's name)
    let tails = [
        ("2\"", "Unfinished string", 2, false),
        ("true\"", "Unfinished string", 5, false),
        ("2[", "Unfinished JSON term", 2, false),
        ("2 \"", "Unfinished string", 3, true),
        ("\"a\"\"", "Unfinished string", 4, true),
    ];
    for (tail, message, column, kept) in tails {
        let text = format!("{front}{tail}");
        std::fs::write(dir.path().join("f.json"), &text)?;
        for (files, stdin, line, name) in [
            (vec![], text.as_str(), lines + 1, "<stdin>"),
            (vec!["f.json".to_string()], "", lines + 1, "f.json"),
            (
                vec!["a.json".to_string(), "f.json".to_string()],
                "",
                2 * lines + 1,
                "f.json",
            ),
        ] {
            let at = if kept {
                format!("{name}:{lines}")
            } else {
                "<unknown>".to_string()
            };
            let _ = std::fs::remove_file(&log);
            let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
            command
                .args(["jq", "-nc", "[inputs] | length"])
                .args(&files)
                .current_dir(dir.path())
                .env("LC_ALL", "C")
                .env("SUCCINCTLY_DEBUG_JQ_ROUTE_LOG", &log)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            small_window::apply(&mut command);
            let mut child = command.spawn()?;
            let mut pipe = child.stdin.take().expect("stdin is piped");
            let bytes = stdin.as_bytes().to_vec();
            let writer = std::thread::spawn(move || {
                let _ = pipe.write_all(&bytes);
            });
            let output = child.wait_with_output()?;
            writer.join().expect("the writer does not panic");
            assert_eq!(
                (
                    String::from_utf8_lossy(&output.stdout).into_owned(),
                    String::from_utf8_lossy(&output.stderr).into_owned(),
                    output.status.code(),
                ),
                (
                    String::new(),
                    format!(
                        "jq: error (at {at}): {message} at EOF at line {line}, column {column}\n"
                    ),
                    Some(5),
                ),
                "{tail:?} after {lines} lines, files {files:?}"
            );
            if files.len() == 2 {
                let routes = std::fs::read_to_string(&log).unwrap_or_default();
                assert_eq!(routes.trim(), "inputs", "{tail:?}: the several-files route");
            }
        }
    }
    Ok(())
}
