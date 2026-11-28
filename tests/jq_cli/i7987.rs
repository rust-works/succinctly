//! #7987 step 6: the default route (a program that reads no `input`, `inputs` or
//! `input_line_number`, over JSON read whole) enters ADR-0033's model of jq 1.7.1's reader on its
//! triggers and hands back to the semi-index at the next sync point.
//!
//! Every expected output was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2
//! (`/opt/homebrew/bin/jq`) prints the same on every row. The one row that pins `succinctly jq`'s
//! own answer says so, with the rule it follows.

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

/// `succinctly jq ARGS` in `dir`, with standard input `stdin` and `env` set: (stdout, stderr,
/// status).
fn run(
    dir: &std::path::Path,
    args: &[&str],
    stdin: &[u8],
    env: &[(&str, &str)],
) -> Result<(String, String, i32)> {
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = std::process::Command::new(succinctly_bin());
            command.arg("jq").args(args).current_dir(dir);
            for &(key, value) in env {
                command.env(key, value);
            }
            command
        },
        Some(stdin),
    )?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        code,
    ))
}

/// One row: the files (`(name, bytes)`, none for standard input), the arguments before them, what
/// standard input holds, and jq's (stdout, stderr, status).
type Row = (
    &'static [(&'static str, &'static [u8])],
    &'static [&'static str],
    &'static [u8],
    (&'static str, &'static str, i32),
);

static ROWS: &[Row] = &[
    // #5804 retired on the default route: a mark that breaks off is jq's `Malformed BOM`, where
    // the route used to word the first read as a numeric literal.
    (
        &[],
        &["."],
        b"\xef\xbf\xbd1",
        ("", "jq: parse error: Malformed BOM\n", 5),
    ),
    (
        &[("f.json", b"\xef\xbb1\n[2]\n")],
        &["-c", "."],
        b"",
        ("", "jq: parse error: Malformed BOM\n", 5),
    ),
    // A mark cut across files is skipped as jq skips it, and the model hands back after the read
    // that completes it.
    (
        &[("m0.json", b"\xef"), ("m1.json", b"\xbb\xbf[1]\n2\n")],
        &["-c", "."],
        b"",
        ("[1]\n2\n", "", 0),
    ),
    // A fault: the model reads the file it is in, so a malformed value there that `.` would have
    // printed unread is jq's fault, and nothing after it is printed.
    (
        &[("v.json", b"[]\n{} \"a\\x\" }\n")],
        &["-c", "."],
        b"",
        (
            "[]\n{}\n",
            "jq: parse error: Invalid escape at line 2, column 8\n",
            5,
        ),
    ),
    // Invalid UTF-8: the model reads the line it is on and validates it as jq's parser does.
    (
        &[("r1.json", b"1\n\"\xff\" \"a\\x\"\n2\n")],
        &["-c", "."],
        b"",
        (
            "1\n\"\u{fffd}\"\n",
            "jq: parse error: Invalid escape at line 2, column 9\n",
            5,
        ),
    ),
    // Several substituted lines, each a region, with the positions of the values between them and
    // the fault after them jq's.
    (
        &[("u.json", b"\"\xff\"\n1\n\"a\xfeb\" 2\n[3]\n\"\xe9\"\n}\n")],
        &["-c", "error(tojson)"],
        b"",
        (
            "",
            concat!(
                "jq: error (at u.json:1): \"\u{fffd}\"\n",
                "jq: error (at u.json:2): 1\n",
                "jq: error (at u.json:3): \"a\u{fffd}b\"\n",
                "jq: error (at u.json:3): 2\n",
                "jq: error (at u.json:4): [3]\n",
                "jq: error (at u.json:5): \"\u{fffd}\"\n",
                "jq: parse error: Unmatched '}' at line 6, column 1\n",
            ),
            5,
        ),
    ),
    // jq names the first fault of its input, which here is in a file before the one the route met
    // its fault in: the model reads the stream from its start for it.
    (
        &[("e0.json", b"\"a\\x\"\n"), ("e1.json", b"}\n")],
        &["-c", "."],
        b"",
        (
            // Not jq's stdout (jq prints nothing): `.` prints a value of an earlier file without
            // reading it (#3035, the #2103 amendment's lazy validation).
            "\"a\\x\"\n",
            "jq: parse error: Invalid escape at line 1, column 5\n",
            5,
        ),
    ),
];

#[test]
fn the_default_route_reads_its_edges_through_the_reader_model_7987() -> Result<()> {
    for &(files, args, stdin, want) in ROWS {
        let dir = tempfile::tempdir()?;
        let mut argv: Vec<&str> = args.to_vec();
        for &(name, bytes) in files {
            std::fs::write(dir.path().join(name), bytes)?;
            argv.push(name);
        }
        let got = run(dir.path(), &argv, stdin, &[])?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            want,
            "{args:?} {files:?}"
        );
    }
    Ok(())
}

/// The model hands back at the next sync point: past the substituted line the semi-index reads on,
/// so a malformed value it prints without reading is not diagnosed (#3035), as it is not far from
/// any trigger. jq 1.7.1 says `Invalid escape at line 2, column 5` and prints only the first value.
#[test]
fn the_semi_index_reads_on_after_the_hand_back_7987() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("r2.json"), b"\"\xff\"\n\"a\\x\"\n")?;
    let got = run(dir.path(), &["-c", ".", "r2.json"], b"", &[])?;
    assert_eq!(got, ("\"\u{fffd}\"\n\"a\\x\"\n".into(), String::new(), 0));
    Ok(())
}

/// `SUCCINCTLY_READER_MODEL=0` selects the deciders the model replaces (ADR-0033: two readers until
/// step 10), so the route words the broken mark as it did before (#5804).
#[test]
fn the_switch_selects_the_old_reader_on_the_default_route_7987() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let got = run(
        dir.path(),
        &["."],
        b"\xef\xbf\xbd1",
        &[("SUCCINCTLY_READER_MODEL", "0")],
    )?;
    assert_eq!(
        got,
        (
            String::new(),
            "jq: parse error: Invalid numeric literal at EOF at line 1, column 4\n".into(),
            5
        )
    );
    Ok(())
}
