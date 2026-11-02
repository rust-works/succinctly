//! #5167: a `//` over a bind of an outer `$x` in a two-argument `foreach` or a `reduce` UPDATE, with a right operand that is
//! not a constant and with a `?//` chain on the left.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// #5161 read the `//` only for a constant right operand and a single pattern. An operand that is the register's node (an
// outer `$x`, `.`, the loop's `$i` over a source that navigates, which is that element) passes the emission's check where a
// constant raises, and the resolver resolves it as a branch with its own provenance, so it is read like a constant. A
// `?//` chain behind the `//` is read for a two-argument `foreach` (a `reduce` keeps the single pattern: its final
// accumulator is judged after jq retried a failed alternative on the state it left). Left unread, the bind refusal was the
// resolver's guess and a `try`/`?` around the `//` swallowed it: the UPDATE emitted nothing and `path`/`del`/`|=` answered
// where jq raises.

/// #5167: the issue's rows (an outer `$x`, a navigation and the loop's `$i` over `(.a,.)` as the right operand, under
/// `try` and `?`) and the operands the grid showed move nothing that matched. Every row captured from `/usr/bin/jq` 1.7.1,
/// both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5167_two_argument_foreach_alternative_with_a_non_constant_operand() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try (($x as {a:$a} | 5) // $x)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try (($x as {a:$a} | 5) // .a)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; try ((($x as {a:$a} | 5) // $i))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; try ((($x as {a:$a} | 5) // .))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try (($x as {a:$a} | 5) // (.a, 6))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; (($x as {a:$a} | 5) // $i)?)) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
    ])
}

/// #5167: a `?//` chain behind the `//` in a two-argument `foreach`, where jq retries the failed alternative on the state it
/// left.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5167_two_argument_foreach_alternative_over_a_destructuring_alternation() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (1,2) as $i (null; (($x as {a:$a} ?// $a | .a) // 5)?))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (1,2) as $i (5; try (($x as {a:$a} ?// $a | .a) // 5)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
    ])
}

/// #5167: a `reduce` reads the `$x` operand, and keeps its `?//` chain as it was: jq names the `null` its failed
/// alternative left, and reading the chain named the stored `5`.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5167_reduce_alternative_with_a_marker_operand_and_an_alternation() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r"null",
            r"path(. as $x | reduce (1,2) as $i (null; try (($x as [$a] | $a) // $x)))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce (.a,.) as $i (null; (($x as {a:$a} | .a) // .a?)?))",
            "",
            "Invalid path expression with result null",
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

/// #5167: the same operands behind a three-argument `foreach`'s `//` (#5157), which refused the loop's `$i` and a
/// comma of a navigation and a constant after the first step.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5167_three_argument_foreach_alternative_with_a_non_constant_operand() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // ($i + 1); $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // (.a, 6); $i))",
            "[\"a\",\"a\"]\n[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
    ])
}
