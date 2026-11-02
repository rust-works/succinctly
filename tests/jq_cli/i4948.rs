//! #4948: an `if . then FE else B end` UPDATE of a `foreach` over a moving source, `FE` a `foreach` whose source
//! is a caught destructure of a foreign `$x`.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// jq runs the step's UPDATE from the element the source moved the register onto, with the state the steps before
// it left. `FE` raises at its source's pattern step (`$x` is not the register), so it emits nothing and the state
// the next step meets is `null`: `if .` is then false and the `else .` branch hands on a `null` that is no node of
// the register, which jq's own index error (`.a` of `null`) or end-of-path check reports. The branch not taken
// moves nothing, so the register is where the source put it; the binary read the whole `if` as one that may have
// moved it, took the `null` for a register it had lost, and raised the index error as a guess that no `?` or
// `try` catches (`[path(..)?]` exited 5 where jq answers `[]`).

/// #4948: `else .` is the branch taken on the `null` the emptied step left behind. Every row captured from
/// `/usr/bin/jq` 1.7.1 (stdout, first stderr line up to the container, exit code), both routes: the rows that print
/// `[]`, `"C"` or a written document were exits 5 before.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4948_else_dot_after_an_emptied_caught_foreign_foreach_step() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a)) |= 5",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a)) = 5",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i))?]",
            "[null]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i)) |= 5",
            "5\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i)) |= 5",
            "5\n",
            "",
            0,
        ),
    ])
}

/// #4948: the same step behind the other spellings of the `if` (no `else`, a `$y` EXTRACT, an array pattern) and
/// an `else` that computes a literal on a `null` document.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4948_other_if_shapes_around_the_caught_foreign_foreach() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end; $i))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; $y)) else . end; .a)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; $y)) else . end; $i)) |= 5",
            "5\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as [$a] | .)) as $y (.; .; .)) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as [$a] | .)) as $y (.; .; .)) else . end; $i))?]",
            "[null]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach .a as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else 7 end; $i))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"[path(. as $x | foreach .a as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else 7 end; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r"null",
            r"del(. as $x | foreach .a as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else 7 end; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r"null",
            r"(. as $x | foreach .a as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else 7 end; $i)) |= 5",
            "{\"a\":5}\n",
            "",
            0,
        ),
    ])
}

/// #4948: controls that matched jq before the change and must still: a state that is truthy runs `FE`, so the
/// `then` branch is read as it was; an `if` that is not around a `foreach`; the bare `foreach`.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4948_controls_keep_their_answers() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"[path(. as $x | foreach .a as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .[]? as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then empty else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then empty else . end; .a))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))",
            "",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
    ])
}
