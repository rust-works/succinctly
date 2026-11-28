//! #7863: after a malformed byte-order mark jq's parser stays in the fault (`bom_strip_position`
//! 0xff), so every later `input` fails with `Malformed BOM` again, never reads past the mark and never
//! finds the end of the stream. `succinctly jq` raised the fault once and then found the stream at its
//! end: the second `try input catch .` said `break`, an `input` after a caught `inputs` said `break`
//! at `(at <unknown>)`, and a stream with more text after the mark handed that text out as documents.
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 gives the same on each. Since step
//! 5 of #7987 the materializing route these runs take reads the mark through ADR-0033's reader model,
//! so the first read says `Malformed BOM` too, as jq does. A pipe longer than its first read is read
//! lazily, a route the model has not reached, whose first read keeps the numeric-literal wording
//! (#5804, recorded in docs/compliance/jq/limitations.md): `first` says what `succinctly jq` says
//! there; the reads after it are jq's own words.

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

/// One run: its flags and program, the files it names (written under their names, in this order) or
/// else the bytes on standard input, what jq 1.7.1 answered (standard output, standard error, exit
/// status) and the wording of the first fault where `succinctly jq` words it differently.
struct Row {
    label: &'static str,
    program: &'static str,
    files: &'static [(&'static str, &'static [u8])],
    stdin: &'static [u8],
    stdout: &'static str,
    stderr: &'static str,
    status: i32,
    first: Option<&'static str>,
}

const THREE: &str = "(try input catch .), (try input catch .), (try input catch .)";
const THREE_BOM: &str = "\"Malformed BOM\"\n\"Malformed BOM\"\n\"Malformed BOM\"\n";
const AT_LINE_2: &str = "Invalid numeric literal at line 2, column 0";

static ROWS: &[Row] = &[
    Row {
        label: "three reads on standard input",
        program: THREE,
        files: &[],
        stdin: b"\xef1\n",
        stdout: THREE_BOM,
        stderr: "",
        status: 0,
        first: None,
    },
    // Text after the mark is never read: it is not handed out as documents.
    Row {
        label: "a document after the mark",
        program: THREE,
        files: &[],
        stdin: b"\xef1\n[2]\n",
        stdout: THREE_BOM,
        stderr: "",
        status: 0,
        first: None,
    },
    Row {
        label: "EF BB, then a byte that is not BF",
        program: THREE,
        files: &[],
        stdin: b"\xef\xbb1\n",
        stdout: THREE_BOM,
        stderr: "",
        status: 0,
        first: None,
    },
    Row {
        label: "EF BF BD, a character that is not the mark",
        program: THREE,
        files: &[],
        stdin: b"\xef\xbf\xbd1",
        stdout: THREE_BOM,
        stderr: "",
        status: 0,
        first: None,
    },
    // The first fault is caught by `?`: the `input` after it says it, at the same place.
    Row {
        label: "inputs?, then input",
        program: "[inputs]?, input",
        files: &[],
        stdin: b"\xef1\n",
        stdout: "",
        stderr: "jq: error (at <stdin>:1): Malformed BOM\n",
        status: 5,
        first: None,
    },
    Row {
        label: "first(inputs) twice",
        program: "(try first(inputs) catch .), (try first(inputs) catch .)",
        files: &[],
        stdin: b"\xef1\n",
        stdout: "\"Malformed BOM\"\n\"Malformed BOM\"\n",
        stderr: "",
        status: 0,
        first: None,
    },
    Row {
        label: "the marker names the file the mark broke off in",
        program: "[inputs]?, input",
        files: &[("efbad.json", b"\xef\xbf\xbd1")],
        stdin: b"",
        stdout: "",
        stderr: "jq: error (at efbad.json:0): Malformed BOM\n",
        status: 5,
        first: None,
    },
    Row {
        label: "a file after the one the mark broke off in",
        program: THREE,
        files: &[("efbad.json", b"\xef\xbf\xbd1"), ("one.json", b"[1]\n")],
        stdin: b"",
        stdout: THREE_BOM,
        stderr: "",
        status: 0,
        first: None,
    },
    Row {
        label: "a mark cut across files that departs in the second",
        program: THREE,
        files: &[("ef.json", b"\xef"), ("one.json", b"[1]\n")],
        stdin: b"",
        stdout: THREE_BOM,
        stderr: "",
        status: 0,
        first: None,
    },
    Row {
        label: "input_line_number and input_filename after the fault",
        program: "(try input catch .), input_line_number, input_filename",
        files: &[],
        stdin: b"\xef1\n",
        stdout: "\"Malformed BOM\"\n1\n\"<stdin>\"\n",
        stderr: "",
        status: 0,
        first: None,
    },
    Row {
        label: "inputs",
        program: "[inputs]",
        files: &[],
        stdin: b"\xef1\n",
        stdout: "",
        stderr: "jq: error (at <stdin>:1): Malformed BOM\n",
        status: 5,
        first: None,
    },
    // A mark that ends the input is no fault (#7454): the stream is empty, so every read is its end.
    Row {
        label: "EF alone",
        program: THREE,
        files: &[],
        stdin: b"\xef",
        stdout: "\"break\"\n\"break\"\n\"break\"\n",
        stderr: "",
        status: 0,
        first: None,
    },
    // A whole mark is skipped, and what follows it is not a mark at the front of the stream.
    Row {
        label: "a whole mark, then EF 31",
        program: THREE,
        files: &[],
        stdin: b"\xef\xbb\xbf\xef1\n[3]\n",
        stdout: "\"Invalid numeric literal at line 2, column 0\"\n[3]\n\"break\"\n",
        stderr: "",
        status: 0,
        first: None,
    },
];

/// What `succinctly jq` says for `row`: jq's answer with its first `Malformed BOM` in the numeric-literal
/// wording where the first read's fault is in the output.
fn expected(row: &Row) -> (String, String) {
    let word = |text: &str| match row.first {
        Some(first) => text.replacen("Malformed BOM", first, 1),
        None => text.to_owned(),
    };
    // A fault that ends the run is the one on standard error, the first of the reads when it is also
    // the only one.
    if row.stdout.is_empty() {
        (String::new(), word(row.stderr))
    } else {
        (word(row.stdout), row.stderr.to_owned())
    }
}

fn run(row: &Row, stdin: &[u8]) -> Result<(String, String, i32)> {
    let dir = tempfile::tempdir()?;
    for (name, bytes) in row.files {
        std::fs::write(dir.path().join(name), bytes)?;
    }
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = std::process::Command::new(succinctly_bin());
            command.args(["jq", "-c", "-n", row.program]);
            command.args(row.files.iter().map(|(name, _)| name));
            command.current_dir(dir.path());
            command
        },
        (!row.files.is_empty()).then_some(&b""[..]).or(Some(stdin)),
    )?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        code,
    ))
}

#[test]
fn every_read_after_a_malformed_mark_repeats_jqs_fault_7863() -> Result<()> {
    for row in ROWS {
        let (stdout, stderr) = expected(row);
        assert_eq!(
            run(row, row.stdin)?,
            (stdout, stderr, row.status),
            "{}",
            row.label
        );
    }
    Ok(())
}

/// A pipe is read a window at a time past its first 256 KiB, where the fault is met in the first read
/// of it and the text after it is a megabyte of documents jq never reads.
#[test]
fn a_pipe_longer_than_its_first_read_repeats_the_fault_too_7863() -> Result<()> {
    let mut stdin = b"\xef1\n".to_vec();
    stdin.extend(b"{\"a\":1}\n".repeat(150_000));
    let row = Row {
        label: "a long pipe",
        program: THREE,
        files: &[],
        stdin: b"",
        stdout: THREE_BOM,
        stderr: "",
        status: 0,
        first: Some(AT_LINE_2),
    };
    let (stdout, stderr) = expected(&row);
    assert_eq!(run(&row, &stdin)?, (stdout, stderr, 0));
    Ok(())
}
