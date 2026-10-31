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
