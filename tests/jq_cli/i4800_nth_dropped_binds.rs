//! #4800: a `foreach` UPDATE `nth(k; ITEMS)` whose first `k` comma items each hand `.` on once (`.` or a
//! single-pattern bind of `.`), with a bind or a navigation as the delivered item.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #4800: `nth(k; ..)` over a comma drops the first `k` outputs and stores the next as the state. A dropped
/// single-pattern bind of `.` (`. as {a:$q} | .`) path-checks nothing, so `nth(1; B, .a)` stores `.a`'s node
/// and moves the register there exactly as `nth(1; ., .a)` does, where the bind's `.` was refused for the
/// register it was handed and a `?` turned the refusal into `[]` (`[path(foreach (1,2) as $i (.; nth(1; B,
/// .a); .a))?]` on `{"a":null}` is `[["a","a"]]` in jq). A delivered bind or `.` behind such items stores
/// the register's own node as a bare bind does, so an `empty` EXTRACT prints nothing and deletes nothing.
/// A dropped `.a` can raise by the state and keeps its refusal, below. Every row captured from jq 1.7.1,
/// both routes.
#[test]
fn test_foreach_update_nth_over_dropped_dot_binds_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), .a); .a))",
            "[\"a\",\"a\"]\n",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), .a); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), .a); .))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":3}}"#,
            r"path(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), .a); .a))",
            "[\"a\",\"a\"]\n",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":3}",
            5,
        ),
        (
            r#"{"a":{"a":3}}"#,
            r"[path(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), .a); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":3}}"#,
            r"[path(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), .a); .))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), .a); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; nth(2; (. as {a:$q} | .), (. as {a:$q} | .), .a); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; nth(1; ., .a); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), .); .a))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), .); .a))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), .); .a)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), (. as {a:$q} | .)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), (. as {a:$q} | .)); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), (. as {a:$q} | .)); empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), (. as {a:$q} | .)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"del(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), (. as {a:$q} | .)); empty))",
            "{\"a\":1}\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), (. as {a:$q} | .)); empty)) |= 9",
            "{\"a\":1}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; nth(1; ., (. as {a:$q} | .)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; nth(1; ., (. as {a:$q} | .)); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(foreach (1,2) as $i (.; nth(1; ., (. as {a:$q} | .)); empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(foreach (1,2) as $i (.; nth(1; ., (. as {a:$q} | .)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"del(foreach (1,2) as $i (.; nth(1; ., (. as {a:$q} | .)); empty))",
            "{\"a\":1}\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"(foreach (1,2) as $i (.; nth(1; ., (. as {a:$q} | .)); empty)) |= 9",
            "{\"a\":1}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; nth(2; (. as {a:$q} | .), ., (. as {a:$q} | .), 7); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; nth(2; (. as {a:$q} | .), ., (. as {a:$q} | .), 7); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(foreach (1,2) as $i (.; nth(2; (. as {a:$q} | .), ., (. as {a:$q} | .), 7); empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(foreach (1,2) as $i (.; nth(2; (. as {a:$q} | .), ., (. as {a:$q} | .), 7); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"del(foreach (1,2) as $i (.; nth(2; (. as {a:$q} | .), ., (. as {a:$q} | .), 7); empty))",
            "{\"a\":1}\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"(foreach (1,2) as $i (.; nth(2; (. as {a:$q} | .), ., (. as {a:$q} | .), 7); empty)) |= 9",
            "{\"a\":1}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), (. as {a:$q} | .)); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r"[1]",
            r"[path(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), .a); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r"[1]",
            r"path(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), .a); .a))",
            "",
            "Cannot index array with string \"a\"",
            5,
        ),
        // A dropped bind still runs its pattern step: an array pattern on an object raises, as in jq.
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; nth(1; (. as [$q] | .), .a); .a))",
            "",
            "Cannot index object with number",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; nth(1; (. as [$q] | .), .a); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; nth(1; (. as {(empty):$q} | .), .a); .a))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #4800: what stays refused. A dropped `.a` is a navigation that raises where the state is no object
/// (`[1]`), which the bind-and-`.` reading does not cover, so `nth(1; .a, B)` with an `empty` EXTRACT keeps
/// the refusal naming the state where jq prints nothing (jq 1.7.1: exit 0). Not a wrong accept.
///
/// #5223: the path-mode interpreter carries jq's register, so the rows the shipped resolver
/// refused here now hold jq 1.7.1's answers.
#[test]
fn test_foreach_update_nth_over_a_dropped_navigation_keeps_its_refusal_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[(
        r#"{"a":null}"#,
        r"path(foreach (1,2) as $i (.; nth(1; .a, (. as {a:$q} | .)); empty))",
        "",
        "",
        0,
    )])
}
