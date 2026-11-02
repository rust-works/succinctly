//! #5162: a three-argument `foreach` UPDATE that is `(B) // const | .`, a `//` over a bind of an outer `$x` piped
//! into `.`.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// A `| .` is no code at all in jq, so `(B) // 5 | .` is `(B) // 5`, the shape #5157 reads at the register its bind
// moved to. The piped spelling was not marked, and met the step after the first with the stored `5` and no register
// to step from.

/// #5162: the issue's row, its `(..) | .` spelling, other constants, the write contexts, and a bind that ends on a
/// literal or an INIT that is not the register, each over the `null` document, plus the rows that refuse in jq (the
/// EXTRACT judging the stored `5`, a non-`null` register), a doubled `| .`, the other constant right operands, and the
/// left operands the rewrite leaves piped (a comma tail, a `def`: the bare spelling answers them wrongly, #5170). Every row captured from `/usr/bin/jq`
/// 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5162_foreach_update_alternative_over_a_bind_piped_into_identity() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // 5 | .; $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; (($x as {a:$a} | .) // 5) | .; $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // false | .; $i))",
            "[\"a\"]\n[]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // [1] | .; $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // 5 | .; $i)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r"null",
            r"del(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // 5 | .; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | 5) // 6 | .; $i))",
            "[\"a\",\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (5; ($x as {a:$a} | .) // 6 | .; $i))",
            "[\"a\",\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (null; (($x as {a:$a} | .) // 5) | .; $x))",
            "[]\n",
            "Invalid path expression with result {\"a\":{\"b\":1}}",
            5,
        ),
        (
            r"[1]",
            r"path(. as $x | foreach (1,2) as $i (false; (($x as [$a] | .) // 5) | .; $x))",
            "[]\n",
            "Invalid path expression with result [1]",
            5,
        ),
        (
            r"true",
            r"path(. as $x | foreach (.,.a) as $i (false; (($x as {a:$a} ?// $a | .) // 5) | .; $x))",
            "[]\n",
            "Cannot index boolean with string \"a\"",
            5,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // 5 | .; .))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // 5 | .; $i))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":null}",
            5,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // 5 | .))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // 5 | . | .; $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | (.,.)) // 5 | .; $i))",
            "[\"a\"]\n[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; (def f: .; $x as {a:$a} | f) // 5 | .; $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // (1,2) | .; $i))",
            "[\"a\"]\n[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // {} | .; $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // {a:1} | .; $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | .) // 5 | .; $x))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
    ])
}
