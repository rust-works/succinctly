//! #4953: a `try`/`?` around a foreign `$x` bind chain that ends in a computed value, in a `reduce` UPDATE (#4934 residual).

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #4953: #4934's wrong accept for a chain that ends in a computed value instead of `.`. The accumulator
/// is the literal INIT (or a value the tail computed), no node of jq's register, so the final emission
/// raises `with result <that value>`; the binary took the bind step on a `null` `$x` for a raise (the
/// register was withheld from the body), the `try` swallowed it, and the empty UPDATE left the `null`
/// state a `null` register accepts -- `path` answered `[]`, `del` deleted the document and `|=` wrote
/// through it. A reduce UPDATE's binds all sit in tail position there, and `update_binds_only_in_tail`
/// did not read the `try`/`?` or the wrappers around it, so the register was never carried in. Covers an
/// object, an array and a nested pattern, a plain `$q`, tails `5`/`"s"`/`[1]`/`{}`/`$a`, INITs
/// `0`/`{}`/`[]`, and `limit`/`first`/`nth(0; ..)`/`if`/`label`/a comma around the `try`. Every row captured from `/usr/bin/jq`
/// 1.7.1, both routes.
#[test]
fn test_4953_reduce_update_try_around_foreign_bind_with_computed_tail_raises_on_null() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; try ($x as {a:$a} | 5)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "del(. as $x | reduce 1 as $i (0; try ($x as {a:$a} | 5)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "(. as $x | reduce 1 as $i (0; ($x as {a:$a} | 5)?)) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; try ($x as [$a] | 5)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "del(. as $x | reduce 1 as $i (0; try ($x as [$a] | 5)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "(. as $x | reduce .a as $i (0; try ($x as {a:{b:$a}} | 5))) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i ({}; try ($x as {a:$a} | 5)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce (1,2) as $i ([]; try ($x as $q | 5)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; try ($x as {a:$a} | \"s\")))",
            "",
            "Invalid path expression with result \"s\"",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; try ($x as {a:$a} | [1])))",
            "",
            "Invalid path expression with result [1]",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; try ($x as {a:$a} | $a)))",
            "[]\n",
            "",
            0,
        ),
        (
            "null",
            "del(. as $x | reduce 1 as $i (0; try ($x as {a:$a} | $a)))",
            "null\n",
            "",
            0,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; try ($x as {a:$a} | {})))",
            "",
            "Invalid path expression with result {}",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; try ($x as {a:$a} | 5), .))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; limit(1; try ($x as {a:$a} | 5))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; label $l | try ($x as {a:$a} | 5)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce .. as $i (0; try ($x as {a:$a} | 5)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; if true then try ($x as {a:$a} | 5) else . end))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "del(. as $x | reduce 1 as $i (0; if false then . else try ($x as {a:$a} | $a) end))",
            "null\n",
            "",
            0,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; first(try ($x as {a:$a} | 5))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "del(. as $x | reduce 1 as $i (0; nth(0; try ($x as {a:$a} | 5))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "(. as $x | reduce 1 as $i (0; limit(2; try ($x as {a:$a} | 5)))) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; ($x as {a:$a} | 5), ($x as {a:$a} | 6)))",
            "",
            "Invalid path expression with result 6",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; label $l | try ($x as {a:$a} | 5)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; try (try ($x as {a:$a} | 5))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
    ])
}

/// #4953: the same tails where the answer already matched or only the wording changed: a document that
/// is no `null`, whose wording named the bind's `null` where jq names the tail's value, and INIT `.`.
#[test]
fn test_4953_reduce_update_try_around_foreign_bind_with_computed_tail_off_null_and_dot_init(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "{\"a\":1}",
            "path(. as $x | reduce 1 as $i (0; try ($x as {a:$a} | 5)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "{\"a\":1}",
            "path(. as $x | reduce 1 as $i (0; try ($x as {a:$a} | $a)))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "path(. as $x | reduce 1 as $i (.; try ($x as {a:$a} | 5)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "del(. as $x | reduce 1 as $i (.; try ($x as {a:$a} | $a)))",
            "",
            "Invalid path expression with result {\"b\":1}",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (.; try ($x as {a:$a} | 5)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "(. as $x | reduce 1 as $i (.; ($x as {a:$a} | $a)?)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            "{\"a\":1}",
            "del(. as $x | reduce (1,2) as $i (.; try ($x as [$a] | 5)))",
            "",
            "Invalid path expression with result null",
            5,
        ),
    ])
}
