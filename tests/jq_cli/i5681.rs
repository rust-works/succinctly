//! #5681: under `--stream` a filter reads each event as the tree the event is, instead of the
//! event being written out as text and indexed again for every leaf. The answers, the order of
//! the outputs, the number spellings of the leaves (`1.0`, `1E+2`, `-0`), a raised error and the
//! exit status must be the ones the indexed route gave and jq 1.7.1 gives. Every row was captured
//! from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(arguments, stdin, expected stdout, expected exit status)`.
type Row = (&'static [&'static str], &'static str, &'static str, i32);

const ROWS: &[Row] = &[
    (
        &[
            "-c",
            "--stream",
            "select(length==2 and .[0][1]==\"d\")|.[1]",
        ],
        r#"{"c":{"d":1.0,"e":[1,2]}}"#,
        r#"1.0"#,
        0,
    ),
    (
        &["-c", "--stream", "select(length==2)|.[1]"],
        r#"[1.0, 100000000000000000000, 1E2, -0, 0.10, "x"]"#,
        r#"1.0
100000000000000000000
1E+2
-0
0.10
"x""#,
        0,
    ),
    (
        &["-c", "--stream", "select(length==2)|.[1]|tojson"],
        r#"[1.0, 100000000000000000000, 1E2, -0, 0.10]"#,
        r#""1.0"
"100000000000000000000"
"1E+2"
"-0"
"0.10""#,
        0,
    ),
    (
        &["-c", "--stream", "select(length==2)|.[1]+0"],
        r#"[1.0, 100000000000000000000, 1E2, 0.10]"#,
        r#"1
1e+20
100
0.1"#,
        0,
    ),
    (
        &["-c", "--stream", "select(length==2)|[.[0],.[1]]|length"],
        r#"{"a":[1,{"b":2}]}"#,
        r#"2
2"#,
        0,
    ),
    (
        &["-c", "--stream", ".[0]|length"],
        r#"{"a":[1,{"b":2}],"c":null}"#,
        r#"2
3
3
2
1
1"#,
        0,
    ),
    (
        &["-c", "--stream", ".[0]|map(tostring)|join(\"/\")"],
        r#"{"a":[1,{"b":2}],"c":null}"#,
        r#""a/0"
"a/1/b"
"a/1/b"
"a/1"
"c"
"c""#,
        0,
    ),
    (
        &["-c", "--stream", "select(length==1)|.[0]|length"],
        r#"[[],{},[[]]]"#,
        r#"2
1"#,
        0,
    ),
    (
        &["-c", "--stream", "first(.[])"],
        r#"[1,[2]]"#,
        r#"[0]
[1,0]
[1,0]
[1]"#,
        0,
    ),
    (
        &["-c", "--stream", "[.[0][]|numbers]"],
        r#"{"a":[1,{"b":2}]}"#,
        r#"[0]
[1]
[1]
[1]
[]"#,
        0,
    ),
    (
        &["-c", "--stream", ".[1] as $x | $x"],
        r#"[1,"a",null]"#,
        r#"1
"a"
null
null"#,
        0,
    ),
    (
        &["-c", "--stream", "path(.[0])"],
        r#"[7]"#,
        r#"[0]
[0]"#,
        0,
    ),
    (
        &["-c", "--stream", ".[0] |= . + [9]"],
        r#"[7]"#,
        r#"[[0,9],7]
[[0,9]]"#,
        0,
    ),
    (
        &["-c", "--stream", "del(.[0][0])"],
        r#"{"a":[7]}"#,
        r#"[[0],7]
[[0]]
[[]]"#,
        0,
    ),
    (
        &["-c", "--stream", "limit(1;.[])"],
        r#"[7]"#,
        r#"[0]
[0]"#,
        0,
    ),
    (
        &["-c", "--stream", "label $f | .[] | ., break $f"],
        r#"[7,8]"#,
        r#"[0]
[1]
[1]"#,
        0,
    ),
    (
        &["-c", "--stream", "try error(\"x\") catch ."],
        r#"[7]"#,
        r#""x"
"x""#,
        0,
    ),
    (
        &["-c", "--stream", ".[1]|-."],
        r#"[7,-3]"#,
        r#"-7
3"#,
        5,
    ),
    (
        &["-c", "--stream", ".[0][0]|ltrimstr(\"a\")"],
        r#"{"ab":1}"#,
        r#""b"
"b""#,
        0,
    ),
    (
        &["-c", "--stream", "select(length==2)|.[1]|@json"],
        r#"["a\u0000b\u00e9\ud83d\ude00"]"#,
        r#""\"a\\u0000bé😀\"""#,
        0,
    ),
    (
        &["-c", "--stream", ".[0]|tostring"],
        r#"[[1],{"k":[2]}]"#,
        r#""[0,0]"
"[0,0]"
"[1,\"k\",0]"
"[1,\"k\",0]"
"[1,\"k\"]"
"[1]""#,
        0,
    ),
    (
        &["--stream", "-c", "1,2"],
        r#"[7]"#,
        r#"1
2
1
2"#,
        0,
    ),
    (&["--stream", "-c", "empty"], r#"[7]"#, r#""#, 0),
    (&["--stream", "-c", "error"], r#"[7] 8"#, r#""#, 5),
    (&["--stream", "-c", ".[1]|error"], r#"[7]"#, r#""#, 5),
    (
        &["--stream", "-c", "input_line_number"],
        r#"[7]
[8]
"#,
        r#"1
1
2
2"#,
        0,
    ),
];

#[test]
fn test_stream_events_read_in_place_by_a_filter_match_jq_5681() -> Result<()> {
    for (args, stdin, expected, code) in ROWS {
        let (stdout, stderr, status) = run_jq_full(args, Some(stdin))?;
        assert_eq!(
            (stdout.trim_end(), status),
            (*expected, *code),
            "{args:?} on {stdin:?}: stderr {stderr}"
        );
    }
    Ok(())
}
