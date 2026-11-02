//! #5840: a program that holds `empty`, `error(...)?` or `select`'s own definition spelled as an
//! `if`, run over a `--stream` event or an `-R` line, reads the input as the value it is instead of
//! writing it out as text and indexing it first. The answers, their order and the exit status must
//! be jq 1.7.1's. Every row was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(arguments, stdin, expected stdout, expected exit status)`.
type Row = (&'static [&'static str], &'static str, &'static str, i32);

const ROWS: &[Row] = &[
    (
        &["-c", "--stream", "empty"],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r"",
        0,
    ),
    (
        &["-c", "--stream", "empty | error(\"x\")"],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r"",
        0,
    ),
    (
        &["-c", "--stream", "error(\"x\")?"],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r"",
        0,
    ),
    (
        &["-c", "--stream", "error?"],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r"",
        0,
    ),
    (
        &["-c", "--stream", "try error(\"x\")"],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r"",
        0,
    ),
    (
        &["-c", "--stream", "(1, 2) | empty"],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r"",
        0,
    ),
    (
        &["-c", "--stream", "if length==2 then . else empty end"],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r#"[["a",0],1]
[["a",1],null]
[["a",2],false]
[["a",3],2.50]
[["b","c"],0]
[["d"],[]]
[["e"],"s"]"#,
        0,
    ),
    (
        &["-c", "--stream", "if length==2 then empty else . end"],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r#"[["a",3]]
[["b","c"]]
[["e"]]"#,
        0,
    ),
    (
        &["-c", "--stream", "if length==2 then .[1] else empty end"],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r#"1
null
false
2.50
0
[]
"s""#,
        0,
    ),
    (
        &["-c", "--stream", "if .[0][0]==\"a\" then . else empty end"],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r#"[["a",0],1]
[["a",1],null]
[["a",2],false]
[["a",3],2.50]
[["a",3]]"#,
        0,
    ),
    (
        &["-c", "--stream", ".[1] // empty"],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r#"1
2.50
0
[]
"s""#,
        0,
    ),
    (
        &["-c", "--stream", ".[0][1] // empty"],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r#"0
1
2
3
3
"c"
"c""#,
        0,
    ),
    (
        &[
            "-c",
            "--stream",
            "if length==2 then . else empty end | .[1]",
        ],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r#"1
null
false
2.50
0
[]
"s""#,
        0,
    ),
    (
        &[
            "-c",
            "--stream",
            "select(length==2) | if .[1] then . else empty end",
        ],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r#"[["a",0],1]
[["a",3],2.50]
[["b","c"],0]
[["d"],[]]
[["e"],"s"]"#,
        0,
    ),
    (
        &["-c", "--stream", "1, empty"],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r"1
1
1
1
1
1
1
1
1
1",
        0,
    ),
    (
        &[
            "-c",
            "--stream",
            "if length==2 then error(\"boom\") else empty end",
        ],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r"",
        0,
    ),
    (
        &["-c", "--stream", "if length==3 then . else empty end"],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r"",
        0,
    ),
    (
        &["-c", "--stream", ".[1] // empty | tojson"],
        r#"{"a":[1,null,false,2.50],"b":{"c":0},"d":[],"e":"s"}"#,
        r#""1"
"2.50"
"0"
"[]"
"\"s\"""#,
        0,
    ),
    (
        &["-c", "-R", "empty"],
        r"alpha

beta gamma
42
",
        r"",
        0,
    ),
    (
        &["-c", "-R", "if length>4 then . else empty end"],
        r"alpha

beta gamma
42
",
        r#""alpha"
"beta gamma""#,
        0,
    ),
    (
        &["-c", "-R", "if length>4 then empty else . end"],
        r"alpha

beta gamma
42
",
        r#"""
"42""#,
        0,
    ),
    (
        &["-c", "-R", ". // empty"],
        r"alpha

beta gamma
42
",
        r#""alpha"
""
"beta gamma"
"42""#,
        0,
    ),
    (
        &["-c", "-R", "error(\"x\")?"],
        r"alpha

beta gamma
42
",
        r"",
        0,
    ),
    (
        &["-c", "-R", "empty | input"],
        r"alpha

beta gamma
42
",
        r"",
        0,
    ),
];

#[test]
fn test_empty_and_select_spelled_as_if_match_jq_over_stream_events_and_lines_5840() -> Result<()> {
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
