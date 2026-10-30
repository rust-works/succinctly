//! #5549: `tojson` writes its text into one buffer and a `del` of a static path runs on the owned
//! value, where both used to build a `String` per node and write the value out as JSON to index
//! it again. The text and the answer must not move. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -c`); a `del` that raises is caught, so the message is compared too.

use anyhow::Result;

use crate::run_jq_full;

/// `(stdin, filter, expected stdout)`; a `None` stdin runs the filter with `-n`.
type Row = (Option<&'static str>, &'static str, &'static str);

const ROWS: &[Row] = &[
    (
        Some(r#"{"users":[{"id":1},{"id":2}]}"#),
        r"del(.users[0]) | .users | length",
        r"1",
    ),
    (
        Some(r#"{"users":[{"id":1},{"id":2}]}"#),
        r"del(.users[0])",
        r#"{"users":[{"id":2}]}"#,
    ),
    (
        Some(r#"{"a":{"b":1,"c":2}}"#),
        r"del(.a.b)",
        r#"{"a":{"c":2}}"#,
    ),
    (Some(r#"{"a":1}"#), r"del(.b)", r#"{"a":1}"#),
    (Some(r"[1,2,3]"), r"del(.[-1])", r"[1,2]"),
    (Some(r"[1,2,3]"), r"del(.[7])", r"[1,2,3]"),
    (Some(r"[1,2,3]"), r"del(.[1])", r"[1,3]"),
    (Some(r"null"), r"del(.a)", r"null"),
    (
        Some(r#"{"a":{"b":{"c":1}}}"#),
        r"del(.a.b.c)",
        r#"{"a":{"b":{}}}"#,
    ),
    (
        Some(r#"{"a":[1,2,{"b":3}]}"#),
        r"del(.a[2].b)",
        r#"{"a":[1,2,{}]}"#,
    ),
    (
        Some(r#"{"a":[1,2,{"b":3}]}"#),
        r"del(.a[5].b)",
        r#"{"a":[1,2,{"b":3}]}"#,
    ),
    (Some(r#"{"a":null}"#), r"del(.a.b)", r#"{"a":null}"#),
    (
        Some(r#"[{"a":1},{"a":2}]"#),
        r"del(.[0].a)",
        r#"[{},{"a":2}]"#,
    ),
    (Some(r#"{"a":1}"#), r"del(.)", r"null"),
    (Some(r#"{"a":1}"#), r"del(.a) | .a", r"null"),
    (Some(r#"{"a":1}"#), r"[del(.a), del(.b)]", r#"[{},{"a":1}]"#),
    (
        Some(r#"{"x":0,"a":1.50,"b":100000000000000000000,"c":1e1000,"d":0.1,"e":1E2}"#),
        r"del(.x)",
        r#"{"a":1.50,"b":100000000000000000000,"c":1E+1000,"d":0.1,"e":1E+2}"#,
    ),
    (Some(r#"{"a":1,"a":2,"b":3}"#), r"del(.b)", r#"{"a":2}"#),
    (Some(r#"{"a":1,"a":2}"#), r"del(.a)", r"{}"),
    (Some(r#"{"a":[1,2]}"#), r"del(.a[0], .a[1])", r#"{"a":[]}"#),
    (
        Some(r"1"),
        r"try del(.a) catch .",
        r#""Cannot index number with string \"a\"""#,
    ),
    (
        Some(r#""s""#),
        r"try del(.[0]) catch .",
        r#""Cannot index string with number""#,
    ),
    (
        Some(r#"{"a":1}"#),
        r"try del(.[0]) catch .",
        r#""Cannot index object with number""#,
    ),
    (
        Some(r"[1]"),
        r"try del(.a) catch .",
        r#""Cannot index array with string \"a\"""#,
    ),
    (
        Some(r#"{"a":1}"#),
        r"try del(.a.b) catch .",
        r#""Cannot index number with string \"b\"""#,
    ),
    (
        Some(r#"{"a":"x"}"#),
        r"try del(.a[0]) catch .",
        r#""Cannot index string with number""#,
    ),
    (Some(r#"{"a":1}"#), r"del(.a.b)?", r""),
    (
        None,
        r#"[nan, infinite, -infinite, 1.0, 1e1000, -0, "é\u0001\u007f😀", [], {}, null, true] | tojson"#,
        r#""[null,1.7976931348623157e+308,-1.7976931348623157e+308,1.0,1E+1000,-0,\"é\\u0001\\u007f😀\",[],{},null,true]""#,
    ),
    (
        None,
        r"[nan, infinite, -infinite] | map(. * 1) | tojson",
        r#""[null,1.7976931348623157e+308,-1.7976931348623157e+308]""#,
    ),
    (
        Some(r#"{"a":[1.50,2e2,-0.0],"b\"":"q\n"}"#),
        r"tojson",
        r#""{\"a\":[1.50,2E+2,-0.0],\"b\\\"\":\"q\\n\"}""#,
    ),
    (
        Some(r#"{"a":[1.50,2e2,-0.0],"b\"":"q\n"}"#),
        r"tojson | fromjson",
        r#"{"a":[1.50,2E+2,-0.0],"b\"":"q\n"}"#,
    ),
    (
        Some(r#"{"a":[1.50,2e2,-0.0],"b\"":"q\n"}"#),
        r"tostring",
        r#""{\"a\":[1.50,2E+2,-0.0],\"b\\\"\":\"q\\n\"}""#,
    ),
    (
        Some(r#"{"a":100000000000000000000,"b":1.0}"#),
        r"tojson",
        r#""{\"a\":100000000000000000000,\"b\":1.0}""#,
    ),
    (
        Some(r"[0.1,0.2]"),
        r"add | tojson",
        r#""0.30000000000000004""#,
    ),
    (Some(r"[1,2]"), r"map(. + 0.5) | tojson", r#""[1.5,2.5]""#),
    (
        Some(r#"[1,[2,{"a":"é"}]]"#),
        r"tostring",
        r#""[1,[2,{\"a\":\"é\"}]]""#,
    ),
    (
        Some(r#"{"a":[1,{"b":2}]}"#),
        r#""v=\(.a)""#,
        r#""v=[1,{\"b\":2}]""#,
    ),
    (
        Some(r#"{"a":[1,{"b":2}]}"#),
        r"@json",
        r#""{\"a\":[1,{\"b\":2}]}""#,
    ),
    (
        Some(r#"{"a":[1,{"b":2}]}"#),
        r"@text",
        r#""{\"a\":[1,{\"b\":2}]}""#,
    ),
    (
        Some(r"[1e1000,-1e1000]"),
        r"tojson",
        r#""[1E+1000,-1E+1000]""#,
    ),
    (
        None,
        r"reduce range(256) as $i (0; [.]) | tojson | length",
        r"513",
    ),
    (
        None,
        r"reduce range(300) as $i (0; [.]) | tojson | length",
        r"533",
    ),
    (
        None,
        r"reduce range(300) as $i (0; {a: .}) | tojson | .[-30:]",
        r#""}}}}}}}}}}}}}}}}}}}}}}}}}}}}}}""#,
    ),
];

const IDENTITY_ROWS: &[Row] = &[
    (None, r"[nan] | del(.[1]) | .[0] == .[0]", r"false"),
    (None, r#"{"a":[nan],"b":1} | del(.b) | .a == .a"#, r"true"),
    (
        None,
        r#"{"a":[nan],"b":1} | del(.b) | .a[0] == .a[0]"#,
        r"false",
    ),
    (None, r"[[nan],1] | del(.[1]) | .[0] == .[0]", r"true"),
    (
        None,
        r"[[nan],1] | del(.[1]) | (.[0] | .[0]) == (.[0] | .[0])",
        r"false",
    ),
    (
        None,
        r#"{"a":{"b":[nan]},"c":1} | del(.c) | [.a.b] == [.a.b]"#,
        r"true",
    ),
    (None, r"[nan] | [., .] | del(.[2]) | .[0] == .[1]", r"true"),
    (
        None,
        r"[nan] as $a | {y:$a,z:1} | del(.z) | .y == $a",
        r"true",
    ),
];

fn check(rows: &[Row]) -> Result<()> {
    for (stdin, filter, expected) in rows {
        let (stdout, stderr, code) = match stdin {
            Some(input) => run_jq_full(&["-c", filter], Some(input))?,
            None => run_jq_full(&["-nc", filter], None)?,
        };
        assert_eq!(
            (stdout.trim_end(), code),
            (*expected, 0),
            "{filter} on {stdin:?}: stderr {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_tojson_and_static_del_match_jq_5549() -> Result<()> {
    check(ROWS)
}

/// A NaN is equal to itself only as the very node it is, so these read a value back through a
/// `del` and compare it with itself: the answer is jq's `jv_identical` short-circuit, which the
/// unshared-containers holdout has no node to give.
#[test]
#[cfg_attr(
    feature = "unshared-containers",
    ignore = "holdout has no node identity or sharing (#5254)"
)]
fn test_del_keeps_node_identity_like_jq_5549() -> Result<()> {
    check(IDENTITY_ROWS)
}
