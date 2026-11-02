//! #5004: a `catch` handler's output in a `foreach` UPDATE over a moving source, when the UPDATE is a `try` around the
//! caught foreign-`$x` `foreach` of #4800 (`FE'` = `foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))`, whose
//! EXTRACT raises).

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #5004: at the step whose element is the root, `$x` is jq's register, the pattern's step moves it onto `.a` and the
/// nested `foreach`'s EXTRACT raises. The `try` backtracks to its fork point, which restores the register to where it
/// entered (the root element), and the handler's output is emitted there, so the `$i` EXTRACT (the root) is
/// `jv_identical` to it and the path is `[]`. The handler was seeded with no register (the fold's frame carries none),
/// so `$i` was refused as a guess: `path`, `|=` and `del` raised `with result <doc>` and `[path(..)?]` dropped jq's
/// `[]`. For a `null` landing and a container landing, an emitting handler of either kind. Rows captured from jq 1.7.1
/// (stdout, first stderr line, exit code), both routes.
#[test]
fn test_foreach_update_try_handler_output_sits_at_the_try_entry_5004() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r#"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch (.); $i))?]"#,
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch (.); $i))"#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch (.); $i)) |= 9"#,
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"del(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch (.); $i))"#,
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch 7; $i))?]"#,
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch 7; $i))"#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch 7; $i)) |= 9"#,
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"del(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch 7; $i))"#,
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch (.); $i))?]"#,
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch (.); $i))"#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch (.); $i)) |= 9"#,
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"del(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch (.); $i))"#,
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch 7; $i))?]"#,
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch 7; $i))"#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch 7; $i)) |= 9"#,
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"del(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch 7; $i))"#,
            "null\n",
            "",
            0,
        ),
    ])
}

/// #5004: the `try` may sit behind a value head or a `select(true)` stage (`null | (try (FE) catch 7)`), whose pipe
/// hands it on copied and carries the register on its frame: the handler is still seeded with the register the step
/// entered its UPDATE with, and its statement is read back. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_try_handler_behind_a_value_head_5004() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":[1]}"#,
            r#"del(. as $x | foreach (.a) as $i (.; not | (try (foreach ($x as {a:$a} | .) as $y (0; .a?; error("x"))) catch $x); $i))"#,
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r#"[del(. as $x | foreach (.) as $i (.; not | (try (foreach (try ($x as {a:$a} | .)) as $y (.; .a?; (., error("x")))) catch 7); $i | .a))?]"#,
            "[{}]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":1}}"#,
            r"(. as $x | foreach (.a,.) as $i (.; (try (foreach (try ($x as {b:$a} | .)) as $y (.; .; error({}))) catch 7) | .; $i | .a)) = 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach ($x) as $i (.; null | (try (foreach (try ($x as {a:$a} | .)) as $y (.; .a?; error(.))) catch $__loc__); $x)) catch "C""#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":true}}"#,
            r"(. as $x | foreach (.a) as $i (.; not | (try (foreach ($x as {a:$a} | .) as $y (null; empty; error({}))) catch (.)); $i)) = 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":{"a":null}}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (try (foreach (try ($x as {b:$a} | .)) as $y (.; $y; error(.))) catch true) | .; $i))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":2}}"#,
            r"(. as $x | foreach (.) as $i (0; null | (try (foreach (try ($x as {b:$a} | .)) as $y (null; $y; error({}))) catch true); $i | .a)) = 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":2}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; select(true) | (try (foreach (try ($x as {b:$a} | .)) as $y (null; $y; error(.))) catch length); $i | .a)) catch "C""#,
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"b":null}"#,
            r"[del(. as $x | foreach (.) as $i (.; null | (try (foreach (try ($x as {a:$a} | .)) as $y (.; .a?; error(null))) catch (empty, 7)); $i | .a))?]",
            "[{\"b\":null}]\n",
            "",
            0,
        ),
    ])
}

/// #5004: where the resolver does not model whether the nested `foreach` raises at all, its handler is not seeded and
/// the refusal stays, with the wording it had (jq answers these). A two-entry pattern (`{a:$a,b:$b}`: jq's second step
/// is emptied by the source's own `try`, so jq's handler never runs where this one would) over a source that does not
/// move the register (`(1,2)`; over one that does, #5012 reads it, `tests/jq_cli/i5012.rs`), an outer `?//` chain (whose
/// first alternative jq retries on the body's raise), a nested `foreach` whose INIT navigates (an earlier step's raise
/// the resolver does not see), and a source that is a comma of caught destructures (jq's `try`s empty each item):
/// seeding the handler there answered or wrote what jq does not (`= 9` wrote `{"a":9}`, `del` wrote `{"a":{}}`).
/// Loud, not wrong.
///
/// #5223: the path-mode interpreter carries jq's register, so the rows the shipped resolver
/// refused here now hold jq 1.7.1's answers.
#[test]
fn test_foreach_update_try_handler_where_the_nested_foreach_is_not_modelled_stays_refused_5004(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (r#"{"a":false}"#, "(. as $x | foreach (1,2) as $i (0; (try (foreach (try ($x as {a:$a,b:$b} | .)) as $y (.; .; error({}))) catch null)?; $x)) = 9", "{\"a\":false}\n", "", 0),
        (r#"{"a":1}"#, "path(. as $x | (foreach ($x) as $i ?// [$i] (0; try (foreach (try ($x as {a:$a,b:$b} | .)) as $y (.; .; error)) catch 7; $i) | .))", "", "", 0),
        (r#"{"a":{"a":true}}"#, "(. as $x | foreach (.,.a) as $i (.; (try (foreach ($x as {a:$a} | .) as $y (.a?; empty; error($x))) catch null)?; $i)) = 9", "", r#"Cannot index number with string "a""#, 5),
        (r#"{"a":{"a":null}}"#, "[path(. as $x | foreach (.a) as {a:$q} ?// $i (.; try (foreach ($x as {a:$a} | .) as $y (.; $y; $y)) catch null; $i))]", "[[\"a\",\"a\"]]\n", "", 0),
        (r#"{"a":{"a":null}}"#, "del(. as $x | foreach (.a) as $i (.; null | (try (foreach (try ($x as {a:$a} | .), try ($x as {a:$a} | .)) as $y (null; .; error($x))) catch null); $i | .a?))", "{\"a\":{\"a\":null}}\n", "", 0),
        (r#"{"a":{"a":null}}"#, "(. as $x | foreach (.a) as $i (.; null | (try (foreach (try ($x as {a:$a} | .), try ($x as {a:$a} | .)) as $y (null; .; error($x))) catch null); $i | .a?)) |= 9", "{\"a\":{\"a\":null}}\n", "", 0),
        (r#"{"a":{"a":null}}"#, "[path(. as $x | foreach (.a) as $i (.; null | (try (foreach (try ($x as {a:$a} | .), try ($x as {a:$a} | .)) as $y (null; .; error($x))) catch null); $i | .a?))]", "[]\n", "", 0),
    ])?;
    Ok(())
}

/// #5004: a step that destructures (`as {a:$i}`) is not read: its walk moves the register in ways the step does not
/// report, and #4978's refusal of a container landing behind a handler that emits stands there. Reading it answered
/// `{"a":[null]}` for jq's error from `|= 9` and changed two messages. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_try_handler_behind_a_destructuring_step_is_not_read_5004() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":[1]}"#,
            r#"(. as $x | (foreach ((.,.)) as {a:$i} (null; try (foreach ($x as {a:$a} | .) as $y (.; .; null)) catch error("y"); $i) | .)) = 9"#,
            "",
            "y",
            5,
        ),
        (
            r#"{"a":[null]}"#,
            r"(. as $x | foreach ((.,.)) as {a:$i} (null; try (foreach ($x as {a:$a} | .) as $y (.; empty; $y)) catch (., 1); .a?)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"a\" of \"Invalid path expression n...",
            5,
        ),
        (
            r#"{"a":[null]}"#,
            r"[path(. as $x | foreach ($x) as {a:$i} (null; try (foreach ($x as {a:$a} | .) as $y (null; .; null)) catch (.a?); 7))]",
            "",
            "Invalid path expression near attempt to access element \"a\" of \"Invalid path expression n...",
            5,
        ),
    ])
}

/// #5004: a `break` to a label outside the `foreach` is caught by the `try` too (its handler gets jq's label object),
/// and jq runs that handler from the same restored register: the `$i` EXTRACT is the root, `[]`. Rows captured from
/// jq 1.7.1, both routes.
#[test]
fn test_foreach_update_try_handler_of_a_caught_break_sits_at_the_try_entry_5004() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"[label $out | path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch (.); $i))]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[label $out | path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch (.); $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"label $out | del(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch (.); $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[label $out | path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch 7; $i))]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[label $out | path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch 7; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"label $out | del(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch 7; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[label $out | path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch $i; $i))]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[label $out | path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch $i; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"label $out | del(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch $i; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[label $out | path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch (.); $i))]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[label $out | path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch (.); $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"label $out | del(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch (.); $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[label $out | path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch 7; $i))]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[label $out | path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch 7; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"label $out | del(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch 7; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[label $out | path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch $i; $i))]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[label $out | path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch $i; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"label $out | del(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; break $out)) catch $i; $i))",
            "null\n",
            "",
            0,
        ),
    ])
}

/// #5004: a handler inside the nested `foreach`'s INIT, UPDATE or EXTRACT (`try (.a) catch 7`) emits into that
/// `foreach`'s own state, not at the outer `try`'s entry, so it does not keep #4978's refusal of a container landing
/// behind a handler that emits: only a handler around the `foreach` does, and that one is placed. It was refused where
/// jq answers. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_try_handler_beside_a_handler_inside_the_nested_foreach_5004() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r#"path(. as $x | foreach (.a,.) as $i (null; try (try (foreach (try ($x as {a:$a} | .)) as $y (null; try (.a) catch 7; error("x"))) catch select(false)) catch (.); $i))"#,
            "",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r#"(. as $x | foreach (.a,.) as $i (null; try (try (foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; error("x"))) catch select(false)) catch $i; $i)) |= 9"#,
            "{\"a\":[null]}\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r#"path(. as $x | foreach (.a,.) as $i (.; try (try (foreach (try ($x as {a:$a} | .)) as $y (null; try (.a) catch 7; error("x"))) catch select(false)) catch $i; $i))"#,
            "",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"path(. as $x | foreach (.a,.) as $i (null; try (try (foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)) catch select(false)) catch (.); $i))",
            "",
            "Invalid path expression with result {\"a\":[null]}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"[path(. as $x | foreach (.a,.) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (null; try (.a) catch 7; error("x"))) catch (.); $i))?]"#,
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"path(. as $x | foreach (.a,.) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (null; try (.a) catch 7; error("x"))) catch (.); $i))"#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"(. as $x | foreach (.a,.) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (null; try (.a) catch 7; error("x"))) catch (.); $i)) |= 9"#,
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"del(. as $x | foreach (.a,.) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (null; try (.a) catch 7; error("x"))) catch (.); $i))"#,
            "null\n",
            "",
            0,
        ),
    ])
}

/// #5004: the handler is placed on the run's last step, which a source whose shape bounds its output count names
/// before the next item is asked for: `(.a,$x)`, `((.a,.a),.)` and `(.a?,.)` emit at most two, three and two items,
/// and their step on the root is the last one (`source_output_bound`). Rows captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_try_handler_on_the_last_step_of_a_bounded_source_5004() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r#"path(. as $x | foreach (.a,$x) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch 7; $i))"#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"[path(. as $x | foreach (.a,$x) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch 7; $i))?]"#,
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"del(. as $x | foreach (.a,$x) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch 7; $i))"#,
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"(. as $x | foreach (.a,$x) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch 7; $i)) |= 9"#,
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"path(. as $x | foreach ((.a,.a),.) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch (.); $i))"#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"[path(. as $x | foreach ((.a,.a),.) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch (.); $i))?]"#,
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"del(. as $x | foreach ((.a,.a),.) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch (.); $i))"#,
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"(. as $x | foreach ((.a,.a),.) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch (.); $i)) |= 9"#,
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"[path(. as $x | foreach (.a?,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch (.); $i))]"#,
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"del(. as $x | foreach (.a?,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; error("x"))) catch (.); $i))"#,
            "null\n",
            "",
            0,
        ),
    ])
}
