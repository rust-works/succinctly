//! #4934: a `try`/`?` around a destructure of an outer `$x` in a `reduce` UPDATE (#4901, #4932 residual).

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #4934: a `reduce` whose UPDATE catches (`try`/`?`) a destructure of an outer `$x` and whose INIT is a
/// literal hands that literal on, which is no node of jq's register, so the final emission raises
/// `with result <INIT>`. The binary took the bind step for a raise on a `null` document (the register
/// was withheld from the body, so a `null` `$x` against it was refused as exact), the `try` swallowed
/// it, the UPDATE read as empty and the state as `null`, which a `null` register accepts: `path`
/// answered `[]`, `del` deleted the document and `|=`/`=` wrote through it. Covers `try` and `?`, an
/// object, an array and a nested pattern, a plain `$q`, INITs `0`/`{}`/`[]`, a one-shot, a comma, a
/// navigating and a non-navigating source, and the wrappers around the `try` (`limit`, `first`,
/// `label`, `if`, a comma). Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
fn test_4934_reduce_update_try_around_foreign_bind_with_literal_init_raises_on_null() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; try ($x as {a:$a} | .)))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "del(. as $x | reduce 1 as $i (0; try ($x as {a:$a} | .)))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "(. as $x | reduce 1 as $i (0; ($x as {a:$a} | .)?)) |= 9",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; ($x as {a:$a} | .)?))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; try $x as {a:$a} | .))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i ({}; try ($x as {a:$a} | .)))",
            "",
            "Invalid path expression with result {}",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i ([]; try ($x as {a:$a} | .)))",
            "",
            "Invalid path expression with result []",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; try ($x as [$a] | .)))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; try ($x as {a:{b:$a}} | .)))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; try ($x as $q | .)))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce (1,2) as $i (0; try ($x as {a:$a} | .)))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce .a as $i (0; try ($x as {a:$a} | .)))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce .a? as $i (0; try ($x as {a:$a} | .)))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce .. as $i (0; try ($x as {a:$a} | .)))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "del(. as $x | reduce (1,2) as $i (0; limit(1; try ($x as {a:$a} | .))))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "(. as $x | reduce 1 as $i (0; label $l | try ($x as {a:$a} | .))) |= 9",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; if true then try ($x as {a:$a} | .) else . end))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; first(try ($x as {a:$a} | .))))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (0; try ($x as {a:$a} | .), .))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
    ])
}

/// #4934: the same raise on a document that is no `null`, where the wording used to name the bind's
/// `null` where jq names the literal accumulator (`with result 0`).
#[test]
fn test_4934_reduce_update_try_around_foreign_bind_with_literal_init_raises_off_null() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[
        (
            "{\"a\":1}",
            "path(. as $x | reduce 1 as $i (0; try ($x as {a:$a} | .)))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "{\"a\":1}",
            "del(. as $x | reduce 1 as $i (0; ($x as {a:$a} | .)?))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            "{\"a\":null}",
            "path(. as $x | reduce 1 as $i ({}; try ($x as {a:$a} | .)))",
            "",
            "Invalid path expression with result {}",
            5,
        ),
        (
            "[1]",
            "path(. as $x | reduce 1 as $i (0; try ($x as [$a] | .)))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
    ])
}

/// #4934: INIT `.` makes the accumulator the register's node, so a `try`/`?` around the destructure
/// hands it back and jq answers `[]` (`del` deletes the document, `|= 9` replaces it). The `{"a":1}`
/// and `{"a":{"b":1}}` rows were a wrong refusal (`with result <document>`): the withheld register
/// refused the bind step and the `try` made the UPDATE empty.
#[test]
fn test_4934_reduce_update_try_around_foreign_bind_with_dot_init_still_answers() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        // A navigation after the bind is a control that answers: `.a` on a `null` `$x` stays on the register.
        (
            "null",
            "path(. as $x | reduce 1 as $i (.; try ($x as {a:$a} | .a)))",
            "[]\n",
            "",
            0,
        ),
        (
            "null",
            "path(. as $x | reduce 1 as $i (.; try ($x as {a:$a} | .)))",
            "[]\n",
            "",
            0,
        ),
        (
            "null",
            "del(. as $x | reduce 1 as $i (.; try ($x as {a:$a} | .)))",
            "null\n",
            "",
            0,
        ),
        (
            "null",
            "(. as $x | reduce 1 as $i (.; ($x as {a:$a} | .)?)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            "{\"a\":1}",
            "path(. as $x | reduce 1 as $i (.; try ($x as {a:$a} | .)))",
            "[]\n",
            "",
            0,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "del(. as $x | reduce 1 as $i (.; try ($x as {a:$a} | .), .))",
            "null\n",
            "",
            0,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "(. as $x | reduce 1 as $i (.; try ($x as {a:$a} | .))) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            "{\"a\":null}",
            "path(. as $x | reduce 1 as $i (.; label $l | try ($x as {a:$a} | .)))",
            "[]\n",
            "",
            0,
        ),
        (
            "{\"a\":null}",
            "del(. as $x | reduce 1 as $i (.; if true then try ($x as {a:$a} | .) else . end))",
            "null\n",
            "",
            0,
        ),
        (
            "{\"a\":1}",
            "path(. as $x | reduce (1,2) as $i (.; ($x as {a:$a} | .)?))",
            "[]\n",
            "",
            0,
        ),
        (
            "{\"a\":1}",
            "path(. as $x | reduce 1 as $i (.; limit(1; try ($x as {a:$a} | .))))",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #4934: what must keep raising -- a computed or navigating tail after the destructure is no accumulator.
#[test]
fn test_4934_reduce_update_try_around_foreign_bind_still_refuses_a_navigating_or_computed_tail(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "{\"a\":{\"b\":1}}",
            "path(. as $x | reduce 1 as $i (.; try ($x as {a:$a} | $a)))",
            "",
            "Invalid path expression with result {\"b\":1}",
            5,
        ),
        (
            "{\"a\":1}",
            "path(. as $x | reduce 1 as $i (.; try ($x as {a:$a} | $a)))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "path(. as $x | reduce 1 as $i (.; try ($x as {a:$a} | . as {a:$b} | .)))",
            "",
            "Invalid path expression with result null",
            5,
        ),
    ])
}

/// #4934: a `try` body that raises on some steps and not others. A navigating source leaves jq's
/// register on the element, so `$x` is no longer it from the first step and the bind raises, the
/// `try` catches it, the UPDATE is empty and the state `null`; the register is carried in so each
/// step is judged against where jq has it. A one-shot (`limit`, `first`) around the `try` keeps that
/// verdict. Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
fn test_4934_reduce_update_try_around_foreign_bind_raising_on_later_steps() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "{\"a\":{\"b\":1}}",
            "path(. as $x | reduce .a as $i (.; try ($x as {a:$a} | .)))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "del(. as $x | reduce .a as $i (.; ($x as {a:$a} | .)?))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "(. as $x | reduce .. as $i (.; try ($x as {a:$a} | .))) |= 9",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            "[[1]]",
            "path(. as $x | reduce .[] as $i (.; ($x as [$a] | .)?))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            "[[1]]",
            "del(. as $x | reduce .[] as $i (.; try ($x as [$a] | .)))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            "{\"a\":null}",
            "path(. as $x | reduce .a as $i (.; try ($x as {a:$a} | .)))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            "{\"a\":null}",
            "del(. as $x | reduce .a? as $i (0; try ($x as {a:$a} | .)))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "path(. as $x | reduce .a as $i (0; try ($x as {a:$a} | .)))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "(. as $x | reduce (.a, .) as $i (.; try ($x as {a:$a} | .))) |= 9",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            "{\"a\":[1,2]}",
            "path(. as $x | reduce .a[] as $i (.; limit(1; try ($x as {a:$a} | .))))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            "{\"a\":[1,2]}",
            "del(. as $x | reduce .a[] as $i (.; first(try ($x as {a:$a} | .))))",
            "",
            "Invalid path expression with result null",
            5,
        ),
    ])
}
