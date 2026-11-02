//! #5228: the explicit pipe spelling of a `//` alternate, and a `?` or `try` around the path of the plain one, answer as jq does.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #5223 Phase 3: this issue's repro rows, with `path`, `del` and `|=`
/// wherever the row is a `path(...)`, as jq 1.7.1 answers them (stdout, the
/// first error message, the exit code). The path-mode interpreter carries
/// jq's register, so they match by construction, on both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5228() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .) | .; $i))",
            "[\"a\"]\n",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .) | .; $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .) | .; $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .) | .; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .) | .; $i)) catch "C""#,
            "[\"a\"]\n\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // $x) | .; $i))",
            "[\"a\"]\n",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // $x) | .; $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // $x) | .; $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // $x) | .; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // $x) | .; $i)) catch "C""#,
            "[\"a\"]\n\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // {b:1}) | .; $i))",
            "[\"a\"]\n",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // {b:1}) | .; $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // {b:1}) | .; $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // {b:1}) | .; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // {b:1}) | .; $i)) catch "C""#,
            "[\"a\"]\n\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .; $i))",
            "[\"a\"]\n",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .; $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .; $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .; $i)) catch "C""#,
            "[\"a\"]\n\"C\"\n",
            "",
            0,
        ),
    ])
}
