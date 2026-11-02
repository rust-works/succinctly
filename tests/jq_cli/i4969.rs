//! #4969: a `?//` chain that destructures `.` in a `reduce` UPDATE over a source whose first element is the
//! accumulator itself (#4964 residual).

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #4969: `reduce .. as $i (.; . as {a:$a} ?// $a | $a)` -- the source's first output is `.` itself, so at
/// step 1 the accumulator (INIT `.`) IS jq's register and the first alternative's pattern step passes. The
/// binary marked the step as one whose source had moved the register off the accumulator (right for
/// `.[]?` or `.a?`, whose element is a child), so alternative one was refused as exact, the chain retried
/// onto the bare `$a` -- all of `.` -- and every later step kept the root: the final emission accepted it,
/// where jq's `$a` is the child the pattern bound (`path` answered `[]`, `del` deleted the document, `|=`
/// wrote through it). Every row captured from `/usr/bin/jq` 1.7.1, both routes; the rows jq answers are the
/// controls that must keep answering (a child element, a `try`, a `?`, a computed tail).
#[test]
fn test_4969_reduce_update_alt_chain_bind_of_dot_over_a_source_starting_at_the_accumulator_raises(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "{\"a\":1}",
            "path(reduce .. as $i (.; . as {a:$a} ?// $a | $a))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":1}",
            "path(reduce .. as $i (.; try (. as {a:$a} ?// $a | $a)))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":1}",
            "path(reduce .. as $i (.; (. as {a:$a} ?// $a | $a)?))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":1}",
            "path(reduce .. as $i (.; . as {a:$a} ?// $a | 5))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "{\"a\":1}",
            "del(reduce .. as $i (.; . as {a:$a} ?// $a | $a))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":1}",
            "del(reduce .. as $i (.; try (. as {a:$a} ?// $a | $a)))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":1}",
            "del(reduce .. as $i (.; (. as {a:$a} ?// $a | $a)?))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":1}",
            "del(reduce .. as $i (.; . as {a:$a} ?// $a | 5))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "{\"a\":1}",
            "(reduce .. as $i (.; . as {a:$a} ?// $a | $a)) |= 9",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":1}",
            "(reduce .. as $i (.; try (. as {a:$a} ?// $a | $a))) |= 9",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":1}",
            "(reduce .. as $i (.; (. as {a:$a} ?// $a | $a)?)) |= 9",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":1}",
            "(reduce .. as $i (.; . as {a:$a} ?// $a | 5)) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "path(reduce .. as $i (.; . as {a:$a} ?// $a | 5))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "del(reduce .. as $i (.; . as {a:$a} ?// $a | 5))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "(reduce .. as $i (.; . as {a:$a} ?// $a | 5)) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "[1]",
            "path(reduce .. as $i (.; . as {a:$a} ?// $a | 5))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "[1]",
            "del(reduce .. as $i (.; . as {a:$a} ?// $a | 5))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "[1]",
            "(reduce .. as $i (.; . as {a:$a} ?// $a | 5)) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "path(reduce .. as $i (.; . as {a:$a} ?// $a | 5))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "del(reduce .. as $i (.; . as {a:$a} ?// $a | 5))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "null",
            "(reduce .. as $i (.; . as {a:$a} ?// $a | 5)) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            "{\"a\":1}",
            "path(reduce (.,.a?) as $i (.; try (. as {a:$a} ?// $a | $a)))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":1}",
            "del(reduce (.,.a?) as $i (.; . as {a:$a} ?// $a | $a))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":1}",
            "path(reduce first(..) as $i (.; try (. as {a:$a} ?// $a | $a)))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":1}",
            "del(reduce first(..) as $i (.; . as {a:$a} ?// $a | $a))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "path(reduce first(..) as $i (.; try (. as {a:$a} ?// $a | $a)))",
            "",
            "Invalid path expression with result {\"b\":1}",
            5,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "del(reduce first(..) as $i (.; . as {a:$a} ?// $a | $a))",
            "",
            "Invalid path expression with result {\"b\":1}",
            5,
        ),
        (
            "{\"a\":1}",
            "path(reduce .. as $i (.; try (. as {a:$a} | $a)))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            "{\"a\":1}",
            "del(reduce 1 as $i (.; . as {a:$a} ?// $a | $a))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
    ])
}

/// #4969: what jq answers must keep answering -- INIT `.` with a navigating source whose elements are
/// children, a single pattern, and the `.` tail that hands the accumulator back.
#[test]
fn test_4969_reduce_update_alt_chain_bind_of_dot_controls_that_answer() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "[1]",
            "path(reduce .. as $i (.; . as {a:$a} ?// $a | $a))",
            "[]\n",
            "",
            0,
        ),
        (
            "[1]",
            "path(reduce .. as $i (.; try (. as {a:$a} ?// $a | $a)))",
            "[]\n",
            "",
            0,
        ),
        (
            "[1]",
            "path(reduce .. as $i (.; (. as {a:$a} ?// $a | $a)?))",
            "[]\n",
            "",
            0,
        ),
        (
            "[1]",
            "del(reduce .. as $i (.; . as {a:$a} ?// $a | $a))",
            "null\n",
            "",
            0,
        ),
        (
            "[1]",
            "del(reduce .. as $i (.; try (. as {a:$a} ?// $a | $a)))",
            "null\n",
            "",
            0,
        ),
        (
            "[1]",
            "del(reduce .. as $i (.; (. as {a:$a} ?// $a | $a)?))",
            "null\n",
            "",
            0,
        ),
        (
            "[1]",
            "(reduce .. as $i (.; . as {a:$a} ?// $a | $a)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            "[1]",
            "(reduce .. as $i (.; try (. as {a:$a} ?// $a | $a))) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            "[1]",
            "(reduce .. as $i (.; (. as {a:$a} ?// $a | $a)?)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            "null",
            "path(reduce .. as $i (.; . as {a:$a} ?// $a | $a))",
            "[]\n",
            "",
            0,
        ),
        (
            "null",
            "path(reduce .. as $i (.; try (. as {a:$a} ?// $a | $a)))",
            "[]\n",
            "",
            0,
        ),
        (
            "null",
            "path(reduce .. as $i (.; (. as {a:$a} ?// $a | $a)?))",
            "[]\n",
            "",
            0,
        ),
        (
            "null",
            "del(reduce .. as $i (.; . as {a:$a} ?// $a | $a))",
            "null\n",
            "",
            0,
        ),
        (
            "null",
            "del(reduce .. as $i (.; try (. as {a:$a} ?// $a | $a)))",
            "null\n",
            "",
            0,
        ),
        (
            "null",
            "del(reduce .. as $i (.; (. as {a:$a} ?// $a | $a)?))",
            "null\n",
            "",
            0,
        ),
        (
            "null",
            "(reduce .. as $i (.; . as {a:$a} ?// $a | $a)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            "null",
            "(reduce .. as $i (.; try (. as {a:$a} ?// $a | $a))) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            "null",
            "(reduce .. as $i (.; (. as {a:$a} ?// $a | $a)?)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            "{\"a\":1}",
            "path(reduce .[]? as $i (.; try (. as {a:$a} ?// $a | $a)))",
            "[]\n",
            "",
            0,
        ),
        (
            "{\"a\":1}",
            "del(reduce .[]? as $i (.; . as {a:$a} ?// $a | $a))",
            "null\n",
            "",
            0,
        ),
        (
            "{\"a\":1}",
            "path(reduce .a? as $i (.; try (. as {a:$a} ?// $a | $a)))",
            "[]\n",
            "",
            0,
        ),
        (
            "{\"a\":1}",
            "del(reduce .a? as $i (.; . as {a:$a} ?// $a | $a))",
            "null\n",
            "",
            0,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "path(reduce .[]? as $i (.; try (. as {a:$a} ?// $a | $a)))",
            "[]\n",
            "",
            0,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "del(reduce .[]? as $i (.; . as {a:$a} ?// $a | $a))",
            "null\n",
            "",
            0,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "path(reduce .a? as $i (.; try (. as {a:$a} ?// $a | $a)))",
            "[]\n",
            "",
            0,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "del(reduce .a? as $i (.; . as {a:$a} ?// $a | $a))",
            "null\n",
            "",
            0,
        ),
        (
            "{\"a\":{\"b\":1}}",
            "del(reduce .. as $i (.; . as [$a] ?// $a | $a))",
            "null\n",
            "",
            0,
        ),
    ])
}

/// #4969: the `?//` chains with no bare `$name` alternative must keep their exact wording. Their retry
/// target is another pattern, not the whole accumulator, so the first alternative's refusal names the step
/// jq names (`near attempt to access element "a" of ...`); the exemption for a source starting at the
/// accumulator is read only for a chain holding a bare `$name`. Also the single-pattern forms, whose
/// wording was never touched.
#[test]
fn test_4969_reduce_update_alt_chain_without_a_bare_alternative_keeps_its_wording() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "{\"a\":1}",
            "path(reduce .. as $i (.; . as [$a] ?// {a:$a} | .))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":1}",
            5,
        ),
        (
            "{\"a\":1}",
            "del(reduce .. as $i (.; . as [$a] ?// {a:$a} | .))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":1}",
            5,
        ),
        (
            "{\"a\":1}",
            "path(reduce (.,.a?) as $i (.; . as [$a] ?// {a:$a} | .))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":1}",
            5,
        ),
        (
            "{\"a\":1}",
            "del(reduce (.,.a?) as $i (.; . as [$a] ?// {a:$a} | .))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":1}",
            5,
        ),
        (
            "{\"a\":1}",
            "path(reduce (.,..) as $i (.; . as [$a] ?// {a:$a} | .))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":1}",
            5,
        ),
        (
            "{\"a\":1}",
            "del(reduce (.,..) as $i (.; . as [$a] ?// {a:$a} | .))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":1}",
            5,
        ),
        (
            "{\"a\":1}",
            "path(reduce limit(2;..) as $i (.; . as [$a] ?// {a:$a} | .))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":1}",
            5,
        ),
        (
            "{\"a\":1}",
            "del(reduce limit(2;..) as $i (.; . as [$a] ?// {a:$a} | .))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":1}",
            5,
        ),
        (
            "{\"a\":1}",
            "path(reduce (.,.[]?) as $i (.; . as [$a] ?// {a:$a} | .))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":1}",
            5,
        ),
        (
            "{\"a\":1}",
            "del(reduce (.,.[]?) as $i (.; . as [$a] ?// {a:$a} | .))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":1}",
            5,
        ),
    ])
}
