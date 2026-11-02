//! #4800: a `foreach` UPDATE that is an `if` around the caught foreign-`$x` `foreach` (alone or in a comma whose other
//! items emit nothing) on a condition other than `.`, behind a `try`/`?`, or with a `.` or `select(true)` stage after it,
//! is read as the bare `if . then FE else . end` is. `FE` is `foreach (try ($x as {a:$a} | .)) as $y (.; .; .)`.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #4800: `if .a then FE else . end` on a container state was refused naming the state where jq answers (nothing for an
/// `empty` EXTRACT, `[]` under a `?`, its own index error for `.a`): #4957's mark for the nested `foreach` read an `if`
/// only on a condition that cannot move the register, and jq runs every `if` condition in a subexp. A `.a` source whose
/// state takes the `else .` branch refused the `$i` emission where jq answers `["a"]`. Rows captured from jq 1.7.1
/// (stdout, first stderr line, exit code), both routes.
#[test]
fn test_foreach_update_if_on_a_pure_condition_around_the_foreach_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach . as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach . as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach . as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":true}",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach . as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; empty))",
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach . as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; (.a)?)) |= 9",
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach . as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach . as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach . as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":{\"a\":null}}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach . as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; empty))",
            "{\"a\":{\"a\":null}}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach . as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; (.a)?)) |= 9",
            "{\"a\":{\"a\":null}}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach .a as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; $i))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach .a as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; $i))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach .a as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; $i)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
    ])
}

/// #4800: the second step of a `(.a,.)` source runs on the `null` state the empty first step left, which takes the `else
/// .` branch, and the `.a` EXTRACT then raises jq's own catchable index error. `taken_if_branch` narrowed the step to the
/// branch it took only for a bare `if .`, so on `if .a`, behind a `try`/`?` and with a stage after the `if` the whole
/// `if` was judged and the error became an uncatchable guess that escaped the `?`/`try` where jq answers `[]` or `"c"`
/// (which is why #5010 kept those comma spellings refused). Rows captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_if_else_step_on_the_null_state_stays_catchable_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; .a)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; (.a)?))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; (.a)?))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .a)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; (.a)?))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; (.a)?))",
            "{}\n",
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
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .; .a)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .; (.a)?))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .; (.a)?))",
            "{\"a\":null}\n",
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
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; try (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end); .a)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; try (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end); (.a)?))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; try (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end); (.a)?))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end)?; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end)?; .a)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end)?; (.a)?))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end)?; (.a)?))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .; .a)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .; (.a)?))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .; (.a)?))",
            "{}\n",
            "",
            0,
        ),
    ])
}

/// #4800: `(if . then (FE, empty) else . end) | select(true)`, `if .a then (FE, empty) else . end` and a `try`/`?`
/// around `if . then (FE, empty) else . end` on a container state were refused naming the state where jq answers (a
/// `try` answered `"c"` where jq emits nothing): `select(true)` was neither a stage #4957 reads after the `if` nor one
/// #4754 lets follow a truthy-gated `if`, and #5010 kept the other two comma spellings refused. Rows captured from jq
/// 1.7.1, both routes.
#[test]
fn test_foreach_update_if_comma_branch_with_select_true_after_or_a_pure_condition_4800(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | select(true); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach . as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | select(true); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach .a as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | select(true); .))",
            "",
            "",
            0,
        ),
        (
            r"{}",
            r#"try path(. as $x | foreach .a as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | select(true); $i)) catch "c""#,
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | select(true); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | select(true); $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.,.a) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | select(true); .)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.,.a) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | select(true); (.a)?))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach .a as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r#"try path(. as $x | foreach .a as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .)) catch "c""#,
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach . as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; $i)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach .a as $i (.; try (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end); .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach .a as $i (.; try (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end); .))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r#"try path(. as $x | foreach .a as $i (.; try (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end); .)) catch "c""#,
            "",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach .a as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end)?; .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach .a as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end)?; .))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r#"try path(. as $x | foreach .a as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end)?; .)) catch "c""#,
            "",
            "",
            0,
        ),
    ])
}

/// #4800: the comma `if` read beyond the bare `if .` (on another condition, with a stage or a wrapper around it) is read
/// only in a `foreach` UPDATE whose `if` `taken_if_branch` finds, the only place the step whose `null` state takes the
/// `else .` branch is narrowed to that branch. Lifting #5010's refusal everywhere let a `.a` or `.b` index error escape
/// a `?` and a `try` where jq answers `[]` / `"C"`: in a `foreach` EXTRACT, behind a head stage (`. |`, `select(.) |`),
/// a `first`/`limit`/`catch` wrapper and a stage that computes (`| .b`, `| length`). Those keep the answers `main`
/// gave, which match jq here; a `reduce` UPDATE keeps its refusal too. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_comma_if_beyond_the_bare_form_outside_a_read_foreach_update_keeps_its_refusal_4800(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .b; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | .b; $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; first(if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; limit(1; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) catch empty; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try ([path(. as $x | foreach (.a,.) as $i (.; try (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) catch .; .a))]) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; . | if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; .; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | length))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; .; select(.) | if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; .; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | select(true) | .))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}
