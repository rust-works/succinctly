//! #4800: a `foreach` UPDATE that is an `if` whose `then` branch holds the caught foreign-`$x` `foreach` and whose
//! `else` branch is a literal (`else null`). `FE` is `foreach (try ($x as {a:$a} | .)) as $y (.; .; .)`.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #4800: a step whose condition is falsy runs the `else` arm alone, and the arm is resolved as a UPDATE of its own
/// (`taken_if_branch`). The `if` was still resolved whole, so `cannot_move_register` read the `foreach` in the arm that
/// did not run and withheld the tracking of the `null` the literal arm stores: with a `.a` EXTRACT that `null` is
/// jq's register by kind (`jv_identical`), and `.a` raised `Invalid path expression near attempt to access element
/// "a" of null` where jq answers `["a","a"]`. Rows captured from jq 1.7.1 (stdout, first stderr line, exit code),
/// both routes; the first eight fail on `main`, and resolving the `if` whole again fails the test.
#[test]
fn test_foreach_update_if_with_the_caught_foreach_in_then_and_a_literal_else_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end; .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end; .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach .a as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end; .a))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach .a as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end; .a)) catch "c""#,
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach .a as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end; .a))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach .a as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end; .a)) |= 9",
            "{\"a\":{\"a\":9}}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; if .a then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) else null end; .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) elif .b then 7 else null end; .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach .a as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; if .b then 1 elif .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end; .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
    ])
}

/// #4800: the arm is resolved alone behind a wrapper too (`resolved_if_arm`): a comma whose other items emit nothing and
/// a `label` scope without a `break` ahead of the item do not change what the `if` emits, and a handler-less `try`/`?`
/// around a state that took a literal `else` arm drops a handler that could catch nothing. jq answers
/// `[["a","a"]]` for the `.a` EXTRACT rows, and those fail on the base (`[]`; an `elif` chain and a `(.a,.)` source
/// included). The `$i` EXTRACT rows, the `| .` row and the `{"a":true}` row already matched on the base: they pin that
/// the arm-only resolve does not move them. Resolving the wrapped `if` whole again fails the test.
#[test]
fn test_foreach_update_wrapped_if_with_a_literal_else_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end)?; .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end, empty); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; label $l | if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end; .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end) | .; .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; label $l | (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end, break $l); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) elif .b then 7 else null end); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end); $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end)?; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end, empty); $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; label $l | if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach .a as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end); .a))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #4800: a `try` with a handler and a comma beside an item that emits are read too, where the state reaches a literal
/// `else` arm of an `if` whose other arms move the register only through a fold that heads them (`literal_taken_arm`):
/// the literal cannot raise, so the handler never runs, and the step takes it as the UPDATE it ran
/// (`handler_literal_arm`, `resolved_if_arm`); the comma's items are resolved one at a time, the `if` as its arm
/// (`if_arm_comma_items`). Every row fails on the base: the `.a` EXTRACT rows raised `Invalid path expression near
/// attempt to access element "a" of null`, the `$i` rows answered `[]`, and `|= 9` raised. Rows captured from jq 1.7.1;
/// disabling either new path fails the test.
#[test]
fn test_foreach_update_if_with_a_literal_else_behind_a_handler_or_beside_an_emitting_item_4800(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end) catch 1; .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end) catch 1; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach .a as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end) catch 1; .a))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach .a as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end) catch 1; $i))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach .a as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end) catch 1; .a)) |= 9",
            "{\"a\":{\"a\":9}}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end) catch ($x | .a); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) elif .b then 7 else null end) catch 1; .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end, 7); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end, 7); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach .a as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end, 7); .a))",
            "[\"a\",\"a\"]\n",
            "Invalid path expression near attempt to access element \"a\" of 7",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach .a as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end, 7); $i)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; (7, if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end, $i); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; (((if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end)?), 7); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else 1 end) catch 1; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"[path(. as $x | foreach .a as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else "s" end) catch 1; $i))?]"#,
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"(. as $x | foreach .a as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end) catch 1; $i)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end) catch 1; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.,.a) as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end) catch 1; $i))?]",
            "[[],[\"a\"]]\n",
            "",
            0,
        ),
    ])
}

/// #4800: what the handler and comma reads leave alone, each matching jq on the base and pinned to keep matching. A
/// `then` that ran (the #5082 reading) or a computed `else` (`[.a]`) behind a handler or beside a comma sibling, an
/// `if` with no fold (`if .a then .b else 3 end`, which the base already reads whole), and a truthy state that runs
/// the `foreach`, keep the whole-`if` resolve: narrowing them lost each of these rows in a differential grid.
#[test]
fn test_foreach_update_if_behind_a_handler_or_comma_keeps_the_whole_resolve_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r"{}",
            r"(. as $x | foreach . as $i (.; try (if .a then 7 else true end) catch ($x | .a); $i)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r"null",
            r"del(. as $x | foreach (.,.a) as $i (.; (if .a then 7 else [.a] end, $i); $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach .a as $i (.; try (if . then 7 else [.a] end) catch 1; $i)) catch "c""#,
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach .a as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end) catch 1; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach .a as $i (.; try (if .a then .b else 3 end) catch 1; $i))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"(foreach .a as $i (.; try (if .a then .b else 3 end) catch 1; $i)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(foreach 1 as $i (.; try (if .a then .b else null end) catch 1; .a)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; try (if .a then .b else null end) catch 1; .a))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #4800: the mirror shape, a literal `then` the state ran beside the fold in the `else` (`if .a then 1 else FE end`),
/// is read behind a `try ... catch` and beside an emitting comma item as the literal `else` arm is
/// (`literal_taken_arm`), on the `foreach`'s last step: the stored literal feeds no later UPDATE, which would run the
/// fold arm on it. Every row fails on the base: the `$i` rows answered `[]` and `path`, `del` and `|= 9` raised
/// `Invalid path expression with result {"a":null}`, the `.a` EXTRACT row raised `Invalid path expression near attempt
/// to access element "a" of null`. Rows captured from jq 1.7.1; dropping the `then` reading fails the test.
#[test]
fn test_foreach_update_if_with_a_literal_then_beside_a_fold_in_the_else_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach .a as $i (.; try (if .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach .a as $i (.; try (if .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach .a as $i (.; try (if .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach .a as $i (.; try (if .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach .a as $i (.; (if .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end, 7); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach .a as $i (.; (7, if .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach .a as $i (.; (if .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end, 7); $i))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach .a as $i (.; (if .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end, 7); $i)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach .a as $i (.; try (if .a then null else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"[path(. as $x | foreach .a as $i (.; try (if .a then "s" else ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) end) catch 1; $i))?]"#,
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"[path(. as $x | foreach .a as $i (.; try (if . then "s" else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i))?]"#,
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach .a as {a:$k} ?// $k (.; (if .a then null else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end, 7); $k))?]",
            "[[\"a\",\"a\"],[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach .a as {a:$k} ?// $k (.; try (if .a then null else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $k))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
    ])
}

/// #4800: a literal `then` past a falsy `elif` condition is not read ([`taken_then_branch`]: an earlier step may run the
/// later arm), so the chain keeps the whole-`if` resolve it had. jq answers `[[]]`, `[]`, `[[]]` and `[[],[]]`; the
/// answers are pinned as they stand (a fix flips them; #5208 flipped the second row, a step whose state is the register, to jq's `[]`).
/// Reading it changed the last row, `(7, if .a then null elif .b
/// then 2 else FE end)` on `{"b":true}`, from one wrong answer (`[]`) to another (`[[]]`).
///
/// #5223: the path-mode interpreter carries jq's register, so the rows the shipped resolver
/// refused here now hold jq 1.7.1's answers.
#[cfg(not(feature = "unshared-containers"))]
#[test]
fn test_foreach_update_if_with_a_literal_then_past_a_falsy_elif_stays_differing_4800() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach . as $i (.; try (if .c then 7 elif .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach . as $i (.; try (if .c then 7 elif .b then null else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"b":true,"a":1}"#,
            r"[path(. as $x | foreach . as $i (.; try (if .c then 7 elif .b then null else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"b":true}"#,
            r"[path(. as $x | foreach . as $i (.; (7, if .a then null elif .b then 2 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end); $i))?]",
            "[[],[]]\n",
            "",
            0,
        ),
    ])
}

/// #4800: the shapes still resolved whole, pinned as they stand (a fix flips them). jq answers, row by row, `[["a"]]`,
/// `[["a"],["a"]]`, `[["a"]]`, `{"a":9}`, `[["a","a"],["a","a"]]`, `[["a"]]`, `[["a"],[]]` and `[["a"]]`: a comma sibling that may
/// move the register (`.a`), a `try ... catch` that is itself a comma item, a computed `else`, an arm that navigates
/// ahead of its fold (`.b | FE`), and a literal `then` that ran beside a fold in the `else` over a source of two
/// elements (the mirror shape off its last step). The `.b | FE` row stays a refusal on purpose: the step that runs it
/// names `["b"]` where jq's register moved to `["a"]`, and narrowing the literal arm's step turned the refusal that
/// masked it into a write of `.b`. The literal `then` is read only on the `foreach`'s last step, or on an earlier one when every later step stores a
/// literal too (#5113, the seventh row, now jq's `[["a"],[]]`): read on the first of
/// two, the `.a` EXTRACT row's refusal became a truncated `[["a","a"]]`, because the second step runs the fold arm on
/// the stored literal and refuses where a `?` swallows it. The same holds for a `?//` retry that runs UPDATE again on
/// the stored literal (the last row; the base answers `[]` there too). The last step of an earlier INIT fork is read
/// since #5113 (`tests/jq_cli/i5113.rs`): the next fork stores INIT's own output. So is a `try ... catch` that is a comma
/// item beside a literal sibling (the second row, now jq's `[["a"],["a"]]`).
///
/// #5223: the path-mode interpreter carries jq's register, so the rows the shipped resolver
/// refused here now hold jq 1.7.1's answers.
#[cfg(not(feature = "unshared-containers"))]
#[test]
fn test_foreach_update_wrapped_if_with_a_literal_else_stays_differing_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end, .a); $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; (try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end) catch 1, 7); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach .a as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else .a end) catch 1; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.,.a) as $i (.; (if . then (.b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else false end, if . then (.b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else false end); null)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"b":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (if .b then null else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; .a))?]",
            "[[\"a\",\"a\"],[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"b":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (if .b then null else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (if .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i))?]",
            "[[\"a\"],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"[path(. as $x | foreach .a as {a:$k} ?// $k (.; (if .a then null else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end, 7); (if $k == null then error("x") else $k end)))?]"#,
            "[[\"a\"]]\n",
            "",
            0,
        ),
    ])
}

/// #4800: a condition the walk cannot decide on the state (`.a?`, a comma, `range(2)`, a bind) names no arm
/// ([`taken_if_branch`] answers the `if` itself), so the wrapped `if` is resolved whole and the pipe's `if` stage is
/// read by `alternate_update_shape`, which is the one route that still reaches the `return None` of its `Pipe` arm
/// (the literal-arm rows above are resolved as the arm and never get there). Where jq answers `[[]]` or `[[],[]]`
/// these answer `[]`; the answers are pinned as they stand (a fix flips them), and the rows keep the arm covered.
#[cfg(not(feature = "unshared-containers"))]
#[test]
fn test_foreach_update_wrapped_if_with_an_undecidable_condition_stays_differing_4800() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":true,"b":2}"#,
            r"[path(foreach . as $i (.; if .a? then 7 else .b end | .; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":true,"b":2}"#,
            r"[path(foreach . as $i (.; if .a as $x | $x then 7 else .b end | .; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":true,"b":2}"#,
            r"[path(foreach . as $i (.; (if (.a,.b) then 7 else .b end) | select(true); $i))?]",
            "[[],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":false,"b":2}"#,
            r"[path(foreach . as $i (.; if range(2) then 7 else .b end | .; $i))?]",
            "[[],[]]\n",
            "",
            0,
        ),
    ])
}
