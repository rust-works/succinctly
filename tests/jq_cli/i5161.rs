//! #5161: a `//` over a bind of an outer `$x`, as the UPDATE of a two-argument `foreach` or of a `reduce`.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// jq's `a // b` hands the truthy outputs of `a` on without a path check, so the `.` a bind in `a` ends on is checked
// where the fold emits it, and what the error names is the output `//` kept (`5`), not the falsy `.` it replaced. The
// resolver checked the bind's own tail, refused `$x`'s destructure as the resolver's guess, and a `try` around the `//`
// swallowed that refusal: the UPDATE emitted nothing and `path`/`del`/`|=` answered where jq raises.

/// #5161: the issue's rows (`try`, `?`, the bare `//`), each over a two-argument `foreach` whose state is not the
/// register. Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5161_two_argument_foreach_update_alternative_over_a_bind() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try (($x as {a:$a} | 5) // 6)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | foreach (1,2) as $i (false; try (($x as {a:$a} | 5) // 6)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | foreach (1,2) as $i (false; (($x as {a:$a} | 5) // 6)?)) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.a) as $i (false; (($x as {a:$a} | .)) // 5))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        // The right operand is any inert operand: the loop's `$i`, arithmetic of it, a comma of them, and a `//` of `//`s.
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try (($x as {a:$a} | 5) // $i)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try (($x as {a:$a} | 5) // (6, $i))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try (($x as {a:$a} | 5) // ($i + 1))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try ((($x as {a:$a} | 5) // 6) // 7)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        // The bind's `.` is falsy over a `false` state, so `//` replaces it: the error names the `6`; over a truthy `7`
        // it keeps it.
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (false; (($x as {a:$a} | .) // 6)))",
            "",
            "Invalid path expression with result 6",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (7; (($x as {a:$a} | .) // 6)))",
            "",
            "Invalid path expression with result 7",
            5,
        ),
    ])
}

/// #5161: the same `//` as a `reduce` UPDATE, whose final accumulator jq path-checks, and the `?//` chain behind it that
/// stays as it was (jq names `null`, the stored `5` is not what its retry leaves).
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5161_reduce_update_alternative_over_a_bind() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r"null",
            r"del(. as $x | reduce (1,2) as $i (null; try (($x as [$a] | $a) // 5)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"path(. as $x | reduce (1,2) as $i (null; (($x as {a:$a} | 5) // 6)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"path(. as $x | reduce (1,2) as $i (null; try (($x as [$a] | $a) // $i)))",
            "",
            "Invalid path expression with result 2",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce (.a,.) as $i (null; (($x as {a:$a} | .a) // 5)?))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce (.a,.) as $i (null; (($x as {a:$a} ?// $a | .a) // 5)?))",
            "",
            "Invalid path expression with result null",
            5,
        ),
    ])
}
