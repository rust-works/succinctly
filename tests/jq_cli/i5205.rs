//! #5205: a `null` or boolean `$x` in the caught destructure that is the whole UPDATE of a `foreach` step. The marker is
//! `jv_identical` by kind to the register of a `null` element, so the pattern's step passes it there and jq's register moves
//! onto the bound child; on any other element it fails `path_intact` and the `try` leaves the source empty. The step that
//! passes is placed (a nested `foreach` over a bare bind whose INIT is `.` or a literal, as #4957 read for `.`), which is
//! what lets the steps that do not pass be read as empty.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #5205: the issue's row and its `?`, `path`, `del` and `|=` forms, a literal INIT (`0`, `false`, `true`), a one-element
/// and a three-element source, a boolean marker, a marker bound two levels down, and INIT `.`. Captured from
/// `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_foreach_update_caught_bind_of_a_null_or_boolean_marker_5205() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"[path(.a as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (null; .; .) | .); $i))]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(.a as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (null; .; .)); $i))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(.a as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (null; .; .)); $i))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(.a as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (null; .; .)); $i))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(.a as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (null; .; .)); $i)) |= 9",
            "{\"a\":{\"a\":9}}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(.a as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (0; .; .)); $i))]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(.a as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (false; .; .)); $i))]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(.a as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (true; .; .)); $i))]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(.a as $x | foreach (.a) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (null; .; .)); $i))]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(.a as $x | foreach (.a,.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (null; .; .)); $i))]",
            "[[\"a\",\"a\"],[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(.a as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (null; .; .)); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"(.a as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (null; .; .)); $i)) |= 9",
            "{\"a\":false}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":true}}"#,
            r"[path(.a.a as $x | foreach (.a.a,.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (null; .; .)); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(.a as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); $i))]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        // The pipe spelling `$x | . as P | .` of a `null` marker is not read (jq answers `[["a","a"]]`); it refuses as `main` does.
        (
            r#"{"a":null}"#,
            r".a as $x | [path(foreach (.a,.) as $i (.; (foreach (try ($x | . as {a:$a} | .)) as $y (null; .; .)); $i))]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
    ])
}

/// #5205: shapes the reading does not cover keep the refusal `main` raised: the pipe spelling of a `null` marker, and a
/// nested `foreach` whose INIT navigates
/// (`.a?`), where lifting the exclusion turned a raise into a write of `{"a":{"a":{"a":{"a":9}}}}` (8 of 12,000 rows
/// of the first draft's grid).
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_foreach_update_caught_bind_of_a_null_marker_unplaced_shapes_stay_refused_5205() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[(
        r#"{"a":null}"#,
        r"(.a as $x | foreach (.,.a,.a) as $i (.; (foreach (try ($x as {a:{a:$a}} | 1)) as $y (.a?; .; .))?; $i)) = 9",
        "",
        "Invalid path expression with result {\"a\":null}",
        5,
    )])
}

/// #5223 Phase 3: this issue's repro rows, with `path`, `del` and `|=`
/// wherever the row is a `path(...)`, as jq 1.7.1 answers them (stdout, the
/// first error message, the exit code). The path-mode interpreter carries
/// jq's register, so they match by construction, on both routes.
#[test]
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5205() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r".a as $x | [path(foreach ((.a,.)) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (null; .; .) | .); $i))]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r".a as $x | del(foreach ((.a,.)) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (null; .; .) | .); $i))",
            "{\"a\":null}\n",
            "",
            0,
        ),
    ])
}
