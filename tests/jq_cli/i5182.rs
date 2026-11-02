//! #5182: a nested `foreach` over an uncaught destructure of a foreign `$x`, in the UPDATE of a `foreach` whose source is not the root.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// #5176 read the caught form: the pattern's step fails `path_intact`, the `try` in the source empties it and the UPDATE emits
// nothing. With no `try` the same step raises `Invalid path expression near attempt to access element K of $x`, the error leaves
// the whole loop, and a `?`, `try` or `catch` outside it decides what is left; the source was driven as if `$x` were the
// register, the pattern walked off the root and a `$i` EXTRACT named `["b","a"]`, which `del` and `|=` wrote.

/// #5182: the issue's rows (`path`, `?`, `del`, `|=` and the comma UPDATEs) on `null`, `true`, string and container elements, every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5182_uncaught_foreign_foreach_over_a_non_root_source_raises() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); $i))]",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a) as $i (null; foreach ($x as {a:$a} | .) as $y (.; .; .); $i|.))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#". as $x | try del(foreach (.a) as $i (null; foreach ($x as {a:$a} | .) as $y (.; .; .); $i)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#". as $x | try ((foreach (.a) as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); $i)) |= 9) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); $i))]",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":true}"#,
            5,
        ),
        (
            r#"{"a":"s"}"#,
            r". as $x | [path(foreach (.a) as $i (0; foreach ($x as {a:$a} | .) as $y (.; .; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r#". as $x | try del(foreach (.[]?) as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); $x)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); $i))]",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"a":null}}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; (foreach ($x as {a:$a} | .) as $y (.; .; .), 7); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; (7, foreach ($x as {a:$a} | .) as $y (.; .; .)); $i))?]",
            "[[\"b\"]]\n",
            "",
            0,
        ),
    ])
}

/// #5182: the step the error names for an array pattern, two entries, a nested pattern, a `$a` body and `[$a]` under `?`.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5182_uncaught_foreign_foreach_patterns_name_the_step_jq_names() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; foreach ($x as [$a,$b] | .) as $y (.; .; .); $i))]",
            r"",
            r#"Invalid path expression near attempt to access element 1 of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; foreach ($x as {a:$a,b:$b} | .) as $y (.; .; .); $i))]",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; foreach ($x as {a:{b:$q}} | .) as $y (.; .; .); $i))]",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; foreach ($x as {a:$a} | $a) as $y (.; .; .); $i))]",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; foreach ($x as [$a] | .) as $y (.; .; $y); $i))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5182: the sources on which the root `$x` is the register on a step (`.`, `(.a, .)`), a `$x` that is the element itself, and the caught form are not read as foreign.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5182_uncaught_foreign_foreach_where_the_marker_is_the_register_is_unchanged() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.) as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); $i))]",
            r"",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a, .) as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); $i))]",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":{"a":1}}"#,
            r".a as $x | [path(foreach (.a) as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); $i))]",
            r"",
            r#"Invalid path expression with result {"a":1}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// A `?` over a step the nested `foreach` still refuses (its bind body is `$a`, not `.`: `bare_bind_source_key` reads `.` only)
/// used to be answered by step 1's wrong emission, which the raise removed. jq answers `[["a"]]`; the refusal is dropped by the
/// `?` and the path is `[]`, pinned here (a divergence, not jq's answer) until a bind body other than `.` is read.
///
/// #5223: the path-mode interpreter carries jq's register, so the rows the shipped resolver
/// refused here now hold jq 1.7.1's answers.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5182_step_two_refusal_a_question_mark_drops_until_a_non_dot_bind_body_is_read() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[(
        r#"{"a":null}"#,
        r". as $x | [path(foreach (.a?, 1) as $i (null; try foreach ($x as {a:$a} | $a) as $y (.; .; .); .))?]",
        "[[\"a\"]]\n",
        "",
        0,
    )])
}
