//! #5177: a nested `foreach` source that destructures an outer `$x` on a step whose state is the register.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// Where the UPDATE's state is the node the source moved the register onto and `$x` is that node too, the pattern's first step
// passes and the pattern then raises (an array pattern on an object, the second entry of an object pattern). jq's raise is
// catchable. The fold source's refusal was the resolver's guess, which nothing catches, so a `try` around the nested `foreach`
// never ran its handler and the message was `with result`.

/// #5177: the issue's rows (INIT `.`, so the state is the root, the element and `$x`) over `path`, `del`, `|=` and `[path(..)?]`,
/// and the `try del(..)` row that #5173 turned loud. Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5177_nested_foreach_destructure_on_a_state_that_is_the_register() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":[null]}"#,
            r"path(. as $x | foreach (.) as $i (.; try (foreach ($x as [$a] | .) as $y (.; .; .)) catch $i; $i))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"path(. as $x | foreach (.) as $i (.; try (foreach ($x as {a:$a,b:$b} | .) as $y (.; .; .)) catch $i; $i))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"path(. as $x | foreach (.) as $i (.; foreach ($x as [$a] | .) as $y (.; .; 5); $i))",
            "",
            "Cannot index object with number",
            5,
        ),
        (
            r#"{"a":[null]}"#,
            r"del(. as $x | foreach (.) as $i (.; try (foreach ($x as [$a] | .) as $y (.; .; .)) catch $i; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"(. as $x | foreach (.) as $i (.; try (foreach ($x as [$a] | .) as $y (.; .; .)) catch $i; $i)) |= 1",
            "1\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"[path(. as $x | foreach (.) as $i (.; try (foreach ($x as [$a] | .) as $y (.; .; .)) catch $i; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r#"try del(. as $x | foreach (.,.,.a) as $i (null; try (foreach ($x as [$a] | .) as $y (.; $y; .a)) catch $x; $x)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r#"try path(. as $x | foreach (.,.) as $i (null; try (foreach ($x as [$a] | .) as $y (.; .; .)) catch $x; $x)) catch "C""#,
            "[]\n[]\n",
            "",
            0,
        ),
        (
            r#"{"x":{"a":[1]},"y":{"a":[1]}}"#,
            r"path(.x as $x | foreach (.y) as $i (.y; try (foreach ($x as [$a] | .) as $z (.; .; .)) catch $i; $i))",
            "",
            "Invalid path expression near attempt to access element",
            5,
        ),
        (
            r#"{"x":{"a":[1]},"y":{"a":[1]}}"#,
            r"path(.y as $x | foreach (.y) as $i (.y; try (foreach ($x as [$a] | .) as $z (.; .; .)) catch $i; $i))",
            "",
            "Invalid path expression near attempt to access element",
            5,
        ),
        (
            r#"{"x":{"a":[1]},"y":{"a":[1]}}"#,
            r"path(. as $x | foreach (.y) as $i (.y; try (foreach ($x as [$a] | .) as $z (.; .; .)) catch $i; $i))",
            "",
            "Invalid path expression near attempt to access element",
            5,
        ),
        (
            r#"{"x":{"a":[1]},"y":{"a":[1]}}"#,
            r"path(.x as $x | foreach (.y) as $i (.x; foreach ($x as [$a] | .) as $z (.; .; 5); $i))",
            "",
            "Invalid path expression near attempt to access element",
            5,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.) as $i (.; try (foreach ($x as [$a] | .) as $z (.; .; .)) catch $i; $i))",
            "[0]\n",
            "",
            0,
        ),
        (
            r"[null]",
            r"path(. as $x | foreach (.[0]) as $i (.[0]; try (foreach ($x as [$a] | .) as $z (.; .; .)) catch $i; $i))",
            "",
            "Invalid path expression near attempt to access element 0",
            5,
        ),
    ])
}
