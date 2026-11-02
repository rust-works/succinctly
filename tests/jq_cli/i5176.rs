//! #5176: a caught destructure of a foreign `$x` in a nested `foreach` that is the UPDATE of a `foreach` whose source is not the root.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// The nested `foreach` runs in the UPDATE of a step whose element the outer source moved jq's register onto (`.a`), and a
// `$x` bound at the root is not that node: its pattern step fails `path_intact` before it reads a key, the `try` around
// it empties the nested `foreach`'s source, and the UPDATE emits nothing for the step. #4800's mark named the element for
// a destructure that lands on a `null` or a boolean of the state's kind; a `$x` of another kind than the element
// (the root object against a `null`, a boolean, a number or a different object) was read against the state instead, so the
// pattern walked off the root, the nested `foreach` emitted, and a `$i` EXTRACT named `["a","a"]`, which `|=` wrote.

/// #5176: the issue's rows (`path`, `|=`, `del` on `null`, `true` and `1` elements, and the `(FE, 7)`/`(7, FE)` commas), every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5176_caught_foreign_foreach_over_a_non_root_source_emits_nothing() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | (foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r". as $x | (foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i)) |= 9",
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r". as $x | (foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i)) |= 9",
            "{\"a\":1}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | del(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r". as $x | [path(foreach (.a) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), 7); $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r". as $x | [path(foreach (.a) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), 7); $i))]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r". as $x | [path(foreach (.a) as $i (.; (7, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); $i))]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
    ])
}

/// #5176: other sources (`.b`, `(.a, 1)`, `.[]?`, `.a | .`), INITs (`null`, `0`) and EXTRACTs (`$i | .`, `.`, `$x`) of the same shape.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5176_caught_foreign_foreach_over_other_sources_and_inits() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a, 1) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))]",
            r"",
            r"Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":false}"#,
            r". as $x | [path(foreach (.[]?) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i|.))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a | .) as $i (null; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":"s"}"#,
            r". as $x | (foreach (.a) as $i (0; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .)) |= 9",
            "{\"a\":\"s\"}\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r". as $x | del(foreach (.a) as $i (null; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), 7); $x))",
            r"",
            r#"Invalid path expression with result {"a":[null]}"#,
            5,
        ),
        (
            r#"{"a":true}"#,
            r". as $x | [path(foreach (.b) as $i (0; (7, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); $i))]",
            "[[\"b\"]]\n",
            "",
            0,
        ),
    ])
}

/// #5176: an array pattern and a `?` around the destructure are read as the object pattern under `try` is.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5176_caught_foreign_foreach_array_pattern_and_optional_source() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach (try ($x as [$a] | .)) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r". as $x | (foreach (.a) as $i (.; foreach ((. as {a:$a} | .)?) as $y (.; .; .); $i)) |= 9",
            "{\"a\":1}\n",
            "",
            0,
        ),
    ])
}

/// #5176: the sources on which the root `$x` is the register on a step (`.`, `(.a, .)`, `(., .a)`), and a `$x` that is the element itself (a container, a string, a number, a container holding a `NaN`), are not read as foreign: they match jq as they did.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5176_caught_foreign_foreach_where_the_marker_is_the_register_is_unchanged() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))]",
            r"",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a, .) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))]",
            r"",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (., .a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))]",
            r"",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
        (
            r#"{"a":{"a":1}}"#,
            r".a as $x | [path(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))]",
            r"",
            r#"Invalid path expression with result {"a":1}"#,
            5,
        ),
        (
            r#"{"a":[nan]}"#,
            r".a as $x | [path(foreach (.a) as $i (.; foreach (try ($x as [$q] | .)) as $y (.; .; .); $i))]",
            "",
            "Invalid path expression with result [null]",
            5,
        ),
        (
            r#"{"a":[1,2]}"#,
            r".a as $x | (foreach (.a) as $i (.; foreach (try ($x as [$q] | .)) as $y (.; .; .); $i)) |= 9",
            "",
            "Invalid path expression with result [1,2]",
            5,
        ),
        (
            r#"{"a":1}"#,
            r".a as $x | (foreach (.a) as $i (.; foreach (try ($x as [$q] | .)) as $y (.; .; .); $i)) |= 9",
            "{\"a\":1}\n",
            "",
            0,
        ),
        (
            r#"{"a":"s"}"#,
            r".a as $x | [path(foreach (.a) as $i (.; foreach (try ($x as [$q] | .)) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5183: a `?` over a step the nested `foreach` refused (the `$y` EXTRACT on a literal element, where jq's register is the root
/// and `$x` is it) was answered by step 1's wrong emission, which #5176 removed. The step is read now, and jq's `[["a"]]` stands.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5176_step_two_a_question_mark_hid_is_read_5183() -> Result<()> {
    assert_path_rows_both_routes_3749(&[(
        r#"{"a":null}"#,
        r". as $x | [path(foreach (.a?, 1) as $i (null; try foreach (try ($x as {a:$a} | .)) as $y (.; .; $y); null))?]",
        "[[\"a\"]]\n",
        "",
        0,
    )])
}
