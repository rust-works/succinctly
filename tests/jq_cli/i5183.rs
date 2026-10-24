//! #5183: a nested `foreach` over a bind of the `$x` that is the register, with an UPDATE or EXTRACT other than `.`, in the UPDATE of a `foreach`.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// jq's pattern step moves the register onto the bound child, and a nested `foreach` leaves it there whatever its UPDATE or
// EXTRACT emits (a literal, the loop variable): the outer EXTRACT meets the `null` the step reached, which is the register
// by kind. #4957 restated the register for a nested `foreach` whose UPDATE and EXTRACT are `.` only
// (`bare_bind_foreach_update_key`); any other emission was refused (`Invalid path expression with result null`) where jq
// answers, and a `?` over it dropped the path.

/// #5183: the issue's rows (the loop variable, a literal and `null` as the nested EXTRACT or UPDATE, behind a `try`), every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5183_nested_foreach_bind_with_an_inert_update_or_extract() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (1) as $i (null; foreach (try ($x as {a:$a} | .)) as $y (.; .; $y); null))]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (1) as $i (null; foreach ($x as {a:$a} | .) as $y (.; .; 5); null))]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (1) as $i (null; foreach ($x as {a:$a} | .) as $y (.; .; null); null))]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (1) as $i (null; foreach ($x as {a:$a} | .) as $y (.; 5; .); null))]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (1) as $i (null; foreach ($x as {a:$a} | .) as $y (.; $y; $y); null))]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a?, 1) as $i (null; try foreach (try ($x as {a:$a} | .)) as $y (.; .; $y); null))]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
    ])
}

/// #5183: `|=`, `del`, a comma beside the nested `foreach` and a two-element source.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5183_nested_foreach_bind_in_write_and_comma_contexts() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r#". as $x | try ((foreach (1) as $i (null; foreach ($x as {a:$a} | .) as $y (.; .; 5); null)) |= 9) catch "C""#,
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#". as $x | try del(foreach (1) as $i (null; foreach ($x as {a:$a} | .) as $y (.; .; 5); null)) catch "C""#,
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r#". as $x | try ((foreach (.b) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; 5; .); null)) |= 9) catch "C""#,
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (1) as $i (null; (foreach (try ($x as {a:$a} | .)) as $y (.; .; $y), 7); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (1) as $i (null; (7, foreach (try ($x as {a:$a} | .)) as $y (.; .; $y)); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (1,2) as $i (.; foreach ($x as {a:$a} | .) as $y (.; 5; .); null))]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r". as $x | [path(foreach (1) as $i (null; foreach ($x as [$a] | .) as $y (.; .; 5); null))]",
            r"",
            r"Cannot index object with number",
            5,
        ),
    ])
}

/// #5183: the nested `.`/`.` form, a source that is not a bind, a `reduce` (jq restores the register when it backtracks out of it) and a `$i` outer EXTRACT are unchanged.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5183_nested_foreach_bind_shapes_that_keep_their_answer() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (1) as $i (null; foreach ($x as {a:$a} | .) as $y (.; .; .); null))]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (1) as $i (null; foreach (1,2) as $y (.; .; $y); null))]",
            r"",
            r"Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (1) as $i (null; reduce ($x as {a:$a} | .) as $y (.; .); null))]",
            r"",
            r"Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (null; foreach ($x as {a:$a} | .) as $y (.; .; 5); $i))]",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
    ])
}

/// #5183: the loop variable shadowing the `$x` it destructures, an outer EXTRACT that navigates, a `del` over two steps, and the
/// pipe tails and heads on which the widening stays off (`FE | 7`, `FE | null`, `.a | FE`) answer as jq does, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5183_nested_foreach_bind_boundaries() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (1) as $i (null; foreach ($x as {a:$a} | .) as $x (.; $x; $x); null))]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (1) as $i (null; foreach ($x as {a:$a} | .) as $y (.; $y; $y); .a?))]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#". as $x | try del(foreach (1,2) as $i (.; foreach ($x as {a:$a} | .) as $y (.; $y; 5); $i|.)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; $y) | 7; $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; 5) | null; $i))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5199: a bare destructure of a `$x` that is not the step's register, with no `try` around it, piped into a stage that computes a
/// value is read as the bind passing. jq's answer is its raise (`near attempt to access element "a"`); this pins what is answered
/// until #5199 checks the marker on that route. The caught spelling is jq's `[]`, as `|=` and `del` leave the document.
#[cfg(not(feature = "unshared-containers"))]
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5183_value_tail_after_the_bind_is_read_as_passing_until_5199() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | 7; $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .) | 7; $i))]",
            "",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
    ])
}
