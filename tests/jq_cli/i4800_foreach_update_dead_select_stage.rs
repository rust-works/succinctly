//! #4800: a `foreach` UPDATE that is a pipe holding a stage that emits nothing (`select(false)`, `empty`) emits nothing
//! whatever the stages around it do, so the caught foreign-`$x` `foreach` ahead of that stage is never asked where it
//! left the register. `FE` is `foreach (try ($x as {a:$a} | .)) as $y (.; .; .)`.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #4800: `FE | select(false)`, `(if .a then (FE, empty) else . end) | select(false)` and the like, over the source `.`
/// (a container state), were refused naming the state -- uncatchably under `?` -- where jq answers: the UPDATE never
/// emits, so `path` emits nothing and `del`/`|=` echo the document. The `empty` spelling of the same UPDATE was already
/// read; `select(false)` reached the stage machinery instead. A stage after the first one that emits nothing never runs
/// (`FE | select(false) | .a`, `(...) | empty | .a`), whatever spells the stage that emits nothing
/// (`first(empty)`, `limit(0; .)`, `select(false // null)`, a parenthesised one) and a parenthesised pipe around it;
/// a pipe that starts with such a stage (`select(false) | FE`, `(select(false) | 1 | FE) // 5`) keeps the read it
/// had (the `at > 0` guard is pinned by `test_foreach_extract_after_an_alternative_behind_an_if_is_read_per_branch_4853`). Rows captured from jq 1.7.1 (stdout, first stderr line, exit code), both routes.
#[test]
fn test_foreach_update_pipe_with_a_stage_that_emits_nothing_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | select(false); .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | select(false); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | select(false); .))",
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | select(false); .)) |= 9",
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach . as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | select(false); .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach . as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | select(false); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach . as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | select(false); .))",
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach . as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) else . end) | select(false); .)) |= 9",
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.,.a) as $i (.; (if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end) | select(false); $i))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.,.a) as $i (.; (if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end) | select(false); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"[path(. as $x | foreach . as $i (.; (if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end) | select(false); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"del(. as $x | foreach . as $i (.; (if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end) | select(false); .a))",
            "{\"a\":1,\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | select(false) | .a; .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | select(false) | .a; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; (if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end) | empty | .a; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach . as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .), empty) | select(null) | .a; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | first(empty); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | first(empty); .))",
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | limit(0; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | (select(false)); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | select(false // null); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach . as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | select(false)) | .a; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach . as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | select(false)) | .a; .))",
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | (select(false) | .a); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | empty | select(false); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | empty; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach . as $i (.; select(false) | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; (select(false) | 1 | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // 5; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
    ])
}
