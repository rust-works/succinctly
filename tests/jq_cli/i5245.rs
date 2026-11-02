//! #5245: a NaN index key held in a variable is a path component, not a refusal.
//!
//! jq's `path(.[$v])` with `$v = nan` is `[null]`; only a write that reaches the component refuses it, with `Cannot set array
//! element at NaN index` (`jv_array_set`). The shipped walker refused at resolution, so `path()` raised where jq answers.
//! Every row of the first table is captured from jq 1.7.1 (stdout, the first error line, the exit code).

use anyhow::Result;

use crate::run_jq_full;

#[test]
#[allow(clippy::type_complexity)] // a table of captured jq answers
fn test_nan_variable_key_is_a_path_component_and_a_write_refusal_5245() -> Result<()> {
    let rows: &[(&str, &str, &str, &str, i32)] = &[
        ("null", "nan as $v | path(.[$v])", "[null]\n", "", 0),
        ("[1,2]", "nan as $v | path(.[$v])", "[null]\n", "", 0),
        ("[]", "nan as $v | path(.[$v])", "[null]\n", "", 0),
        (
            "[1,2]",
            "nan as $v | path(.[$v].x)",
            "[null,\"x\"]\n",
            "",
            0,
        ),
        ("null", "nan as $v | path(.[$v][0])", "[null,0]\n", "", 0),
        ("[1,2]", "nan as $v | path(.[$v]?)", "[null]\n", "", 0),
        (
            "[1,2]",
            "nan as $v | [path(.[$v], .[0], .[$v])]",
            "[[null],[0],[null]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            "nan as $v | path(.a[$v])",
            "[\"a\",null]\n",
            "",
            0,
        ),
        (
            "[1,2]",
            "(nan, 0) as $v | [path(.[$v])]",
            "[[null]]\n[[0]]\n",
            "",
            0,
        ),
        (
            "[1,2]",
            "nan as $v | path(.[$v]) as $p | getpath($p)",
            "null\n",
            "",
            0,
        ),
        (
            "[1,2]",
            "nan as $v | [limit(1; path(.[$v]))]",
            "[[null]]\n",
            "",
            0,
        ),
        ("[1,2]", "nan as $v | path(.[$v] // .[0])", "[0]\n", "", 0),
        (
            "[1,2]",
            "nan as $v | path(first(.[$v], .[0]))",
            "[null]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            "nan as $v | path(.[$v])",
            "",
            "Cannot index object with number",
            5,
        ),
        (r#"{"a":1}"#, "nan as $v | path(.[$v]?)", "", "", 0),
        (
            "5",
            "nan as $v | path(.[$v])",
            "",
            "Cannot index number with number",
            5,
        ),
        ("[1,2]", "nan as $v | .[$v]", "null\n", "", 0),
        (
            "[1,2]",
            "nan as $v | .[$v] = 5",
            "",
            "Cannot set array element at NaN index",
            5,
        ),
        (
            "null",
            "nan as $v | .[$v] = 5",
            "",
            "Cannot set array element at NaN index",
            5,
        ),
        (
            "[1,2]",
            "nan as $v | .[$v]? = 5",
            "",
            "Cannot set array element at NaN index",
            5,
        ),
        (r#"{"a":1}"#, "nan as $v | .[$v]? = 5", "{\"a\":1}\n", "", 0),
        (
            "[1,2]",
            "nan as $v | .[$v] |= 5",
            "",
            "Cannot set array element at NaN index",
            5,
        ),
        (
            "[1,2]",
            "nan as $v | .[$v] += 5",
            "",
            "Cannot set array element at NaN index",
            5,
        ),
        (
            "[1,2]",
            "nan as $v | .[$v].x = 1",
            "",
            "Cannot set array element at NaN index",
            5,
        ),
        (
            "[[1],[2]]",
            "nan as $v | .[0][$v] = 1",
            "",
            "Cannot set array element at NaN index",
            5,
        ),
        (
            "[1,2]",
            "nan as $v | (.[0], .[$v]) |= 9",
            "",
            "Cannot set array element at NaN index",
            5,
        ),
        ("[1,2]", r#"nan as $v | .[$v] |= error("x")"#, "", "x", 5),
        (
            "[1,2]",
            "nan as $v | pick(.[$v])",
            "",
            "Cannot set array element at NaN index",
            5,
        ),
        (
            "[1,2]",
            r#"nan as $v | try (.[$v] = 5) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            "[[1],[2]]",
            "nan as $v | .[$v].x |= 1",
            "",
            "Cannot set array element at NaN index",
            5,
        ),
        (
            "[[1],[2]]",
            "nan as $v | .[$v][0] |= 1",
            "",
            "Cannot set array element at NaN index",
            5,
        ),
        (
            "[[1],[2]]",
            "nan as $v | (.[$v].x, .[0]) |= 1",
            "",
            "Cannot set array element at NaN index",
            5,
        ),
        (
            r#"{"c":null}"#,
            "nan as $v | del(.c[$v])",
            "{\"c\":null}\n",
            "",
            0,
        ),
        ("null", "nan as $v | del(.a[$v])", "null\n", "", 0),
        (
            r#"{"c":null}"#,
            "nan as $v | del(.c[$v], .c)",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"c":null}"#,
            "nan as $v | del(.a.b[$v], .c)",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"c":null}"#,
            "nan as $v | .c[$v] |= empty",
            "{\"c\":null}\n",
            "",
            0,
        ),
        ("null", "nan as $v | .c[$v] |= empty", "null\n", "", 0),
        (
            r#"{"c":null}"#,
            "nan as $v | (.c[$v], .c) |= empty",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"c":null}"#,
            "nan as $v | (.c[$v], .a) |= empty",
            "{\"c\":null}\n",
            "",
            0,
        ),
        ("[1,2]", "path(.[nan])", "[null]\n", "", 0),
        ("[1,2]", "path(nan as $v | .[$v])", "[null]\n", "", 0),
        (
            "[1,2]",
            ".[nan] = 5",
            "",
            "Cannot set array element at NaN index",
            5,
        ),
        (r#"{"c":null}"#, "del(.c[nan])", "{\"c\":null}\n", "", 0),
        (
            r#"{"c":null}"#,
            "nan as $v | .c[$v].x |= empty",
            "{\"c\":null}\n",
            "",
            0,
        ),
        (
            r#"{"c":null}"#,
            "nan as $v | .c[$v][0] |= empty",
            "{\"c\":null}\n",
            "",
            0,
        ),
        (
            r#"{"c":null}"#,
            "nan as $v | (.c[$v].x, .d) |= empty",
            "{\"c\":null}\n",
            "",
            0,
        ),
        ("null", "nan as $v | .c[$v].x |= empty", "null\n", "", 0),
        (
            r#"{"c":null}"#,
            "nan as $v | .c[$v].x |= 1",
            "",
            "jq: error (at <stdin>:0): Cannot set array element at NaN index",
            5,
        ),
        (
            r#"{"c":null}"#,
            "nan as $v | del(.c[$v][0], .d)",
            "{\"c\":null}\n",
            "",
            0,
        ),
        (
            r#"{"k":NaN,"a":[1,2]}"#,
            ".k as $v | .a | setpath([$v]; 9)",
            "",
            "jq: error (at <stdin>:0): Cannot set array element at NaN index",
            5,
        ),
        (
            r#"{"k":NaN,"a":[1,2]}"#,
            ".k as $v | .a | path(.[$v])",
            "[null]\n",
            "",
            0,
        ),
        (
            r#"{"k":NaN,"a":[1,2]}"#,
            ".k as $v | .a | .[$v] = 5",
            "",
            "jq: error (at <stdin>:0): Cannot set array element at NaN index",
            5,
        ),
    ];
    for (input, filter, stdout, error, code) in rows {
        let (out, err, status) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(&out, stdout, "stdout of {filter} on {input}");
        assert_eq!(status, *code, "exit code of {filter} on {input}");
        if error.is_empty() {
            assert_eq!(err, "", "stderr of {filter} on {input}");
        } else {
            assert!(
                err.lines().next().unwrap_or("").contains(error),
                "stderr of {filter} on {input}: {err}"
            );
        }
    }
    Ok(())
}

/// `del` and `|= empty` through a NaN index on an array parent never return in jq 1.7.1, so there is no answer to match.
/// A NaN index names no element, so they delete nothing and the document stays as it is, as the literal `del(.[nan])`
/// already did on the path-mode interpreter. Where jq does answer (a `null` or absent parent) the first table pins it.
#[test]
fn test_nan_variable_key_deletes_nothing_where_jq_never_returns_5245() -> Result<()> {
    let rows: &[(&str, &str, &str)] = &[
        ("[1,2]", "nan as $v | del(.[$v])", "[1,2]\n"),
        ("[1,2]", "nan as $v | del(.[$v]?)", "[1,2]\n"),
        ("[1,2]", "nan as $v | del(.[0], .[$v])", "[2]\n"),
        ("[1,2]", "nan as $v | del(.[$v].x)", "[1,2]\n"),
        ("[[1],[2]]", "nan as $v | del(.[$v][0])", "[[1],[2]]\n"),
        ("[[1],[2]]", "nan as $v | del(.[$v].x, .[0])", "[[2]]\n"),
        (
            "[[1],[2]]",
            "nan as $v | del(.[$v].x, .[$v].y)",
            "[[1],[2]]\n",
        ),
        ("[[1],[2]]", "nan as $v | del(.[$v][$v])", "[[1],[2]]\n"),
        ("[1,2]", "nan as $v | .[$v] |= empty", "[1,2]\n"),
        (
            r#"{"c":[1,2]}"#,
            "nan as $v | .c[$v] |= empty",
            "{\"c\":[1,2]}\n",
        ),
        (
            r#"{"c":[1,2]}"#,
            "nan as $v | del(.c[$v])",
            "{\"c\":[1,2]}\n",
        ),
    ];
    for (input, filter, stdout) in rows {
        let (out, err, status) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(&out, stdout, "stdout of {filter} on {input}");
        assert_eq!(status, 0, "exit code of {filter} on {input}: {err}");
    }
    Ok(())
}
