//! #6061: `-s --stream` closes its array at the call that consumes the last byte of the input, so
//! a fault the stream ends in is met by a later call and comes after the array, which holds every
//! event read: a bare token before a last opener (`1[`), and a `,` or a closing bracket that
//! completes an event as the last byte (`[1,`, `[{}`). A fault met by the call that consumes the
//! last byte, or before it, is the whole result (`[1,2`, `1 [`). Every row is what `/usr/bin/jq`
//! 1.7.1 printed for the same stdin (stdout with its newlines as `\n`, the exit code, and the first
//! line of stderr).

use anyhow::Result;

use crate::run_jq_full;

/// `(stdin, stdout, exit code, the first line of stderr)`
type Row = (&'static str, &'static str, i32, &'static str);

const ROWS: &[Row] = &[
    (
        "1[",
        "[[[],1]]",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 2",
    ),
    (
        "1{",
        "[[[],1]]",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 2",
    ),
    (
        "12[",
        "[[[],12]]",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 3",
    ),
    (
        "1 2[",
        "[[[],1],[[],2]]",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 4",
    ),
    (
        "\"a\"1[",
        "[[[],\"a\"],[[],1]]",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 5",
    ),
    (
        "true[",
        "[[[],true]]",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 5",
    ),
    (
        "[1,",
        "[[[0],1]]",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 3",
    ),
    (
        "[1,2,",
        "[[[0],1],[[1],2]]",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 5",
    ),
    (
        "{\"a\":1,",
        "[[[\"a\"],1]]",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 7",
    ),
    (
        "[{}",
        "[[[0],{}]]",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 3",
    ),
    (
        "[[1]",
        "[[[0,0],1]]\\n[[0,0]]",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 4",
    ),
    (
        "1 [1,",
        "[[[],1],[[0],1]]",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 5",
    ),
    (
        "[1,\"a\",",
        "[[[0],1],[[1],\"a\"]]",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 7",
    ),
    (
        "[1,true,",
        "[[[0],1],[[1],true]]",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 8",
    ),
    (
        "1 [",
        "",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 3",
    ),
    (
        "\"a\"[",
        "",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 4",
    ),
    (
        "[1][",
        "",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 4",
    ),
    (
        "[1,2",
        "",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 4",
    ),
    (
        "[1",
        "",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 2",
    ),
    (
        "[1,2]]",
        "",
        5,
        "jq: parse error: Unmatched ']' at the top-level at line 1, column 6",
    ),
    (
        "1}",
        "",
        5,
        "jq: parse error: Unmatched '}' at the top-level at line 1, column 2",
    ),
    (
        "[1,}",
        "",
        5,
        "jq: parse error: Expected another key:value pair at line 1, column 4",
    ),
    (
        "{\"a\":1",
        "",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 6",
    ),
    (
        "[[1],",
        "",
        5,
        "jq: parse error: Unfinished JSON term at EOF at line 1, column 5",
    ),
    (
        "tru",
        "",
        5,
        "jq: parse error: Invalid literal at EOF at line 1, column 3",
    ),
    (
        "1 tru",
        "",
        5,
        "jq: parse error: Invalid literal at EOF at line 1, column 5",
    ),
    ("", "[]", 0, ""),
];

#[test]
fn a_stream_slurp_holds_the_events_read_before_a_fault_met_after_the_array_closed() -> Result<()> {
    for &(text, stdout, code, first_stderr) in ROWS {
        let (out, err, exit) = run_jq_full(&["-cs", "--stream", "."], Some(text))?;
        assert_eq!(
            out.trim_end().replace('\n', "\\n"),
            stdout,
            "{text:?}: stdout"
        );
        assert_eq!(exit, code, "{text:?}: exit code");
        assert_eq!(
            err.lines().next().unwrap_or(""),
            first_stderr,
            "{text:?}: stderr"
        );
    }
    Ok(())
}
