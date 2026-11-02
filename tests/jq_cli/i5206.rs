//! #5206: the pipe spelling `$x | . as P | .` of a caught destructure of a foreign `$x`, in a sole nested `foreach` UPDATE.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// #5189 (#5113) emptied a sole nested `foreach` whose source is a caught `$x as P | .` destructure of an object or array
// `$x` that differs from the step's register, and read the pipe spelling no further than #5017 had. The destructure is the
// same one: its first step fails `path_intact`, the `try` leaves the source empty, and the UPDATE emits nothing for the step.

/// #5206: the pipe spelling, under `path`, `?`, `|=` and `del`, on `null`, `true` and container elements, with an array pattern, a `.` stage inside the pipe and a `null` EXTRACT; the root source, where `$x` is the register, answers as before.
/// Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5206_piped_caught_destructure_of_a_foreign_marker_leaves_the_source_empty() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach (try ($x | . as {a:$a} | .)) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach (try ($x | . as {a:$a} | .)) as $y (.; .; .); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#". as $x | try ((foreach (.a) as $i (.; foreach (try ($x | . as {a:$a} | .)) as $y (.; .; .); $i)) |= 9) catch "C""#,
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#". as $x | try del(foreach (.a) as $i (.; foreach (try ($x | . as {a:$a} | .)) as $y (.; .; .); $i)) catch "C""#,
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r#". as $x | try ((foreach (.a) as $i (.; foreach (try ($x | . as {a:$a} | .)) as $y (.; .; .); $i)) |= 9) catch "C""#,
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach (try ($x | . as {a:$a} | .)) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach (($x | . as [$a] | .)?) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach (try ($x | . | . as {a:$a} | .)) as $y (.; .; $y); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a,.) as $i (null; foreach (($x | . as {a:$a} | .)?) as $y (.; .; .); null))]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.) as $i (.; foreach (try ($x | . as {a:$a} | .)) as $y (.; .; .); $i))]",
            r"",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
    ])
}
