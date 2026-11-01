//! #5670: `del` of several static paths (`del(.a, .b)`, `del((.a, .b), .c)`, `del(.[0], .[1])`) is
//! answered on the owned value, as `del` of one path is (#5549), instead of writing the value out as
//! JSON, indexing it and deleting from that. jq collects every path from the input before it deletes
//! any, so the answer does not depend on the order written (`del(.[1], .[0])` removes the same
//! two). Expected values were captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_del_of_several_static_paths_matches_jq_5670() -> Result<()> {
    for (program, input, expected) in [
        (
            r"del(.a, .b)",
            r#"{"a":{"b":1,"c":[1,2,3]},"b":[10,20,30],"c":null,"d":1.0}"#,
            r#"{"c":null,"d":1.0}"#,
        ),
        (
            r"del(.a, .a.b)",
            r#"{"a":{"b":1,"c":[1,2,3]},"b":[10,20,30],"c":null,"d":1.0}"#,
            r#"{"b":[10,20,30],"c":null,"d":1.0}"#,
        ),
        (
            r"del(.a.b, .a)",
            r#"{"a":{"b":1,"c":[1,2,3]},"b":[10,20,30],"c":null,"d":1.0}"#,
            r#"{"b":[10,20,30],"c":null,"d":1.0}"#,
        ),
        (
            r"del(.a.c[0], .a.c[2])",
            r#"{"a":{"b":1,"c":[1,2,3]},"b":[10,20,30],"c":null,"d":1.0}"#,
            r#"{"a":{"b":1,"c":[2]},"b":[10,20,30],"c":null,"d":1.0}"#,
        ),
        (
            r"del(.b[0], .b[1])",
            r#"{"a":{"b":1,"c":[1,2,3]},"b":[10,20,30],"c":null,"d":1.0}"#,
            r#"{"a":{"b":1,"c":[1,2,3]},"b":[30],"c":null,"d":1.0}"#,
        ),
        (
            r"del((.a, .b), .c)",
            r#"{"a":{"b":1,"c":[1,2,3]},"b":[10,20,30],"c":null,"d":1.0}"#,
            r#"{"d":1.0}"#,
        ),
        (
            r"del(.a, .b, .c, .d)",
            r#"{"a":{"b":1,"c":[1,2,3]},"b":[10,20,30],"c":null,"d":1.0}"#,
            r"{}",
        ),
        (
            r"del(.x, .y)",
            r#"{"a":{"b":1,"c":[1,2,3]},"b":[10,20,30],"c":null,"d":1.0}"#,
            r#"{"a":{"b":1,"c":[1,2,3]},"b":[10,20,30],"c":null,"d":1.0}"#,
        ),
        (
            r"del(.a, .a)",
            r#"{"a":{"b":1,"c":[1,2,3]},"b":[10,20,30],"c":null,"d":1.0}"#,
            r#"{"b":[10,20,30],"c":null,"d":1.0}"#,
        ),
        (r"del(.[0], .[1])", r"[1,2,3,4,5]", r"[3,4,5]"),
        (r"del(.[1], .[0])", r"[1,2,3,4,5]", r"[3,4,5]"),
        (r"del(.[0], .[-1])", r"[1,2,3,4,5]", r"[2,3,4]"),
        (r"del(.[0], .[0])", r"[1,2,3,4,5]", r"[2,3,4,5]"),
        (r"del(.[5], .[6])", r"[1,2,3,4,5]", r"[1,2,3,4,5]"),
        (r"del(.[-1], .[-2], .[-3])", r"[1,2,3,4,5]", r"[1,2]"),
        (r"del(.[1], .[3])", r"[1,2,3,4,5]", r"[1,3,5]"),
        (
            r".users|map(del(.name, .score))",
            r#"{"users":[{"id":1,"name":"a","score":1.50,"ratio":0.30000000000000004},{"id":2,"name":"b","score":2,"ratio":1e19},{"id":3,"name":"c","score":-0,"ratio":null}]}"#,
            r#"[{"id":1,"ratio":0.30000000000000004},{"id":2,"ratio":1E+19},{"id":3,"ratio":null}]"#,
        ),
        (
            r".users|map(del(.id, .ratio))",
            r#"{"users":[{"id":1,"name":"a","score":1.50,"ratio":0.30000000000000004},{"id":2,"name":"b","score":2,"ratio":1e19},{"id":3,"name":"c","score":-0,"ratio":null}]}"#,
            r#"[{"name":"a","score":1.50},{"name":"b","score":2},{"name":"c","score":-0}]"#,
        ),
        (
            r".users|del(.[0], .[2])",
            r#"{"users":[{"id":1,"name":"a","score":1.50,"ratio":0.30000000000000004},{"id":2,"name":"b","score":2,"ratio":1e19},{"id":3,"name":"c","score":-0,"ratio":null}]}"#,
            r#"[{"id":2,"name":"b","score":2,"ratio":1E+19}]"#,
        ),
        (
            r"del(.users[0].id, .users[0].name)",
            r#"{"users":[{"id":1,"name":"a","score":1.50,"ratio":0.30000000000000004},{"id":2,"name":"b","score":2,"ratio":1e19},{"id":3,"name":"c","score":-0,"ratio":null}]}"#,
            r#"{"users":[{"score":1.50,"ratio":0.30000000000000004},{"id":2,"name":"b","score":2,"ratio":1E+19},{"id":3,"name":"c","score":-0,"ratio":null}]}"#,
        ),
        (
            r"del(.users[0], .users[1].id)",
            r#"{"users":[{"id":1,"name":"a","score":1.50,"ratio":0.30000000000000004},{"id":2,"name":"b","score":2,"ratio":1e19},{"id":3,"name":"c","score":-0,"ratio":null}]}"#,
            r#"{"users":[{"name":"b","score":2,"ratio":1E+19},{"id":3,"name":"c","score":-0,"ratio":null}]}"#,
        ),
        (
            r".users|map(del(.name, .score)|length)",
            r#"{"users":[{"id":1,"name":"a","score":1.50,"ratio":0.30000000000000004},{"id":2,"name":"b","score":2,"ratio":1e19},{"id":3,"name":"c","score":-0,"ratio":null}]}"#,
            r"[2,2,2]",
        ),
        (
            r".users|map(del(.name, .score)|tojson)",
            r#"{"users":[{"id":1,"name":"a","score":1.50,"ratio":0.30000000000000004},{"id":2,"name":"b","score":2,"ratio":1e19},{"id":3,"name":"c","score":-0,"ratio":null}]}"#,
            r#"["{\"id\":1,\"ratio\":0.30000000000000004}","{\"id\":2,\"ratio\":1E+19}","{\"id\":3,\"ratio\":null}"]"#,
        ),
        (r"del(.a, .b)", r"null", r"null"),
        (r"del(.a, .b)", r"{}", r"{}"),
        (r"del(.[0], .[1])", r"[]", r"[]"),
        (r"del(.[0], .[1])", r"null", r"null"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(input))?;
        assert_eq!(code, 0, "{program}: {stderr:?}");
        assert_eq!(stdout.trim_end(), expected, "{program} over {input}");
    }
    Ok(())
}

/// A value `del` cannot delete from keeps the evaluator's message and exit status.
#[test]
fn test_del_of_several_paths_over_the_wrong_type_reports_jq_s_error_5670() -> Result<()> {
    for (program, input, message) in [
        (
            r"del(.a, .b)",
            r"1",
            r#"jq: error (at <stdin>:0): Cannot index number with string "a""#,
        ),
        (
            r"del(.a, .b)",
            r#""s""#,
            r#"jq: error (at <stdin>:0): Cannot index string with string "a""#,
        ),
        (
            r"del(.[0], .[1])",
            r#"{"a":1}"#,
            r"jq: error (at <stdin>:0): Cannot index object with number",
        ),
        (
            r"del(.a, .b)",
            r"[1,2]",
            r#"jq: error (at <stdin>:0): Cannot index array with string "a""#,
        ),
        (
            r"del(.a, .b)",
            r"true",
            r#"jq: error (at <stdin>:0): Cannot index boolean with string "a""#,
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(input))?;
        assert_eq!(code, 5, "{program}");
        assert_eq!(stdout.trim_end(), "", "{program}");
        assert_eq!(stderr.trim_end(), message, "{program}");
    }
    Ok(())
}

/// `delpaths` of an array literal of paths is answered on the owned value too: it is the builtin's
/// own delete (one sorted batch), so the order written does not matter and overlapping paths and
/// repeated indices are deleted together. Expected values were captured from jq 1.7.1.
#[test]
fn test_delpaths_of_literal_paths_matches_jq_5670() -> Result<()> {
    let object = r#"{"a":{"b":1,"c":[1,2,3]},"b":[10,20,30],"c":null,"d":1.0}"#;
    let users = r#"{"users":[{"id":1,"name":"a","score":1.50,"ratio":0.30000000000000004},{"id":2,"name":"b","score":2,"ratio":1e19},{"id":3,"name":"c","score":-0,"ratio":null}]}"#;
    for (program, input, expected) in [
        (
            r#"delpaths([["a"],["b"]])"#,
            object,
            r#"{"c":null,"d":1.0}"#,
        ),
        (
            r#"delpaths([["a"],["a","b"]])"#,
            object,
            r#"{"b":[10,20,30],"c":null,"d":1.0}"#,
        ),
        (
            r#"delpaths([["a","b"],["a"]])"#,
            object,
            r#"{"b":[10,20,30],"c":null,"d":1.0}"#,
        ),
        (
            r#"delpaths([["a","c",0],["a","c",2]])"#,
            object,
            r#"{"a":{"b":1,"c":[2]},"b":[10,20,30],"c":null,"d":1.0}"#,
        ),
        (
            r#"delpaths([["b",0],["b",1]])"#,
            object,
            r#"{"a":{"b":1,"c":[1,2,3]},"b":[30],"c":null,"d":1.0}"#,
        ),
        (
            r#"delpaths([["b",1],["b",0]])"#,
            object,
            r#"{"a":{"b":1,"c":[1,2,3]},"b":[30],"c":null,"d":1.0}"#,
        ),
        (r#"delpaths([["x"],["y"]])"#, object, object),
        (
            r#"delpaths([["a"],["a"]])"#,
            object,
            r#"{"b":[10,20,30],"c":null,"d":1.0}"#,
        ),
        (r#"delpaths([["b",5]])"#, object, object),
        // The empty path is the whole value.
        (r#"delpaths([["a"],[]])"#, object, "null"),
        (r"delpaths([[0],[1]])", r"[1,2,3,4,5]", r"[3,4,5]"),
        (r"delpaths([[1],[0]])", r"[1,2,3,4,5]", r"[3,4,5]"),
        (r"delpaths([[0],[0]])", r"[1,2,3,4,5]", r"[2,3,4,5]"),
        (r"delpaths([[5],[6]])", r"[1,2,3,4,5]", r"[1,2,3,4,5]"),
        (r"delpaths([[1],[3]])", r"[1,2,3,4,5]", r"[1,3,5]"),
        (r"delpaths([[0],[-1]])", r"[1,2,3,4,5]", r"[2,3,4]"),
        (r"delpaths([[-1],[4]])", r"[1,2,3,4,5]", r"[1,2,3,4]"),
        (r"delpaths([[-1],[-2]])", r"[1,2,3,4,5]", r"[1,2,3]"),
        (
            r#".users|map(delpaths([["name"],["score"]]))"#,
            users,
            r#"[{"id":1,"ratio":0.30000000000000004},{"id":2,"ratio":1E+19},{"id":3,"ratio":null}]"#,
        ),
        (
            r#"delpaths([["users",0],["users",2]])"#,
            users,
            r#"{"users":[{"id":2,"name":"b","score":2,"ratio":1E+19}]}"#,
        ),
        (
            r#"delpaths([["users",0,"id"],["users",0,"name"]])"#,
            users,
            r#"{"users":[{"score":1.50,"ratio":0.30000000000000004},{"id":2,"name":"b","score":2,"ratio":1E+19},{"id":3,"name":"c","score":-0,"ratio":null}]}"#,
        ),
        (
            r#".users|map(delpaths([["id"],["ratio"]])|tojson)"#,
            users,
            r#"["{\"name\":\"a\",\"score\":1.50}","{\"name\":\"b\",\"score\":2}","{\"name\":\"c\",\"score\":-0}"]"#,
        ),
        (r#"delpaths([["a"],["b"]])"#, r#"{"a":1}"#, r"{}"),
        (r"delpaths([[0],[1]])", r"[1,2]", r"[]"),
        (r#"delpaths([["a"],["b"]])"#, r"null", r"null"),
        (r"delpaths([[0],[1]])", r"null", r"null"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(input))?;
        assert_eq!(code, 0, "{program}: {stderr:?}");
        assert_eq!(stdout.trim_end(), expected, "{program} over {input}");
    }
    Ok(())
}

/// A value `delpaths` cannot delete from keeps jq's message and exit status, and `?` or `try`
/// swallows it, whichever route answered.
#[test]
fn test_delpaths_of_literal_paths_over_the_wrong_type_reports_jq_s_error_5670() -> Result<()> {
    for (program, input, message) in [
        (
            r#"delpaths([["a"],["b"]])"#,
            r"1",
            "jq: error (at <stdin>:0): Cannot delete fields from number",
        ),
        (
            r"delpaths([[0],[1]])",
            r#""s""#,
            "jq: error (at <stdin>:0): Cannot delete fields from string",
        ),
        (
            r#"delpaths([["a"],["b"]])"#,
            r"true",
            "jq: error (at <stdin>:0): Cannot delete fields from boolean",
        ),
        (
            r#"delpaths([["a"],["b"]])"#,
            r"[1,2]",
            "jq: error (at <stdin>:0): Cannot delete string element of array",
        ),
        (
            r"delpaths([[0],[1]])",
            r#"{"a":1}"#,
            "jq: error (at <stdin>:0): Cannot delete number field of object",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(input))?;
        assert_eq!(code, 5, "{program}");
        assert_eq!(stdout.trim_end(), "", "{program}");
        assert_eq!(stderr.trim_end(), message, "{program}");

        let (stdout, _, code) = run_jq_full(
            &["-c", &format!("try ({program}) catch \"caught\"")],
            Some(input),
        )?;
        assert_eq!(code, 0, "try {program}");
        assert_eq!(stdout.trim_end(), r#""caught""#, "try {program}");

        let (stdout, _, code) = run_jq_full(&["-c", &format!("[{program}?]")], Some(input))?;
        assert_eq!(code, 0, "{program}?");
        assert_eq!(stdout.trim_end(), "[]", "{program}?");
    }
    Ok(())
}
