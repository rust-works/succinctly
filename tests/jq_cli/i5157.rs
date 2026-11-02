//! #5157: a `foreach` UPDATE that is a `//` over a bind of an outer `$x`, over a `null` register.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// jq's `a // b` does not restore its path register when `a` emits a truthy output, so a bind that stepped a `null`
// `$x` against a `null` register has moved the register for every later step; a falsy output backtracks to the entry
// register and runs `b` from there. The step whose state is not the register (the `5` the first step stored) met the
// bind with no register to step from and refused where jq's `null` `$x` is the register by kind.

/// #5157: the issue's rows (`// 5`, `// false`, `(..// 5) // 6`, `|= 9`, `del`) and a bind that ends on a literal or an
/// INIT that is not the register, each over the `null` document. Every row captured from `/usr/bin/jq` 1.7.1, both
/// routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5157_foreach_update_alternative_over_a_bind_of_a_null_x() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; (($x as {a:$a} | .)) // 5; $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; (($x as {a:$a} | .)) // false; $i))",
            "[\"a\"]\n[]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ((($x as {a:$a} | .)) // 5) // 6; $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // 5; $i)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r"null",
            r"del(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // 5; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | 5) // 6; $i))",
            "[\"a\",\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (5; ($x as {a:$a} | .) // 6; $i))",
            "[\"a\",\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as [$a] | .) // 5; $i))",
            "[\"a\"]\n[0]\n",
            "",
            0,
        ),
        // The right operand is any constant: an array, an object, a comma of them.
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // [1]; $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // {}; $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // (1,2); $i))",
            "[\"a\"]\n[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        // ...and the wrappers a chain is read through.
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; try (($x as {a:$a} | .) // 5); $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; limit(1; ($x as {a:$a} | .) // 5); $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; 5 | (($x as {a:$a} | .) // 6); $i))",
            "[\"a\",\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
    ])
}

/// #5157: the rows that must not move. A register that is not `null` is a pointer jq compares, so a bind of `$x`
/// refuses on the first step, and an EXTRACT that is the bound `.` after a `5` is a computed value, which jq refuses.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5157_rows_that_still_refuse() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // 5; $i))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":null}",
            5,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // 5; .))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
    ])
}

/// #5157: the shapes the fix leaves as they were. A two-argument `foreach` emits its UPDATE's output, which jq
/// path-checks (`5` here); a `reduce` is judged on its final accumulator, and a `?//` chain behind the `//` there names
/// `null` in jq, where reading the `//` through named the stored `5`.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5157_two_argument_foreach_and_reduce_keep_their_readings() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | $a) // 5))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | 5) // 6))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.a) as $i (false; (try ($x as {a:$a} | .)) // 5))",
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
