//! #4901: bind shapes and message wording of a `$x` bound elsewhere than jq's register, destructured in a
//! `reduce` UPDATE (#4832 residuals).

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #4901: a `reduce` hands its UPDATE's output back as the next state without a path check, and the
/// final emission is judged on the accumulator's provenance, so an UPDATE that destructures an outer
/// `$x` and ends on `.` is jq's `[]` -- including after a source that moved the register onto the element
/// `$x` is (`.a as $x | reduce .a as $i (.; $x as {b:$q} | .)`), where the destructure's own step passes
/// because `$x` IS the register. The binary refused the first with `with result <accumulator>` and the
/// second with `near attempt to access element "b" of <$x>`. Covers an object, an array and a nested
/// pattern, a bind in front, a moving and a non-moving source, and `path`/`del`/`|=`. Every row captured
/// from `/usr/bin/jq` 1.7.1, both routes.
#[test]
fn test_4901_reduce_update_foreign_marker_destructure_returns_the_accumulator() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce 1 as $i (.; $x as {a:$a} | .))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | reduce 1 as $i (.; $x as {a:$a} | .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | reduce 1 as $i (.; $x as {a:$a} | .)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce (1,2) as $i (.; $x as {a:{b:$q}} | .))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce 1 as $i (.; $x as $q | $x as {a:$a} | .))",
            "[]\n",
            "",
            0,
        ),
        (
            r"[1,[2]]",
            r"path(. as $x | reduce 1 as $i (.; $x as [$p] | .))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(.a as $x | reduce .a as $i (.; $x as {b:$q} | .))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(.a as $x | reduce .a as $i (.; $x as $q | $x as {b:$q} | .))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(.a as $x | reduce .a as $i (.; $x as {b:$q} | .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(.a as $x | reduce .a as $i (.; $x as {b:$q} | .)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(.a as $x | reduce (.a, .a) as $i (.; $x as {b:$q} | .))",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #4901: the step a foreign `$x` raises on at a later source element is named, not the end of the
/// path. At step 1 `$x` is the register (the source's `.a?`), so `$x | . as {a:$a} | .` passes; at
/// step 2 the source's `.` has moved the register back to the root and the pattern step on `$x`
/// raises. The binary refused at step 1 with `with result {"b":1}`, which `try ... catch .` showed.
#[test]
fn test_4901_reduce_update_foreign_marker_pipe_names_the_later_step() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (r#"{"a":{"b":1}}"#, r"try path(.a as $x | reduce (.a?, .) as $i (.; $x | . as {a:$a} | .)) catch .", "\"Invalid path expression near attempt to access element \\\"a\\\" of {\\\"b\\\":1}\"\n", "", 0),
        (r#"{"a":{"b":1}}"#, r"path(.a as $x | reduce (.a?, .) as $i (.; $x | . as {a:$a} | .))", "", r#"Invalid path expression near attempt to access element "a" of {"b":1}"#, 5),
    ])
}

/// #4901: what must keep raising, with jq's wording: a navigation after the destructure, the
/// destructure's own value, a second pattern step on `.` once the register has moved, a `$x` that is
/// not the register (a source that moved it, or a `$x` bound at another node), and the same behind
/// `del`. A relaxation that stopped checking the destructure's steps, or the register the later steps
/// meet, would turn one of these into an answer.
#[test]
fn test_4901_reduce_update_foreign_marker_destructure_still_refuses() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce 1 as $i (.; $x as {a:$a} | .a))",
            "",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce 1 as $i (.; $x as {a:$a} | $a))",
            "",
            r#"Invalid path expression with result {"b":1}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce 1 as $i (.; $x as {a:$a} | . as {a:$b} | .))",
            "",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce 1 as $i (.; . as {a:$b} | $x as {a:$a} | .))",
            "",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce .a as $i (.; $x as {a:$a} | .))",
            "",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce .a as $i (.; $x as {a:$a} | $x))",
            "",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(.a as $x | reduce 1 as $i (.; $x as {b:$q} | .))",
            "",
            r#"Invalid path expression near attempt to access element "b" of {"b":1}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(.a as $x | reduce 1 as $i (.; $x as {b:$q} | .))",
            "",
            r#"Invalid path expression near attempt to access element "b" of {"b":1}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(.a as $x | reduce .a as $i (.; $x as {b:$q} | .b))",
            "",
            r#"Invalid path expression near attempt to access element "b" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(.a as $x | reduce .a as $i (.; $x as {b:$q} | $x))",
            "",
            r#"Invalid path expression with result {"b":1}"#,
            5,
        ),
    ])
}
