//! #5012: a `catch` handler around the `foreach` a `foreach` UPDATE is made of (#5004), on a step whose state is a
//! container. The nested `foreach`'s source destructures a `$x` that raises at the pattern's first step whenever `$x`
//! is not the element the outer source moved jq's register onto, and, where it is, for a pattern that mismatches its
//! value's type or has a second entry (checked against the child the first moved the register onto). Either way the
//! raise precedes the state, so the handler's output is placed at the `try`'s entry on that step too, and on every
//! step of the run, not only its last.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #5012: the issue's rows. On `{"a":[null]}` the step's element is `.a`, which `$x` (the root) is not, so the nested
/// source raises, the `try` restores the register to the element and its `catch $i` is emitted there. The state is the
/// root object, a container, which the #4800 reading needed to be a `null`/boolean: the handler was run with no register
/// and `$i` refused as exact, so `[path(..)?]` dropped jq's path. Then a step before the run's last, and a source with no
/// bound (`.[]?`). Rows captured from jq 1.7.1 (stdout, first stderr line, exit code), both routes.
#[test]
fn test_foreach_update_try_handler_on_a_container_state_sits_at_the_try_entry_5012() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":[null]}"#,
            r"[path(. as $x | foreach (.a) as $i (.; try (foreach ($x as {a:$a} | .) as $y (.; .a?; .)) catch $i; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"path(. as $x | foreach (.a) as $i (.; try (foreach ($x as {a:$a} | .) as $y (.; .a?; .)) catch $i; $i))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"(. as $x | foreach (.a) as $i (.; try (foreach ($x as {a:$a} | .) as $y (.; .a?; .)) catch $i; $i)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"del(. as $x | foreach (.a) as $i (.; try (foreach ($x as {a:$a} | .) as $y (.; .a?; .)) catch $i; $i))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"[path(. as $x | foreach (.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (.; .a?; .)) catch $i; $i))?]",
            "[[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"[path(. as $x | foreach (.a,.,.a) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch 7; $i))?]"#,
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r#"del(. as $x | foreach (.[]?) as $i (.; not | (try (foreach ($x as {a:$a} | .) as $y (0; .a?; error("x"))) catch $x); $i))"#,
            "{}\n",
            "",
            0,
        ),
    ])
}

/// #5012: a later step whose element is the root, which `$x` is (by value, and here by node). An array pattern on an
/// object raises `Cannot index object with number`, `{a:$a,b:$b}` raises at `.b`, checked against `.a`, and
/// `{a:{b:$b}}` raises at `.b` of the array `.a` its first step moved the register onto, so the nested `foreach` raises
/// whatever state the earlier step's handler left. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_try_handler_where_the_pattern_raises_on_the_register_5012() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":[1]}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (0; try (foreach ($x as {a:{b:$b}} | .) as $y (.; error("x"); .a)) catch 7; $i)) catch "C""#,
            "[\"a\"]\n[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r#"[path(. as $x | foreach (.a,.) as $i (0; try (foreach ($x as {a:{b:$b}} | .) as $y (.; error("x"); .a)) catch 7; $i))?]"#,
            "[[\"a\"],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a?,.) as $i (0; try (foreach ($x as [$a] | .) as $y (null; .; $y)) catch $i; $i))?]",
            "[[\"a\"],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a?,.) as $i (0; try (foreach ($x as [$a] | .) as $y (null; .; $y)) catch $i; $i))",
            "[\"a\"]\n[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r#"[path(. as $x | foreach (.a?,.) as $i (0; not | try (foreach ($x as [$a] | .) as $y (.; error("x"); $i)) catch .; $i))?]"#,
            "[[\"a\"],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach ((.a,.a),.) as $i (null; try (foreach ($x as {a:$a,b:$b} | .) as $y (.; error("x"); $i)) catch .; $i)) catch 5"#,
            "[\"a\"]\n[\"a\"]\n[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":true}}"#,
            r"try path(. as $x | foreach (.a,$x) as $i (0; (try (foreach ($x as {a:$a,b:$b} | .) as $y (0; .a?; $i)) catch 7)?; $i)) catch 5",
            "[\"a\"]\n[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"(. as $x | foreach (.a?,.) as $i (.; try (foreach ($x as {a:$a,b:$b} | .) as $y (null; .; .)) catch "c"; $i)) |= 9"#,
            "9\n",
            "",
            0,
        ),
    ])
}

/// #5012: a later step whose element `$x` is, with a one-entry pattern: jq's register lands on `$x.a`, and the state the
/// earlier step's handler left (`"c"`, or the root) is not equal to that landing, so it is never `jv_identical` to it
/// and the pattern's `.` body emits it off the register, as #4800 reads a `null`/boolean state. Rows captured from jq
/// 1.7.1, both routes.
#[test]
fn test_foreach_update_try_handler_where_the_landing_is_not_the_state_5012() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":true}"#,
            r#"[path(. as $x | foreach (.a?,.) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch "c"; $i))?]"#,
            "[[\"a\"],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r#"path(. as $x | foreach (.a?,.) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch "c"; $i))"#,
            "[\"a\"]\n[]\n",
            "",
            0,
        ),
    ])
}

/// #5012: with no handler, a two-entry pattern on the root element, which `$x` is: its second step is checked against
/// the child the first moved the register onto and raises, the source's own `try` empties it, and the `foreach` emits
/// nothing, so `path` is empty and `del`/`|=` leave the document. Read now that a pattern that raises on the register
/// is read whatever the state (#4800 pinned these refused). Rows captured from jq 1.7.1, both routes.
#[test]
fn test_caught_two_entry_destructure_on_the_register_empties_the_step_5012() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null,"b":false}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$p,b:$q} | .)) as $y (.; .; .); .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$p,b:$q} | .)) as $y (.; .; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":false}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$p,b:$q} | .)) as $y (.; .; .); .))",
            "{\"a\":null,\"b\":false}\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":false}"#,
            r"(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$p,b:$q} | .)) as $y (.; .; .); .)) |= 9",
            "{\"a\":null,\"b\":false}\n",
            "",
            0,
        ),
        (
            r"[null,true]",
            r"path(. as $x | foreach (.[0],.) as $i (.; foreach (try ($x as [$p,$q] | .)) as $y (.; .; .); .))",
            "",
            "",
            0,
        ),
        (
            r"[null,true]",
            r"[path(. as $x | foreach (.[0],.) as $i (.; foreach (try ($x as [$p,$q] | .)) as $y (.; .; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r"[null,true]",
            r"del(. as $x | foreach (.[0],.) as $i (.; foreach (try ($x as [$p,$q] | .)) as $y (.; .; .); .))",
            "[null,true]\n",
            "",
            0,
        ),
        (
            r"[null,true]",
            r"(. as $x | foreach (.[0],.) as $i (.; foreach (try ($x as [$p,$q] | .)) as $y (.; .; .); .)) |= 9",
            "[null,true]\n",
            "",
            0,
        ),
    ])
}

/// #5012: every step of `..`: the root (whose state is `null`, #4800's reading), then `.a` and `.a.a`, whose state is
/// the root the previous handler emitted. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_try_handler_on_every_step_of_an_unbounded_source_5012() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":true}}"#,
            r"[path(. as $x | foreach (..) as $i (null; (try (foreach ($x as {a:$a} | .) as $y (.; .a?; $y)) catch $i)?; .))?]",
            "[[],[\"a\"],[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":true}}"#,
            r"del(. as $x | foreach (..) as $i (null; (try (foreach ($x as {a:$a} | .) as $y (.; .a?; $y)) catch $i)?; .))",
            "null\n",
            "",
            0,
        ),
    ])
}

/// #5012: the rows #5004 left refused because the step was not provably the run's last: a source with no bound
/// (`.[]?`, `..`), a step before the bound, and the last step of an INIT fork that is not the last (`(.,null)`). Every
/// such step is now placed, and each later one is covered. Then rows #5004 kept refused as not modelled, which the
/// reading now covers: a two-entry pattern (`{a:$a,b:$b}`), whose second step raises against the moved register and
/// is caught by the source's own `try`, so the `foreach` emits nothing, as in jq. Rows captured from jq 1.7.1, both
/// routes.
#[test]
fn test_foreach_update_try_handler_before_the_run_last_step_is_placed_5012() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":[1]}"#,
            r#"del(. as $x | foreach (.[]?) as $i (.; not | (try (foreach ($x as {a:$a} | .) as $y (0; .a?; error("x"))) catch $x); $i))"#,
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r#"del(. as $x | foreach (.a,.) as $i (.; not | (try (foreach ($x as {a:$a} | .) as $y (0; .a?; error("x"))) catch $x); $i))"#,
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"[path(. as $x | foreach (.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (.; .a?; .)) catch $i; $i))?]",
            "[[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"path(. as $x | foreach (.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (.; .a?; .)) catch $i; $i))",
            "[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{}}"#,
            r#"try del(. as $x | foreach (.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (.; .; .a)) catch (.); $i)) catch "c""#,
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.,.a) as $i (null; try (try (foreach (try ($x as {a:$a} | .)) as $y (null; try (.a) catch 7; .)) catch select(false)) catch $i; $i))",
            "",
            "Invalid path expression with result {\"a\":{\"a\":null}}",
            5,
        ),
        (
            r#"{"a":[null]}"#,
            r"(. as $x | foreach (.a,.) as $i (null; null | (try (foreach ($x as {a:$a} | .) as $y (null; empty; error($x))) catch $__loc__); $i)) = 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"[path(. as $x | foreach (.,.a) as $i (0; null | (try (foreach (($x as {a:$a} | .)?) as $y (.; .a?; empty)) catch 7); $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":1}}"#,
            r#"try path(. as $x | foreach (..) as $i (null; select(true) | (try (foreach (($x as {a:$a} | .)?) as $y (.; .a?; .)) catch 7); $i)) catch "C""#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":true}}"#,
            r#"try path(. as $x | foreach ((.,.)) as $i (.; not | (try (foreach ($x as {a:$a} | .) as $y (.; .; error)) catch "h"); $i | .a)) catch "C""#,
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":2}}"#,
            r"del(. as $x | foreach ((.,.)) as $i (null; (try (foreach (try ($x as {a:$a} | .)) as $y (0; .; error($x))) catch null) | .; $x))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"del(. as $x | foreach (.,.a) as $i (null; try (try (foreach (try ($x as {a:$a} | .)) as $y (null; try (.a) catch 7; .)) catch select(false)) catch (.); $i))",
            "",
            "Invalid path expression with result {\"a\":[null]}",
            5,
        ),
        (
            r#"{"a":[null]}"#,
            r#"(. as $x | foreach (.,.a) as $i (null; try (try (foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; error("x"))) catch select(false)) catch 7; $i)) |= 9"#,
            "{\"a\":[null]}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":true}}"#,
            r"path(. as $x | foreach (.,.a) as $i (null; try (try (foreach (try ($x as {a:$a} | .)) as $y (null; try (.a) catch 7; .)) catch select(false)) catch 7; $i))",
            "",
            "Invalid path expression with result {\"a\":{\"a\":true}}",
            5,
        ),
        (
            r#"{"a":{"a":true}}"#,
            r#"[path(. as $x | foreach (.,.a) as $i (null; try (try (foreach (try ($x as {a:$a} | .)) as $y (null; try (.a) catch 7; error("x"))) catch select(false)) catch (.); $i))?]"#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.,.a) as $i (null; try (try (foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)) catch select(false)) catch 7; $i))",
            "",
            "Invalid path expression with result {\"a\":{\"a\":null}}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"[path(. as $x | foreach (.,.a) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (null; try (.a) catch 7; error("x"))) catch (.); $i))?]"#,
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"path(. as $x | foreach (.,.a) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (null; try (.a) catch 7; error("x"))) catch (.); $i))"#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"(. as $x | foreach (.,.a) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (null; try (.a) catch 7; error("x"))) catch (.); $i)) |= 9"#,
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"del(. as $x | foreach (.,.a) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (null; try (.a) catch 7; error("x"))) catch (.); $i))"#,
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"[path(. as $x | foreach (.,.a) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (null; try (.a) catch 7; error("x"))) catch 7; $i))?]"#,
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"path(. as $x | foreach (.,.a) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (null; try (.a) catch 7; error("x"))) catch 7; $i))"#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"path(. as $x | foreach (.a,.) as $i ((.,null); try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch (.); $i))"#,
            "[]\n",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":[1]}"#,
            r"(. as $x | foreach (.) as $i (0; try (foreach (try ($x as {a:$a,b:$b} | .)) as $y (.; .; .a)) catch $x; $i | .a)) |= 9",
            "{\"a\":[1]}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r"[path(. as $x | foreach (.,.a) as $i (0; try (foreach (try ($x as {a:$a,b:$b} | .)) as $y (.; $y; error($x))) catch $x; .a?))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":{"a":null}}}"#,
            r"path(. as $x | foreach (.a,.) as $i (0; try (foreach (try ($x as {a:$a,b:$b} | .)) as $y (null; .; error($x))) catch (.); $i))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"(. as $x | foreach (.,.a) as $i (0; first(try (foreach (try ($x as {a:$a,b:$b} | .)) as $y (0; .a?; error(.))) catch $x); .)) |= 9",
            "{\"a\":1}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach ((.a,.),.) as $i (.; try (foreach (try ($x as {a:$a,b:$b} | .)) as $y (.; $y; error({}))) catch (., 1); $i)) = 9",
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":1}}"#,
            r"del(. as $x | foreach ((.a,.),.) as $i (.; try (foreach (try ($x as {a:$a,b:$b} | .)) as $y (null; $y; error({}))) catch null; $i))",
            "{\"a\":{\"a\":1}}\n",
            "",
            0,
        ),
    ])
}

/// #5012: what the reading does not cover keeps `main`'s behaviour. A nested INIT that navigates (`.a?`) runs before
/// the source in jq and moves the register first; a `try .. catch 1` around the source's destructure emits from the
/// raise. (A step whose element `$x` is, with a one-entry pattern landing on the state, is read by position since
/// #5179, `tests/jq_cli/i5179.rs`.) Both routes.
#[test]
fn test_foreach_update_try_handler_where_the_reading_does_not_hold_stays_refused_5012() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[
        (r#"{"a":[null]}"#, "path(. as $x | foreach (.a,.) as $i (.; try (foreach ($x as {a:$a} | .) as $y (.a?; .; .)) catch $i; $i))", "[\"a\"]\n[]\n", "", 0),
        (r#"{"a":[null]}"#, "path(. as $x | foreach (.a,.) as $i (null; try (foreach (try ($x as {a:$a} | .) catch 1) as $y (.; .a?; .)) catch $i; $i))", "[\"a\"]\n", "", 0),
    ])?;
    Ok(())
}
