//! #5213: rows the path-mode interpreter (#5223) answers as jq 1.7.1 does.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #5223 Phase 3: this issue's repro rows, with `path`, `del` and `|=`
/// wherever the row is a `path(...)`, as jq 1.7.1 answers them (stdout, the
/// first error message, the exit code). The path-mode interpreter carries
/// jq's register, so they match by construction, on both routes.
#[test]
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5213() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null},"c":1}"#,
            r"[path(. as $x | foreach (.,.,.a) as $i (.; try (foreach ($x | .) as $y (0; .a?; $y)) catch ($i | .a?); .))?]",
            "[[\"a\"],[\"a\"],[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null},"c":1}"#,
            r"((. as $x | foreach (.,.,.a) as $i (.; try (foreach ($x | .) as $y (0; .a?; $y)) catch ($i | .a?); .))?) |= 9",
            "",
            "Cannot index number with string \"a\"",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.,.) as $i (.; try (foreach ($x | .) as $y (0; .a?; $y)) catch ($i | .a?); .))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.,.) as $i (.; try (foreach ($x | .) as $y (0; .a?; $y)) catch ($i | .a?); .))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.,.) as $i (.; try (foreach ($x | .) as $y (0; .a?; $y)) catch ($i | .a?); .)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
    ])
}
