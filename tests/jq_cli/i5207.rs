//! #5207: rows the path-mode interpreter (#5223) answers as jq 1.7.1 does.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #5223 Phase 3: this issue's repro rows, with `path`, `del` and `|=`
/// wherever the row is a `path(...)`, as jq 1.7.1 answers them (stdout, the
/// first error message, the exit code). The path-mode interpreter carries
/// jq's register, so they match by construction, on both routes.
#[test]
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5207() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r". as $x | (foreach ((.a,.)) as $i (null; try (foreach (try ($x as {a:$a} | 1)) as $y (.; .; .a)) catch 1; empty)) |= 9",
            "{\"a\":{\"a\":null}}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r". as $x | (foreach ((.a,.)) as $i (false; try (foreach (try ($x as {a:$a} | 1)) as $y (.; .; .a)) catch 1; empty)) |= 9",
            "{\"a\":{\"a\":null}}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | (foreach ((.a,.)) as $i (null; try (foreach (try ($x as {a:$a} | 1)) as $y (.; .; .a)) catch 1; empty)))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}
