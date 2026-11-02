//! #4800: an `if . then (FE, empty) else . end` UPDATE around the caught foreign-`$x` `foreach`, in a `foreach` UPDATE
//! over a moving source: a comma `then` branch whose other items emit nothing is read as the bare `if` is.
//! `FE` is `foreach (try ($x as {a:$a} | .)) as $y (.; .; .)`.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #4800: `if . then (FE, empty) else . end` (`FE` followed by an `empty` sibling) was not read like the bare `if`: the UPDATE was refused naming the state, where jq answers
/// (`["a"]` for a `$i` EXTRACT, nothing for `empty`), and a `?` answered `[]` for jq's `[["a"]]`. Rows captured from jq
/// 1.7.1 (stdout, first stderr line, exit code), both routes, over `path`, `[path(..)?]`, `del` and `|=` with `$i`, `.`
/// and `empty` EXTRACTs on `{"a":null}` and `{"a":true}`.
#[test]
fn test_foreach_if_dot_comma_branch_comma_empty_after_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; $i))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; $i)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .)) |= 9",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; $i))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; $i)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .)) |= 9",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; empty))",
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; empty)) |= 9",
            "{\"a\":true}\n",
            "",
            0,
        ),
    ])
}

/// #4800: `if . then (empty, FE) else . end` (`empty` ahead of `FE`) was not read like the bare `if`: the UPDATE was refused naming the state, where jq answers
/// (`["a"]` for a `$i` EXTRACT, nothing for `empty`), and a `?` answered `[]` for jq's `[["a"]]`. Rows captured from jq
/// 1.7.1 (stdout, first stderr line, exit code), both routes, over `path`, `[path(..)?]`, `del` and `|=` with `$i`, `.`
/// and `empty` EXTRACTs on `{"a":null}` and `{"a":true}`.
#[test]
fn test_foreach_if_dot_comma_branch_comma_empty_before_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .)) |= 9",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .)) |= 9",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; empty))",
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; empty)) |= 9",
            "{\"a\":true}\n",
            "",
            0,
        ),
    ])
}

/// #4800: `if . then (FE, select(false)) else . end` (a `select(false)` sibling) was not read like the bare `if`: the UPDATE was refused naming the state, where jq answers
/// (`["a"]` for a `$i` EXTRACT, nothing for `empty`), and a `?` answered `[]` for jq's `[["a"]]`. Rows captured from jq
/// 1.7.1 (stdout, first stderr line, exit code), both routes, over `path`, `[path(..)?]`, `del` and `|=` with `$i`, `.`
/// and `empty` EXTRACTs on `{"a":null}` and `{"a":true}`.
#[test]
fn test_foreach_if_dot_comma_branch_comma_select_false_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; $i))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; $i)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; .)) |= 9",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; $i))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; $i)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; .)) |= 9",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; empty))",
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)) else . end; empty)) |= 9",
            "{\"a\":true}\n",
            "",
            0,
        ),
    ])
}

/// #4800: `if . then ((FE), first(empty)) else . end` (a parenthesised `FE` beside `first(empty)`) was not read like the bare `if`: the UPDATE was refused naming the state, where jq answers
/// (`["a"]` for a `$i` EXTRACT, nothing for `empty`), and a `?` answered `[]` for jq's `[["a"]]`. Rows captured from jq
/// 1.7.1 (stdout, first stderr line, exit code), both routes, over `path`, `[path(..)?]`, `del` and `|=` with `$i`, `.`
/// and `empty` EXTRACTs on `{"a":null}` and `{"a":true}`.
#[test]
fn test_foreach_if_dot_comma_branch_comma_parens_first_empty_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; $i))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; $i)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; .)) |= 9",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; $i))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; $i)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; .)) |= 9",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; empty))",
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), first(empty)) else . end; empty)) |= 9",
            "{\"a\":true}\n",
            "",
            0,
        ),
    ])
}

/// #4800: An `if` on a condition other than `.` around a comma branch. These rows once matched only because the comma
/// was refused: read like the bare form, the `.a` EXTRACT's index error on the `null` state of the second step escaped a
/// `?` and a `try` where jq answers `[]`, until `taken_if_branch` read that step's `else` branch. Rows captured from jq
/// 1.7.1, both routes.
#[test]
fn test_foreach_if_comma_branch_if_on_a_condition_other_than_dot_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .a)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .a?))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .a?)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
    ])
}

/// #4800: A stage after the `if` around a comma branch, for the same reason (the `if` is now found at the head of the
/// pipe). Rows captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_if_comma_branch_stage_after_the_if_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .; .a)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .; .a?))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .; .a?)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .; .a)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .; .a?))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .; .a?)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
    ])
}

/// #4800: a comma branch behind a nested `if` on a condition other than `.`, and one under a `try`, for the same reason
/// (the `if` is now found behind the `try`). Rows captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_if_comma_branch_nested_if_and_try_wrapper_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if .a then (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if .a then (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) else . end; .a)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; try (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end); .a)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end); .a?))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; try (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end); .a?)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; try (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end); .a)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end); .a?))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; try (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end); .a?)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
    ])
}
