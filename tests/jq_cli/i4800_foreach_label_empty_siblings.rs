//! #4800: a `label` around the caught foreign-`$x` `foreach` of a `foreach` UPDATE over a moving source.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #4800: `label $l | FE` stores exactly the outputs the bare `foreach` does. It was refused naming the state where jq
/// answers `["a"]` (`[path(..)?]` is `[["a"]]`, here `[]`). Rows captured from jq 1.7.1 (stdout, first stderr line, exit
/// code), both routes, over `path`, `[path(..)?]`, `del` and `|=` with `.`, `$i`, `empty` and `.a` EXTRACTs.
#[test]
fn test_caught_foreign_marker_destructure_foreach_under_a_label_bare_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a)) |= 9",
            "{\"a\":{\"a\":9}}\n",
            "",
            0,
        ),
    ])
}

/// #4800: `label $l | (FE, empty)` stores exactly the outputs the bare `foreach` does. It was refused naming the state where jq
/// answers `["a"]` (`[path(..)?]` is `[["a"]]`, here `[]`). Rows captured from jq 1.7.1 (stdout, first stderr line, exit
/// code), both routes, over `path`, `[path(..)?]`, `del` and `|=` with `.`, `$i`, `empty` and `.a` EXTRACTs.
#[test]
fn test_caught_foreign_marker_destructure_foreach_under_a_label_comma_empty_after_4800(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty); .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty); .))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty); .))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty); .)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty); $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty); $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty); $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty); empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty); empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty); .a))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty); .a))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty); .a)) |= 9",
            "{\"a\":{\"a\":9}}\n",
            "",
            0,
        ),
    ])
}

/// #4800: `label $l | (empty, FE)` stores exactly the outputs the bare `foreach` does. It was refused naming the state where jq
/// answers `["a"]` (`[path(..)?]` is `[["a"]]`, here `[]`). Rows captured from jq 1.7.1 (stdout, first stderr line, exit
/// code), both routes, over `path`, `[path(..)?]`, `del` and `|=` with `.`, `$i`, `empty` and `.a` EXTRACTs.
#[test]
fn test_caught_foreign_marker_destructure_foreach_under_a_label_comma_empty_before_4800(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (empty, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a)) |= 9",
            "{\"a\":{\"a\":9}}\n",
            "",
            0,
        ),
    ])
}

/// #4800: `label $l | (FE, select(false))` stores exactly the outputs the bare `foreach` does. It was refused naming the state where jq
/// answers `["a"]` (`[path(..)?]` is `[["a"]]`, here `[]`). Rows captured from jq 1.7.1 (stdout, first stderr line, exit
/// code), both routes, over `path`, `[path(..)?]`, `del` and `|=` with `.`, `$i`, `empty` and `.a` EXTRACTs.
#[test]
fn test_caught_foreign_marker_destructure_foreach_under_a_label_comma_select_false_4800(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)); .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)); .))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)); .))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)); .)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)); $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)); $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)); $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)); empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)); empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)); .a))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)); .a))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), select(false)); .a)) |= 9",
            "{\"a\":{\"a\":9}}\n",
            "",
            0,
        ),
    ])
}

/// #4800: `label $l | (first(empty), FE)` stores exactly the outputs the bare `foreach` does. It was refused naming the state where jq
/// answers `["a"]` (`[path(..)?]` is `[["a"]]`, here `[]`). Rows captured from jq 1.7.1 (stdout, first stderr line, exit
/// code), both routes, over `path`, `[path(..)?]`, `del` and `|=` with `.`, `$i`, `empty` and `.a` EXTRACTs.
#[test]
fn test_caught_foreign_marker_destructure_foreach_under_a_label_comma_first_empty_4800(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (first(empty), foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (first(empty), foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (first(empty), foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (first(empty), foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (first(empty), foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (first(empty), foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (first(empty), foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (first(empty), foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (first(empty), foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (first(empty), foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (first(empty), foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (first(empty), foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (first(empty), foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (first(empty), foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (first(empty), foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (first(empty), foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a)) |= 9",
            "{\"a\":{\"a\":9}}\n",
            "",
            0,
        ),
    ])
}

/// #4800: `label $l | (FE, break $l)` stores exactly the outputs the bare `foreach` does. It was refused naming the state where jq
/// answers `["a"]` (`[path(..)?]` is `[["a"]]`, here `[]`). Rows captured from jq 1.7.1 (stdout, first stderr line, exit
/// code), both routes, over `path`, `[path(..)?]`, `del` and `|=` with `.`, `$i`, `empty` and `.a` EXTRACTs.
#[test]
fn test_caught_foreign_marker_destructure_foreach_under_a_label_comma_break_after_4800(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), break $l); .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), break $l); .))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), break $l); .))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), break $l); .)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), break $l); $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), break $l); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), break $l); $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), break $l); $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), break $l); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), break $l); empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), break $l); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), break $l); empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), break $l); .a))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), break $l); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), break $l); .a))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), break $l); .a)) |= 9",
            "{\"a\":{\"a\":9}}\n",
            "",
            0,
        ),
    ])
}

/// #4800: `label $l | ((FE), empty) | .` stores exactly the outputs the bare `foreach` does. It was refused naming the state where jq
/// answers `["a"]` (`[path(..)?]` is `[["a"]]`, here `[]`). Rows captured from jq 1.7.1 (stdout, first stderr line, exit
/// code), both routes, over `path`, `[path(..)?]`, `del` and `|=` with `.`, `$i`, `empty` and `.a` EXTRACTs.
#[test]
fn test_caught_foreign_marker_destructure_foreach_under_a_label_pipe_dot_after_comma_4800(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) | .; .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) | .; .))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) | .; .))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) | .; .)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) | .; $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) | .; $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) | .; $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) | .; $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) | .; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) | .; empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) | .; empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) | .; empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) | .; .a))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) | .; .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) | .; .a))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) | .; .a)) |= 9",
            "{\"a\":{\"a\":9}}\n",
            "",
            0,
        ),
    ])
}

/// #4800: `label $l | (break $l, FE)` stores exactly the outputs the bare `foreach` does. It was refused naming the state where jq
/// answers `["a"]` (`[path(..)?]` is `[["a"]]`, here `[]`). Rows captured from jq 1.7.1 (stdout, first stderr line, exit
/// code), both routes, over `path`, `[path(..)?]`, `del` and `|=` with `.`, `$i`, `empty` and `.a` EXTRACTs.
#[test]
fn test_caught_foreign_marker_destructure_foreach_under_a_label_break_ahead_control_4800(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (break $l, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (break $l, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (break $l, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (break $l, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (break $l, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); $i))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (break $l, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (break $l, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); $i))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (break $l, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); $i)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (break $l, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (break $l, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (break $l, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (break $l, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | (break $l, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | (break $l, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | (break $l, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; label $l | (break $l, foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
    ])
}
