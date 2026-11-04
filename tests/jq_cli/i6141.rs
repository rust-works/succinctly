//! #6141: under `--seq --stream`, jq's end-of-stream stop (#2998: the first call after the read that
//! hit the end of the file returns nothing when it meets an empty record) ends only the call that
//! met it. A program that reads inputs finds a barrier there -- `input` raises `break`, `inputs`
//! ends at it, and the main loop ends only if it is the one that pops it -- and the stream goes on
//! behind it, so what follows (a value, a fault a later `input` raises, the EOF report) is not lost.
//! A run that never reads an input ends at the stop. Rows, messages and exit statuses were captured
//! from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(stdin, flags, filter, stdout, stderr, exit status)`.
type Row = (
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
    &'static str,
    i32,
);

const ROWS: &[Row] = &[
    (
        ",\"a\"nultrue}-\x1e[1]\n\x1e[]\\",
        &["-c", "-e"],
        "limit(2;inputs)",
        "\x1e[[0]]\n",
        "jq: error (at <stdin>:1): Invalid numeric literal at EOF at line 2, column 4\n",
        5,
    ),
    (
        "\x1e[1]\n\x1e[2",
        &["-c"],
        "[inputs]",
        "\x1e[[[0]]]\n",
        "jq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 3\n",
        0,
    ),
    (
        "\x1e[1]\n\x1e[2",
        &["-c", "-e"],
        "[inputs]",
        "\x1e[[[0]]]\n",
        "jq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 3\n",
        0,
    ),
    (
        "\x1e[1]\n\x1e[2",
        &["-c", "-n"],
        "[inputs]",
        "\x1e[[[0],1],[[0]]]\n",
        "",
        0,
    ),
    (
        "\x1e[1]\n\x1e\x1e[2]\n",
        &["-c"],
        "[inputs]",
        "\x1e[[[0]],[[0],2],[[0]]]\n",
        "",
        0,
    ),
    (
        "\x1e[1]\n\x1e\x1e[2]\n",
        &["-c"],
        ".",
        "\x1e[[0],1]\n\x1e[[0]]\n\x1e[[0],2]\n\x1e[[0]]\n",
        "",
        0,
    ),
    (
        "\x1e[1]\n\x1e\x1e[2]\n",
        &["-c"],
        "[.,(try input catch .)]",
        "\x1e[[[0],1],[[0]]]\n\x1e[[[0],2],[[0]]]\n",
        "",
        0,
    ),
    (
        "\x1e[1]\n\x1e\x1e[2]\n",
        &["-c"],
        "first(inputs)",
        "\x1e[[0]]\n\x1e[[0]]\n",
        "",
        0,
    ),
    (
        "\x1e[1]\n\x1e\x1e[2]\n",
        &["-c", "-n"],
        "input, input",
        "\x1e[[0],1]\n\x1e[[0]]\n",
        "",
        0,
    ),
    (
        "\x1e[1]\n\x1e\x1e[2]\n",
        &["-c", "-n"],
        "[limit(5;repeat(try input catch .))]",
        "\x1e[[[0],1],[[0]],[[0],2],[[0]],\"break\"]\n",
        "",
        0,
    ),
    (
        "\x1e[1]\n\x1e[1,",
        &["-c"],
        "[.,input]",
        "\x1e[[[0],1],[[0]]]\n",
        "",
        0,
    ),
    (
        "\x1e[1]\n\x1e\x1e[2]\n",
        &["-c", "--stream-errors"],
        "[inputs]",
        "\x1e[[[0]],[[0],2],[[0]]]\n",
        "",
        0,
    ),
    (
        "\x1e[1]\n\x1e[2",
        &["-c", "--stream-errors"],
        "[inputs]",
        "\x1e[[[0]]]\n\x1e[]\n",
        "",
        0,
    ),
    (
        "\x1e[1]\n\x1e\x1e[2",
        &["-c", "-s"],
        ".",
        "\x1e[[[0],1],[[0]]]\n",
        "jq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 4\n",
        0,
    ),
    ("\x1e[1]\n\x1e", &["-c"], "[inputs]", "\x1e[[[0]]]\n", "", 0),
    (
        "\x1e[1]\n\x1e",
        &["-c", "-n"],
        "[inputs],input_line_number",
        "\x1e[[[0],1],[[0]]]\n\x1e1\n",
        "",
        0,
    ),
    (
        "\x1e[1]\n\x1e\x1e[2]\n",
        &["-c"],
        "[inputs],input_line_number,input_filename",
        "\x1e[[[0]],[[0],2],[[0]]]\n\x1e2\n\x1e\"<stdin>\"\n",
        "",
        0,
    ),
    (
        "\x1e[1]\n\x1e\x1e[2]\n",
        &["-c", "-n"],
        "(try input catch .),input_line_number,input",
        "\x1e[[0],1]\n\x1e1\n\x1e[[0]]\n",
        "",
        0,
    ),
];

#[test]
fn test_seq_stream_end_of_stream_stop_is_a_barrier_6141() -> Result<()> {
    for (stdin, flags, filter, stdout, stderr, code) in ROWS {
        let mut args = vec!["--seq", "--stream"];
        args.extend_from_slice(flags);
        args.push(filter);
        let (out, err, status) = run_jq_full(&args, Some(stdin))?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (*stdout, *stderr, *code),
            "{args:?} {stdin:?}"
        );
    }
    Ok(())
}
