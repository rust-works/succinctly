//! #4800: a `foreach` UPDATE that binds `.` with a single pattern, under a `first`/`limit(1; ..)`/`nth(0; ..)` whose
//! comma holds a non-bind sibling behind the bind.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #4800: `first(B, 7)` takes the bind's output and stops, so the sibling behind it is never run or stored and
/// the state is the register's own node as `B` alone leaves it: jq path-checks nothing at the UPDATE and the
/// answer is the EXTRACT's (`$i` is `result 1`, `empty` prints nothing, `.a` is the index error on the
/// register's node). Before, the sibling made the UPDATE "not a bind-only shape" and it was refused naming its
/// own state (`result {"a":null}`), `empty` raised where jq prints nothing, and `[path(..)?]` answered `[]`
/// where jq answers `[["a"]]`. Covers `first`, `limit(1; ..)`, `limit(0.5; ..)` (takes `ceil(0.5)`), `nth(0; ..)`,
/// a nested comma, and the wrappers around them. Every row captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_single_pattern_bind_led_first_over_a_non_bind_sibling_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; first((. as {a:$q} | .), 7); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; first((. as {a:$q} | .), 7); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; first((. as {a:$q} | .), 7); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; first((. as {a:$q} | .), 7); empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(1; (. as {a:$q} | .), 7); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; limit(1; (. as {a:$q} | .), 7); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(1; (. as {a:$q} | .), 7); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; limit(1; (. as {a:$q} | .), 7); empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(0.5; (. as {a:$q} | .), .a); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; limit(0.5; (. as {a:$q} | .), .a); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(0.5; (. as {a:$q} | .), .a); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; limit(0.5; (. as {a:$q} | .), .a); empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; nth(0; (. as {a:$q} | .), 7); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; nth(0; (. as {a:$q} | .), 7); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; nth(0; (. as {a:$q} | .), 7); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; nth(0; (. as {a:$q} | .), 7); empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; first(((. as {a:$q} | .), 7), 8); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; first(((. as {a:$q} | .), 7), 8); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; first(((. as {a:$q} | .), 7), 8); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; first(((. as {a:$q} | .), 7), 8); empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; if . then first((. as {a:$q} | .), 7) else . end; $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; if . then first((. as {a:$q} | .), 7) else . end; $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; if . then first((. as {a:$q} | .), 7) else . end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; if . then first((. as {a:$q} | .), 7) else . end; empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; try first((. as {a:$q} | .), .a); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; try first((. as {a:$q} | .), .a); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; try first((. as {a:$q} | .), .a); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; try first((. as {a:$q} | .), .a); empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; first((. as {a:$q} | .), 7) | .; $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; first((. as {a:$q} | .), 7) | .; $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; first((. as {a:$q} | .), 7) | .; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; first((. as {a:$q} | .), 7) | .; empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; first((. as {a:$q} | .), 7); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(foreach (1,2) as $i (.; first((. as {a:$q} | .), 7); empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; limit(1; (. as {a:$q} | .), 7); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(foreach (1,2) as $i (.; limit(1; (. as {a:$q} | .), 7); empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; limit(0.5; (. as {a:$q} | .), .a); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(foreach (1,2) as $i (.; limit(0.5; (. as {a:$q} | .), .a); empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; nth(0; (. as {a:$q} | .), 7); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(foreach (1,2) as $i (.; nth(0; (. as {a:$q} | .), 7); empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; first(((. as {a:$q} | .), 7), 8); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(foreach (1,2) as $i (.; first(((. as {a:$q} | .), 7), 8); empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; first((. as {a:$q} | .), 7); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(foreach (1,2) as $i (.; first((. as {a:$q} | .), 7); empty))",
            "",
            "",
            0,
        ),
        (
            r"null",
            r"path(foreach (1,2) as $i (.; first((. as {a:$q} | .), 7); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(foreach (1,2) as $i (.; nth(0; (. as {a:$q} | .), 7); empty))",
            "",
            "",
            0,
        ),
        (
            r"null",
            r"path(foreach (1,2) as $i (.; nth(0; (. as {a:$q} | .), 7); empty))",
            "",
            "",
            0,
        ),
    ])
}

/// #4800: the controls of [`test_foreach_update_single_pattern_bind_led_first_over_a_non_bind_sibling_4800`]
/// keep their refusal, since the sibling is stored or supplies the first output: `limit(2; B, 7)` stores `7`
/// as the state, a sibling or an item that can emit nothing ahead of the bind (`first(7, B)`,
/// `first(empty, B, 7)`) leaves the first output to something else, `nth(1; ..)` takes the second, a count
/// that depends on the state is evaluated first, an `if` that navigates decides by the state, and a bind
/// whose pattern computes a key (`{(empty): $q}`) emits nothing, so the sibling supplies the first output and
/// stores a node of its own. Only the
/// refusal's kind is pinned here, as the sibling tests do.
#[test]
fn test_foreach_update_first_over_a_non_bind_sibling_controls_stay_refused_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(2; (. as {a:$q} | .), 7); $i))",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; first(7, (. as {a:$q} | .)); $i))",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; first(empty, (. as {a:$q} | .), 7); $i))",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; first(select(false), (. as {a:$q} | .), 7); $i))",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; nth(1; (. as {a:$q} | .), 7); $i))",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(.a; (. as {a:$q} | .), 7); $i))",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; first(if .a then (. as {a:$q} | .) else . end, 7); $i))",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(foreach (1,2) as $i (.; first((. as {(empty):$q} | .), .a); .))",
            "[\"a\"]\n",
            "Invalid path expression",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(foreach (1,2) as $i (.; limit(1; (. as {(empty):$q} | .), .a); .))",
            "[\"a\"]\n",
            "Invalid path expression",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(foreach (1,2) as $i (.; nth(0; (. as {(empty):$q} | .), .a); .))",
            "[\"a\"]\n",
            "Invalid path expression",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(foreach (1,2) as $i (.; first((. as {(select(false)):$q} | .), .a); .))",
            "[\"a\"]\n",
            "Invalid path expression",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"[path(foreach (1,2) as $i (.; first((. as {(empty):$q} | .), .a); .))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
    ])
}
