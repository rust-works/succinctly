//! #5194: rows the path-mode interpreter (#5223) answers as jq 1.7.1 does.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #5223 Phase 3: this issue's repro rows, with `path`, `del` and `|=`
/// wherever the row is a `path(...)`, as jq 1.7.1 answers them (stdout, the
/// first error message, the exit code). The path-mode interpreter carries
/// jq's register, so they match by construction, on both routes.
#[test]
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5194() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach (1,2) as $i (.; ($x as {a:$a} ?// $a | .a) // false; .))",
            "[\"a\"]\n",
            "Invalid path expression near attempt to access element \"a\" of 1",
            5,
        ),
        (
            r#"{"a":1}"#,
            r"del(. as $x | foreach (1,2) as $i (.; ($x as {a:$a} ?// $a | .a) // false; .))",
            "",
            "Invalid path expression near attempt to access element \"a\" of 1",
            5,
        ),
        (
            r#"{"a":1}"#,
            r"(. as $x | foreach (1,2) as $i (.; ($x as {a:$a} ?// $a | .a) // false; .)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"a\" of 1",
            5,
        ),
    ])
}
