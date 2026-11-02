//! #4964: a `?//` chain that destructures an outer `$x` in a `reduce` UPDATE (#4953 residual).

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #4964: a `?//` chain that destructures an outer `$x` in a `reduce` UPDATE (under a `try`/`?` or bare),
/// with a literal INIT, hands a value on that is no node of jq's register, so the final emission raises
/// `with result <that value>`. The register was withheld from the body (a `?//` chain forks the walk, so
/// `fans_out` counts it), alternative one's pattern step on `$x` was refused as exact, the chain retried
/// onto the bare `$a` -- the whole of `$x`, the register's own node -- and the final emission accepted it:
/// `path` answered `[]`, `del` deleted the document and `|=` wrote through it. Every alternative here
/// starts from the register the chain was entered with (a failed one backtracks), so the chain is one
/// path-checked sequence like a single pattern. Covers object, array and nested alternatives, `try`/`?`/
/// bare, `$a`/`.`/`5` tails, a second bind in the chain, INITs `0`/`{}`, a navigating source, and
/// `limit`/`label` around it. Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
fn test_4964_reduce_update_alt_chain_bind_of_foreign_x_with_literal_init_raises() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; try ($x as {a:$a} ?// $a | $a)))",
            "[]\n",
            "",
            0,
        ),
        (
            "{\"a\":1}",
            "path(. as $x | reduce 1 as $i (0; try ($x as {a:$a} ?// $a | $a)))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":1}",
            "del(. as $x | reduce 1 as $i (0; try ($x as {a:$a} ?// $a | $a)))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":1}",
            "(. as $x | reduce 1 as $i (0; ($x as {a:$a} ?// $a | $a)?)) |= 9",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "[1]",
            "path(. as $x | reduce 1 as $i (0; try ($x as [$a] ?// $a | $a)))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "[1]",
            "del(. as $x | reduce 1 as $i (0; try ($x as [$a] ?// $a | $a)))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; $x as {a:$a} ?// $a | $a))",
            "[]\n",
            "",
            0,
        ),
        (
            "{\"a\":1}",
            "path(. as $x | reduce 1 as $i (0; $x as {a:$a} ?// $a | $a))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":1}",
            "del(. as $x | reduce 1 as $i (0; $x as {a:$a} ?// $a | $a))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "path(. as $x | reduce 1 as $i (0; try ($x as {a:{b:$a}} ?// $a | $a)))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; try ($x as {a:$a} | $x as {a:$a} ?// $a | .)))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "{\"a\":1}",
            "del(. as $x | reduce 1 as $i (0; try ($x as {a:$a} | $x as {a:$a} ?// $a | .)))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "{\"a\":1}",
            "path(. as $x | reduce 1 as $i ({}; try ($x as {a:$a} ?// $a | .)))",
            "",
            "Invalid path expression with result {}",
            5,
        ),
        (
            "{\"a\":1}",
            "path(. as $x | reduce (1,2) as $i (0; limit(1; try ($x as {a:$a} ?// $a | $a))))",
            "[]\n",
            "",
            0,
        ),
        (
            "{\"a\":1}",
            "path(. as $x | reduce 1 as $i (0; label $l | try ($x as {a:$a} ?// $a | 5)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "[1]",
            "path(. as $x | reduce .[] as $i (0; try ($x as {a:$a} ?// $a | $a)))",
            "[]\n",
            "",
            0,
        ),
        (
            "{\"a\":1}",
            "(. as $x | reduce .a as $i (0; try ($x as {a:$a} ?// $a | $a))) |= 9",
            "9\n",
            "",
            0,
        ),
    ])
}

/// #4964: INIT `.` makes the accumulator the register's node, so a `?//` chain that ends in `.` hands it
/// back and jq answers `[]` (`del` deletes the document, `|= 9` replaces it) -- including when a
/// navigating source has moved the register off `$x` from the first step, so alternative one raises on
/// the path check and the bare alternative runs. The `{"a":1}`-shaped rows were a wrong refusal; the
/// navigating-source ones matched before and must keep matching (the bare alternative's `.` is left
/// unchecked there as for a bind in tail position, `state_handed_on_here`).
#[test]
fn test_4964_reduce_update_alt_chain_bind_of_foreign_x_with_dot_init_hands_the_accumulator_back(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "{\"a\":1}",
            "path(. as $x | reduce 1 as $i (.; try ($x as {a:$a} ?// $a | .)))",
            "[]\n",
            "",
            0,
        ),
        (
            "{\"a\":1}",
            "del(. as $x | reduce 1 as $i (.; $x as {a:$a} ?// $a | .))",
            "null\n",
            "",
            0,
        ),
        (
            "{\"a\":1}",
            "(. as $x | reduce 1 as $i (.; ($x as {a:$a} ?// $a | .)?)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            "[1]",
            "path(. as $x | reduce 1 as $i (.; try ($x as [$a] ?// $a | .)))",
            "[]\n",
            "",
            0,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "del(. as $x | reduce 1 as $i (.; $x as {a:{b:$a}} ?// $a | .))",
            "null\n",
            "",
            0,
        ),
        (
            "[1]",
            "path(. as $x | reduce .. as $i (.; $x as {a:$a} ?// $a | .))",
            "[]\n",
            "",
            0,
        ),
        (
            "[1]",
            "del(. as $x | reduce .[] as $i (.; try ($x as {a:$a} ?// $a | .)))",
            "null\n",
            "",
            0,
        ),
        (
            "[[1]]",
            "(. as $x | reduce .[] as $i (.; ($x as {a:$a} ?// $a | .)?)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            "[{\"a\":1}]",
            "path(. as $x | reduce .[0] as $i (.; $x as {a:$a} ?// $a | .))",
            "[]\n",
            "",
            0,
        ),
        (
            "null",
            "path(. as $x | reduce .. as $i (.; $x as {a:$a} ?// $a | .))",
            "[]\n",
            "",
            0,
        ),
        (
            "{\"a\":[1]}",
            "path(. as $x | reduce .[] as $i (.; $x as {a:$a} ?// $a | .))",
            "[]\n",
            "",
            0,
        ),
        (
            "{\"a\":1}",
            "path(. as $x | reduce (1,2) as $i (.; $x as {a:$a} ?// $a | .))",
            "[]\n",
            "",
            0,
        ),
    ])
}
