//! #4800: a `foreach` UPDATE that binds `.` with a single pattern, under a `first`/`limit`/`nth(0; ..)`, an `if`
//! and a `select` of a truthy literal.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #4800: jq stores every output of a `foreach` UPDATE as the state and path-checks none of them, so a
/// single-pattern bind of `.` that hands `.` back leaves the register's own node as the state wherever a
/// `first`/`limit`/`nth(0; ..)` of it, an `if` whose condition does not navigate and whose branches are such
/// binds (or `.`/`empty`), or a `select` of a truthy literal (`select(1)`, `select("x")`) wraps it, and the
/// answer is the EXTRACT's (`$i` is `result 1`, `empty` prints nothing, `.a` is the register's own `["a"]`).
/// Before, each wrapper made the UPDATE "not a bind-only shape" and it was refused naming its own state
/// (`result {"a":null}`), `empty` EXTRACT raised where jq prints nothing and `[path(..; .a)?]` answered `[]`.
/// Every row captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_single_pattern_bind_under_first_limit_if_and_truthy_select_4800(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; first(. as {a:$q} | .); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; first(. as {a:$q} | .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; first(. as {a:$q} | .); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; first(. as {a:$q} | .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(2; (. as {a:$q} | .), (. as {a:$q} | .)); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(2; (. as {a:$q} | .), (. as {a:$q} | .)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; limit(2; (. as {a:$q} | .), (. as {a:$q} | .)); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; limit(2; (. as {a:$q} | .), (. as {a:$q} | .)); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(-1; . as {a:$q} | .); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(-1; . as {a:$q} | .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; limit(-1; . as {a:$q} | .); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; limit(-1; . as {a:$q} | .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(1.5; . as {a:$q} | .); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(1.5; . as {a:$q} | .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; limit(1.5; . as {a:$q} | .); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; limit(1.5; . as {a:$q} | .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; nth(0; (. as {a:$q} | .), (. as {a:$q} | .)); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; nth(0; (. as {a:$q} | .), (. as {a:$q} | .)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; nth(0; (. as {a:$q} | .), (. as {a:$q} | .)); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; nth(0; (. as {a:$q} | .), (. as {a:$q} | .)); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; if true then (. as {a:$q} | .) else . end; $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; if true then (. as {a:$q} | .) else . end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; if true then (. as {a:$q} | .) else . end; empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; if true then (. as {a:$q} | .) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; if . then (. as {a:$q} | .) else (. as {a:$q} | .) end; $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; if . then (. as {a:$q} | .) else (. as {a:$q} | .) end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; if . then (. as {a:$q} | .) else (. as {a:$q} | .) end; empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; if . then (. as {a:$q} | .) else (. as {a:$q} | .) end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; if . then (. as {a:$q} | .) else empty end; $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; if . then (. as {a:$q} | .) else empty end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; if . then (. as {a:$q} | .) else empty end; empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; if . then (. as {a:$q} | .) else empty end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; if . then . else (. as {a:$q} | .) end; $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; if . then . else (. as {a:$q} | .) end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; if . then . else (. as {a:$q} | .) end; empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; if . then . else (. as {a:$q} | .) end; .a))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; first(if . then (. as {a:$q} | .) else . end); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; first(if . then (. as {a:$q} | .) else . end); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; first(if . then (. as {a:$q} | .) else . end); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; first(if . then (. as {a:$q} | .) else . end); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; select(1) | (. as {a:$q} | .); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; select(1) | (. as {a:$q} | .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; select(1) | (. as {a:$q} | .); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; select(1) | (. as {a:$q} | .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"path(foreach (1,2) as $i (.; select("x") | (. as {a:$q} | .); $i))"#,
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r#"path(foreach (1,2) as $i (.; select("x") | (. as {a:$q} | .); empty))"#,
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"del(foreach (1,2) as $i (.; select("x") | (. as {a:$q} | .); empty))"#,
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"[path(foreach (1,2) as $i (.; select("x") | (. as {a:$q} | .); .a))?]"#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .) | select(0); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .) | select(0); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; (. as {a:$q} | .) | select(0); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; (. as {a:$q} | .) | select(0); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; try first(. as {a:$q} | .); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; try first(. as {a:$q} | .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; try first(. as {a:$q} | .); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; try first(. as {a:$q} | .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; label $l | first(. as {a:$q} | .); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; label $l | first(. as {a:$q} | .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; label $l | first(. as {a:$q} | .); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; label $l | first(. as {a:$q} | .); .a))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #4800: the controls of [`test_foreach_update_single_pattern_bind_under_first_limit_if_and_truthy_select_4800`]
/// keep their refusal, since each stores something other than the register's node or decides by the state:
/// an `if` whose condition navigates (`.a`) or whose other branch stores a node of its own (`7`), `nth(1; ..)`
/// (the second output, a different one), a state-dependent count, `select(.)` and a `//` (a falsy register
/// would run the alternate). jq answers `result 1` for each (the EXTRACT's `$i`), so only the refusal's kind
/// is pinned here, as the sibling tests do.
#[test]
fn test_foreach_update_single_pattern_bind_under_wrappers_that_stay_refused_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; if .a then (. as {a:$q} | .) else . end; $i))",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; if . then (. as {a:$q} | .) else 7 end; $i))",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), (. as {a:$q} | .)); $i))",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(.a; . as {a:$q} | .); $i))",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; select(.) | (. as {a:$q} | .); $i))",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .) // 5; $i))",
            "",
            "Invalid path expression with result",
            5,
        ),
    ])
}
