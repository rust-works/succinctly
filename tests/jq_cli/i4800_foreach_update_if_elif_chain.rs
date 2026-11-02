//! #4800: a `foreach` UPDATE that is an `if`/`elif` chain with the caught foreign-`$x` `foreach` in one arm. `FE` is
//! `foreach (try ($x as {a:$a} | .)) as $y (.; .; .)`.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #4800: a step runs the one arm its conditions pick, and `taken_then_branch` read only the outermost `if`'s `then`.
/// With `FE` behind an `elif` (`if .a then 7 elif . then FE else .a end`) the step that picked it kept the whole-`if`
/// reading, which refuses the navigating `else` naming the state -- uncatchably under `?` -- where jq answers. The chain
/// is now walked as `taken_if_branch` walks it: a falsy condition continues into the `else`, the first truthy one names
/// its `then`. Rows captured from jq 1.7.1 (stdout, first stderr line, exit code), both routes; every row but the
/// second (a control that already matched) fails on `main`, and removing the walk fails the test.
#[test]
fn test_foreach_update_elif_chain_with_the_caught_foreach_in_a_later_arm_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; if .a then 7 elif . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else .a end; $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach . as $i (.; if .a then 7 elif . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else .a end; $i))",
            "",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach .a as $i (.; if .a then 7 elif . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else .a end; $i))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach .a as $i (.; if .a then 7 elif . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) else .a end; .)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach .a as $i (.; if .b then 7 elif . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i)) |= 9",
            "{\"a\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach .a as $i (.; if .b then 7 elif . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else .a end; .)) catch "c""#,
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r#"try path(. as $x | foreach .a as $i (.; if .b then 7 elif . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end; .a)) catch "c""#,
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.,.a) as $i (.; if .b then 7 elif . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else null end; empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if false then 7 elif . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else .a end; empty))",
            "",
            r#"Invalid path expression near attempt to access element "a" of null"#,
            5,
        ),
        (
            r#"{"a":1}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if .b then 7 elif . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else .a end; empty))",
            "",
            r#"Invalid path expression near attempt to access element "a" of null"#,
            5,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if .b then 7 elif . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) else .a end; empty))",
            "",
            r#"Invalid path expression near attempt to access element "a" of null"#,
            5,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if .b then 7 elif . then ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty) else .a end; .))",
            "",
            r#"Invalid path expression near attempt to access element "a" of null"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if .a then 7 elif . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else 3 end; $i)) catch "c""#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"del(. as $x | foreach .a as $i (.; if .b then 7 elif . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else .a end; .))",
            "{\"a\":1}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach .a as $i (.; if .a then 7 elif .a then 7 elif . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else .a end; $i))",
            "{\"a\":null}\n",
            "",
            0,
        ),
    ])
}

/// #4800: the chain is read only where `emptied_update_read` reads it. That reading is what states `FE` empty on the
/// earlier step that runs it with `$x` off the register (jq's `.a` source puts the register on `1`); where it does not
/// hold -- an `elif` condition that is not `.`, a fold in two arms -- that step would emit, and marking the next one
/// let `del` echo the document where jq raises the index error on the `null` the empty step left. These stay the
/// refusals they were (the message differs from jq's: `with result`, not `near attempt to access element`).
#[test]
fn test_foreach_update_elif_chain_not_read_by_emptied_update_stays_refused_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if .b then 7 elif .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else .a end; empty))",
            "",
            r"Invalid path expression",
            5,
        ),
        (
            r#"{"a":1}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if .b then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) elif . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else .a end; empty))",
            "",
            r"Invalid path expression",
            5,
        ),
    ])
}
