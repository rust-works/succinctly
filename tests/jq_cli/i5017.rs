//! #5017: a fold source after a navigating INIT, outside every fold body, runs with jq's register where INIT left it
//! (its position and value on the frame), so a `$x` bound at INIT's own node is that register and any other marker's
//! destructure raises at its step, however the destructure is wrapped.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #5017: on `{"a":{"b":1,"c":{"b":3}},"b":2}`, after INIT `.a`: a caught destructure of a foreign `$x` behind a stage
/// (`$x | . as P`), a `label` or a `select(true)`, or beside a comma item, empties that item as jq's `try` does
/// (`del` deleted `.a`, the comma answered `["a"]` twice); a `$x` bound at `.a` is the register, so its destructure
/// walks from it and jq's refusal at the end of the path stands (it answered `["a"]`), and `$x | .c` is
/// `["a","c"]` (it raised). Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_source_after_a_navigating_init_runs_on_the_moved_register_5017() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach (try ($x | . as {b:$q} | .)) as $y (.a; .; .))",
            "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach (label $f | try ($x as {b:$q} | .)) as $y (.a; .; .))",
            "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach (try (select(true) | $x as {b:$q} | .)) as $y (.a; .; .))",
            "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach ((try ($x as {b:$q} | .)), 1) as $y (.a; .; .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach ($x as {b:$q} | .) as $y (.a; .; .))",
            "",
            "Invalid path expression with result {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach ($x | .c) as $y (.a; .; $y))",
            "[\"a\",\"c\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach (try ($x | . as {b:$q} | .)) as $y (.a; .; .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"(.a as $x | foreach ($x | .c) as $y (.a; .; $y)) |= 9",
            "{\"a\":{\"b\":1,\"c\":9},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(.a as $x | foreach ($x as {b:$q} | .) as $y (.a; .; .))",
            "",
            "Invalid path expression with result {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"[path(. as $x | foreach (label $f | try ($x as {b:$q} | .)) as $y (.a; .; .))]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5017: the same on a later INIT fork (`(.a, .a)`: the second fork runs on the register too, where it raised), and a
/// caught destructure spelled through `.` stages or a nested pipe (`$x | . | . as P`, `($x | .) | . as P`,
/// `$x | (. | .) as P`). Inside another fold's body the shared caught-destructure reading meets the same spellings:
/// a `reduce` UPDATE whose state is the register raises jq's `with result null`, and a `foreach` UPDATE over
/// `.a?` emits nothing, as jq does (both answered otherwise). Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_source_moved_register_on_later_forks_and_dot_spelled_catches_5017() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach ($x | .c) as $y ((.a, .a); .; $y))",
            "[\"a\",\"c\"]\n[\"a\",\"c\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach (try ($x | . | . as {b:$q} | .)) as $y (.a; .; .))",
            "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach (try (($x | .) | . as {b:$q} | .)) as $y (.a; .; .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach (try ($x | (. | .) as {b:$q} | .)) as $y (.a; .; .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | reduce 1 as $i (.a?; foreach (try ($x | . as {b:$a} | .)) as $y (.; .; .)))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach .a? as $i (.; foreach (try ($x | . as {b:$a} | .)) as $y (.; .; .)))",
            "",
            "",
            0,
        ),
    ])
}

/// #5017: a fold whose INIT cannot move the register, entered on a frame that already names jq's register and holds
/// its value, runs its SOURCE from that register too: a `reduce` nested in the source of a `foreach` after INIT `.a`,
/// or a fold behind a computed stage (`1 |`, `.a | tostring |`), now raises at a foreign `$x`'s destructure as jq does
/// (`del` deleted `.a`); a `$x` bound at the register's node, a caught destructure and an unmoved INIT answer as before.
/// Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_entered_on_a_register_frame_reads_a_foreign_destructure_5017() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach (reduce ($x as {b:$q} | .) as $w (0; .)) as $y (.a; .; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach (reduce ($x as {b:$q} | .) as $w (0; .)) as $y (.a; .; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | reduce (foreach ($x as {b:$q} | .) as $w (0; .; .)) as $y (.a; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach (reduce ($x as {b:$q} | .) as $w (0; .)) as $y (.a; .; .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach (reduce (try ($x as {b:$q} | .)) as $w (0; .)) as $y (.a; .; .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach (reduce ($x as {b:$q} | .) as $w (0; .)) as $y (.; .; .))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach (1 | reduce ($x as {b:$q} | .) as $w (0; .)) as $y (.a; .; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | .a | tostring | reduce ($x as {b:$q} | .) as $w (0; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | 1 | foreach ($x as {b:$q} | .) as $w (0; .; .))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
    ])
}

/// #5017: the boundary of that reading. An INIT that navigated through a `$r` marker and then computed can move the
/// register, so a fold behind a computed stage is not read as running on the frame's register there (the wrong reading
/// classed a `$x` at `.a` as foreign and changed jq's answer). A later INIT fork that cannot move the register is read
/// as the first one is: `(1,2)`/`(0,0)` forks raise, or empty a caught destructure, as jq does. Rows captured from jq
/// 1.7.1, both routes.
#[test]
fn test_fold_entered_on_a_register_frame_boundary_and_later_forks_5017() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"try path(. as $r | .a as $x | null | reduce ($x as [$q] | .) as $w ($r | first(.a, .b) | tostring; .)) catch .",
            "\"Cannot index object with number\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $r | .a as $x | 1 | reduce (try ($x as {b:$q} | .)) as $y ($r | .a | tostring; $x | .c))",
            "",
            "Invalid path expression near attempt to access element \"c\" of {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach (reduce (select(. == null) | $x as {b:$q} | .) as $w ((1,2); .)) as $y (.a; .; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(.a as $x | 1 | foreach (try ($x as {b:$q} | .)) as $y ((0,0); .; .))",
            "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
    ])
}
