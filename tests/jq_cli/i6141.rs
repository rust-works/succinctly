//! #6141: under `--seq --stream`, jq 1.7.1's end-of-stream stop (#2998: the first call after the
//! read that hit the end of the file returned nothing when it met an empty record) ended only the call
//! that met it, and a program that read inputs found a barrier there (`input` raised `break`). jq 1.8.2
//! reads past every empty record (#7718), and so does `succinctly jq`: these rows were captured from
//! jq 1.8.2 (`/opt/homebrew/bin/jq`), except the `<unknown>` of a read past the last value, which is
//! jq 1.7.1's (#7607).

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
        "\x1e[[0]]\n\x1e[[],[]]\n",
        "jq: ignoring parse error: Invalid numeric literal at EOF at line 2, column 4\n",
        0,
    ),
    (
        "\x1e[1]\n\x1e[2",
        &["-c"],
        "[inputs]",
        "",
        "jq: error (at <stdin>:1): Unfinished JSON term at EOF at line 2, column 3\n",
        5,
    ),
    (
        "\x1e[1]\n\x1e[2",
        &["-c", "-e"],
        "[inputs]",
        "",
        "jq: error (at <stdin>:1): Unfinished JSON term at EOF at line 2, column 3\n",
        5,
    ),
    (
        "\x1e[1]\n\x1e[2",
        &["-c", "-n"],
        "[inputs]",
        "",
        "jq: error (at <stdin>:1): Unfinished JSON term at EOF at line 2, column 3\n",
        5,
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
    // jq 1.8.2 names a file and line; the position after the last value is jq 1.7.1's (#7607).
    (
        "\x1e[1]\n\x1e[1,",
        &["-c"],
        "[.,input]",
        "\x1e[[[0],1],[[0]]]\n",
        "jq: error (at <unknown>): Unfinished JSON term at EOF at line 2, column 4\n",
        5,
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
        "\x1e[[[0]],[\"Unfinished JSON term at EOF at line 2, column 3\",[0]]]\n",
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
fn test_seq_stream_reads_past_an_empty_record_6141() -> Result<()> {
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
