//! #6115: `--seq` with `--stream` or `--stream-errors` reads each record as stream events, as jq's
//! parser does with `JV_PARSE_SEQ` and `JV_PARSE_STREAMING` together, where the stream flag was
//! ignored and the program saw whole documents. A fault is jq's `--seq` fault: an input
//! `[message, path]` under `--stream-errors`, and under `--stream` an ignored parse error that a
//! plain run and `-s` read past and a program's own `input` raises. Every row was captured from
//! jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::{run_jq_full, spawn_jq};

/// (stdin, arguments, stdout, stderr, exit code).
type Row = (
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
    i32,
);

const ROWS: &[Row] = &[
    (
        "\u{1e}[1]\n\u{1e}{\"a\":2}\n",
        &["-c", "--seq", "--stream", "."],
        "\u{1e}[[0],1]\n\u{1e}[[0]]\n\u{1e}[[\"a\"],2]\n\u{1e}[[\"a\"]]\n",
        "",
        0,
    ),
    (
        "\u{1e}[1]\n\u{1e}{\"a\":2}\n",
        &["-c", "--seq", "--stream", "length"],
        "\u{1e}2\n\u{1e}1\n\u{1e}2\n\u{1e}1\n",
        "",
        0,
    ),
    (
        "\u{1e}[1]\n\u{1e}{\"a\":2}\n",
        &["-c", "--seq", "--stream", "-n", "[inputs]"],
        "\u{1e}[[[0],1],[[0]],[[\"a\"],2],[[\"a\"]]]\n",
        "",
        0,
    ),
    (
        "\u{1e}[1]\n\u{1e}{\"a\":2}\n",
        &["-c", "--seq", "--stream", "-s", "."],
        "\u{1e}[[[0],1],[[0]],[[\"a\"],2],[[\"a\"]]]\n",
        "",
        0,
    ),
    (
        "\u{1e}{\"a\":[1,{\"b\":null}],\"c\":[]}\n",
        &["-c", "--seq", "--stream", "."],
        "\u{1e}[[\"a\",0],1]\n\u{1e}[[\"a\",1,\"b\"],null]\n\u{1e}[[\"a\",1,\"b\"]]\n\u{1e}[[\"a\",1]]\n\u{1e}[[\"c\"],[]]\n\u{1e}[[\"c\"]]\n",
        "",
        0,
    ),
    (
        "\u{1e}[1,2",
        &["-c", "--seq", "--stream", "."],
        "\u{1e}[[0],1]\n",
        "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 5\n",
        0,
    ),
    (
        "\u{1e}[1,2",
        &["-c", "--seq", "--stream-errors", "."],
        "\u{1e}[[0],1]\n\u{1e}[\"Unfinished JSON term at EOF at line 1, column 5\",[1]]\n",
        "",
        0,
    ),
    (
        "\u{1e}[1,2\u{1e}[3]\n",
        &["-c", "--seq", "--stream", "."],
        "\u{1e}[[0],1]\n\u{1e}[[0],3]\n\u{1e}[[0]]\n",
        "jq: ignoring parse error: Truncated value at line 1, column 6\n",
        0,
    ),
    (
        "\u{1e}[1,2\u{1e}[3]\n",
        &["-c", "--seq", "--stream-errors", "."],
        "\u{1e}[[0],1]\n\u{1e}[\"Truncated value at line 1, column 6\",[1]]\n\u{1e}[[0],3]\n\u{1e}[[0]]\n",
        "",
        0,
    ),
    (
        "\u{1e}[1,2\u{1e}[3]\n",
        &["-c", "--seq", "--stream", "-n", "[inputs]"],
        "",
        "jq: error (at <stdin>:1): Truncated value at line 1, column 6\n",
        5,
    ),
    (
        "\u{1e}[1,2\u{1e}[3]\n",
        &["-c", "--seq", "--stream", "-s", "."],
        "\u{1e}[[[0],1],[[0],3],[[0]]]\n",
        "jq: ignoring parse error: Truncated value at line 1, column 6\n",
        0,
    ),
    (
        "\u{1e}[1,2\u{1e}[3]\n",
        &["-c", "--seq", "--stream-errors", "-n", "-s", "[inputs]"],
        "\u{1e}[[[[0],1],[\"Truncated value at line 1, column 6\",[1]],[[0],3],[[0]]]]\n",
        "",
        0,
    ),
    (
        "\u{1e}1\u{1e}2\n",
        &["-c", "--seq", "--stream", "."],
        "\u{1e}[[],1]\n\u{1e}[[],2]\n",
        "",
        0,
    ),
    (
        "\u{1e}true\u{1e}[0]\n",
        &["-c", "--seq", "--stream", "."],
        "\u{1e}[[],true]\n\u{1e}[[0],0]\n\u{1e}[[0]]\n",
        "",
        0,
    ),
    (
        "\u{1e}[1]\n\u{1e}}\n\u{1e}[2]\n",
        &["-c", "--seq", "--stream", "."],
        "\u{1e}[[0],1]\n\u{1e}[[0]]\n\u{1e}[[0],2]\n\u{1e}[[0]]\n",
        "jq: ignoring parse error: Unmatched '}' at the top-level at line 2, column 2 (need RS to resync)\n",
        0,
    ),
    (
        "\u{1e}[1]\n\u{1e}}\n\u{1e}[2]\n",
        &["-c", "--seq", "--stream-errors", "."],
        "\u{1e}[[0],1]\n\u{1e}[[0]]\n\u{1e}[\"Unmatched '}' at the top-level at line 2, column 2 (need RS to resync)\",[]]\n\u{1e}[[0],2]\n\u{1e}[[0]]\n",
        "",
        0,
    ),
    (
        "\u{1e}[1,}2]\n\u{1e}[3]\n",
        &["-c", "--seq", "--stream", "."],
        "\u{1e}[[0],1]\n\u{1e}[[0],3]\n\u{1e}[[0]]\n",
        "jq: ignoring parse error: Expected another key:value pair at line 1, column 5 (need RS to resync)\njq: ignoring parse error: Unmatched ']' at the top-level at line 1, column 7 (need RS to resync)\n",
        0,
    ),
    (
        "[1]\n",
        &["-c", "--seq", "--stream", "."],
        "",
        "jq: ignoring parse error: Unfinished abandoned text at EOF at line 2, column 0\n",
        0,
    ),
    (
        "[1]\n",
        &["-c", "--seq", "--stream-errors", "."],
        "\u{1e}[\"Unfinished abandoned text at EOF at line 2, column 0\",[]]\n",
        "",
        0,
    ),
    (
        "[1]\n",
        &["-c", "--seq", "--stream", "-n", "input"],
        "",
        "jq: error (at <stdin>:1): Unfinished abandoned text at EOF at line 2, column 0\n",
        5,
    ),
    (
        "x\u{1e}[1]\n",
        &["-c", "--seq", "--stream", "."],
        "\u{1e}[[0],1]\n\u{1e}[[0]]\n",
        "",
        0,
    ),
    (
        "null",
        &["-c", "--seq", "--stream-errors", "."],
        "\u{1e}[\"Unfinished abandoned text at EOF at line 1, column 4\",[]]\n",
        "",
        0,
    ),
    (
        "\u{1e}\"ab",
        &["-c", "--seq", "--stream-errors", "."],
        "\u{1e}[\"Unfinished string at EOF at line 1, column 4\",[]]\n",
        "",
        0,
    ),
    (
        "\u{1e}{\"a\"\u{1e}[1]\n",
        &["-c", "--seq", "--stream-errors", "."],
        "\u{1e}[\"Truncated value at line 1, column 6\",[null]]\n\u{1e}[[0],1]\n\u{1e}[[0]]\n",
        "",
        0,
    ),
    (
        "\u{1e}[1,2\u{1e}[3]\n",
        &["-c", "--seq", "--stream", "--stream-errors", "length"],
        "\u{1e}2\n\u{1e}2\n\u{1e}2\n\u{1e}1\n",
        "",
        0,
    ),
];

#[test]
fn test_seq_records_are_read_as_stream_events_6115() -> Result<()> {
    for &(stdin, args, stdout, stderr, code) in ROWS {
        let got = run_jq_full(args, Some(stdin))?;
        assert_eq!(
            got,
            (stdout.to_string(), stderr.to_string(), code),
            "{args:?} over {stdin:?}"
        );
    }
    Ok(())
}

/// A BOM that stops short makes jq reset its parser at every call, so the close of the record is
/// lost: `--stream` yields `[[0],1]` alone. A whole BOM is skipped.
#[test]
fn test_seq_stream_after_a_malformed_bom_loses_what_each_call_left_6115() -> Result<()> {
    for (stdin, stdout) in [
        (&b"\xef\xbb\x1e[1]\n"[..], "\u{1e}[[0],1]\n"),
        (
            &b"\xef\xbb\xbf\x1e[1]\n"[..],
            "\u{1e}[[0],1]\n\u{1e}[[0]]\n",
        ),
    ] {
        let (output, code) = spawn_jq(&["-c", "--seq", "--stream", "."], Some(stdin))?;
        assert_eq!(
            (String::from_utf8(output.stdout)?.as_str(), code),
            (stdout, 0),
            "{stdin:?}"
        );
    }
    Ok(())
}
