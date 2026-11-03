//! #6061: `-s --stream` closes its array at the call that consumes the last byte of the input, so
//! a fault the stream ends in is met by a later call and comes after the array, which holds every
//! event read: a bare token before a last opener (`1[`), and a `,` or a closing bracket that
//! completes an event as the last byte (`[1,`, `[{}`). A fault met by the call that consumes the
//! last byte, or before it, is the whole result (`[1,2`, `1 [`). Every row is what `/usr/bin/jq`
//! 1.7.1 printed for the same stdin (stdout with its newlines as `\n`, the exit code, and the first
//! line of stderr).

use anyhow::Result;

use crate::{run_jq_full, run_jq_over_files};

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

/// `(flags, filter, stdin, stdout, exit code, the first line of stderr)`: a fault after more than
/// the eight events the stream reader keeps locations for, one on a later line than the array's
/// close, one that `input` and `[inputs]` reach after the array, and `-e`.
type ShapeRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    i32,
    &'static str,
);

const SHAPE_ROWS: &[ShapeRow] = &[
    ("-cs", ".", "[1,\n}\n1 2", "", 5, "jq: parse error: Expected another key:value pair at line 2, column 1"),
    ("-cs", ".", "[1,\n}\n1", "", 5, "jq: parse error: Expected another key:value pair at line 2, column 1"),
    ("-cs", ".", "[1,\n1[", "", 5, "jq: parse error: Expected a separator between values at line 2, column 2"),
    ("-cs", ".", "[1,\n}\n[2]", "", 5, "jq: parse error: Expected another key:value pair at line 2, column 1"),
    ("-cs", ".", "[1,\n2,\n}\n3", "", 5, "jq: parse error: Expected another key:value pair at line 3, column 1"),
    ("-cs", ".", "[1,}2", "", 5, "jq: parse error: Expected another key:value pair at line 1, column 4"),
    ("-cs", ".", "1 ]2", "", 5, "jq: parse error: Unmatched ']' at the top-level at line 1, column 3"),
    ("-cs", ".", "[1,2,3,4,5,6,7,8,9,10,11,12,", "[[[0],1],[[1],2],[[2],3],[[3],4],[[4],5],[[5],6],[[6],7],[[7],8],[[8],9],[[9],10],[[10],11],[[11],12]]", 5, "jq: parse error: Unfinished JSON term at EOF at line 1, column 28"),
    ("-cs", ".", "1 2 3 4 5 6 7 8 9 10 11 12 [", "", 5, "jq: parse error: Unfinished JSON term at EOF at line 1, column 28"),
    ("-cs", "input", "1[", "", 5, "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 2"),
    ("-cs", "input", "[1,", "", 5, "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 3"),
    ("-cs", "[inputs]", "[1,", "", 5, "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 3"),
    ("-cs", "try input catch .", "1[", "\"Unfinished JSON term at EOF at line 1, column 2\"", 0, ""),
    ("-cse", ".", "1[", "[[[],1]]", 5, "jq: parse error: Unfinished JSON term at EOF at line 1, column 2"),
    ("-cse", ".", "[1,2", "", 5, "jq: parse error: Unfinished JSON term at EOF at line 1, column 4"),
    ("-cs", "length", "1[", "1", 5, "jq: parse error: Unfinished JSON term at EOF at line 1, column 2"),

];

#[test]
fn a_stream_slurp_fault_after_the_array_reads_as_jq_does_across_lines_programs_and_flags(
) -> Result<()> {
    for &(flags, filter, text, stdout, code, first_stderr) in SHAPE_ROWS {
        let (out, err, exit) = run_jq_full(&[flags, "--stream", filter], Some(text))?;
        assert_eq!(
            out.trim_end().replace('\n', "\\n"),
            stdout,
            "{flags} {filter} {text:?}: stdout"
        );
        assert_eq!(exit, code, "{flags} {filter} {text:?}: exit code");
        assert_eq!(
            err.lines().next().unwrap_or(""),
            first_stderr,
            "{flags} {filter} {text:?}: stderr"
        );
    }
    Ok(())
}

/// jq reads the files as one stream, so the array closes at the last byte of the last one.
#[test]
fn a_stream_slurp_over_several_files_closes_its_array_at_the_last_byte_of_the_last() -> Result<()> {
    let rows: &[(&str, &str, &str, i32)] = &[
        ("[1]", "[2,", "[[[0],1],[[0]],[[0],2]]", 5),
        ("[1,", "[2]", "[[[0],1],[[1,0],2]]\\n[[1,0]]", 5),
        ("1", "2[", "[[[],12]]", 5),
        ("[1]\n", "}\n3", "", 5),
        ("1 2", "3 [", "", 5),
    ];
    for &(first, second, stdout, code) in rows {
        let (out, _, exit, _) = run_jq_over_files(&["-cs", "--stream", "."], &[first, second])?;
        assert_eq!(
            out.trim_end().replace('\n', "\\n"),
            stdout,
            "{first:?} {second:?}: stdout"
        );
        assert_eq!(exit, code, "{first:?} {second:?}: exit code");
    }
    Ok(())
}
