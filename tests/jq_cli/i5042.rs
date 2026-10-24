//! #5042: rows the path-mode interpreter (#5223) answers as jq 1.7.1 does.
//!
//! The interpreter reads storage identity, which the `unshared-containers` holdout does not have, so these pin the
//! shipped shape only.
#![cfg(not(feature = "unshared-containers"))]

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #5223 Phase 3: this issue's repro rows, with `path`, `del` and `|=`
/// wherever the row is a `path(...)`, as jq 1.7.1 answers them (stdout, the
/// first error message, the exit code). The path-mode interpreter carries
/// jq's register, so they match by construction, on both routes.
#[test]
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5042() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":{"c":3}},"b":{"c":4}}"#,
            r"path(. as $r | .a as $x | null | foreach ($x as {b:$q} | 1) as $y ($r | .a | tostring; .))",
            "",
            "Invalid path expression with result \"{\\\"b\\\":{\\\"c\\\":3}}\"",
            5,
        ),
        (
            r#"{"a":{"b":{"c":3}},"b":{"c":4}}"#,
            r"del(. as $r | .a as $x | null | foreach ($x as {b:$q} | 1) as $y ($r | .a | tostring; .))",
            "",
            "Invalid path expression with result \"{\\\"b\\\":{\\\"c\\\":3}}\"",
            5,
        ),
        (
            r#"{"a":{"b":{"c":3}},"b":{"c":4}}"#,
            r"(. as $r | .a as $x | null | foreach ($x as {b:$q} | 1) as $y ($r | .a | tostring; .)) |= 9",
            "",
            "Invalid path expression with result \"{\\\"b\\\":{\\\"c\\\":3}}\"",
            5,
        ),
        (
            r#"{"a":true,"b":false}"#,
            r"(.a as $x | . as $z | foreach (foreach (1,2) as $k (.; 1, ($x as {c:$q} | 1); .)) as $z (.a; .; .)) = 9",
            "",
            "Cannot index boolean with string \"c\"",
            5,
        ),
        (
            r#"{"a":{"b":{"c":3}},"b":{"c":4}}"#,
            r"path(first(. as $x | null | foreach (foreach (1,2) as $k (.; .; $x as {b:{c:$q}} | .)) as $z (null; .; .)))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":{"b":{"c":3}},"b":{"c":4}}"#,
            r"del(first(. as $x | null | foreach (foreach (1,2) as $k (.; .; $x as {b:{c:$q}} | .)) as $z (null; .; .)))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":{"b":{"c":3}},"b":{"c":4}}"#,
            r"(first(. as $x | null | foreach (foreach (1,2) as $k (.; .; $x as {b:{c:$q}} | .)) as $z (null; .; .))) |= 9",
            "",
            "Invalid path expression with result null",
            5,
        ),
    ])
}
