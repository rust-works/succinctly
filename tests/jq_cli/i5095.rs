//! #5095: a refusal of a `$x` the resolver knows is bound elsewhere than jq's path register is jq's own,
//! catchable one, not an uncatchable guess, on a document whose two positions hold equal values.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #5095: a fold SOURCE after a navigating INIT runs on the register's own frame (#5013), which names where the
/// register is (`.b`) and holds its value. A `$x` bound at `.a` holding an equal `{"b":1}` was still read as possibly
/// the register by pointer (#3423), so the refusal of its pattern step escaped `?`/`try` as a guess (exit 5) where
/// jq's is caught. Uncaught, the refusal and its message are as before; an INIT that navigated and then computed
/// (`$r | .b | length`, no position kept) answers as before, and a `$x` bound at INIT's own node refuses at the end of
/// the path as before. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_source_destructure_of_an_equal_sibling_is_caught_5095() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1},"b":{"b":1}}"#,
            r"[path(. as $r | .a as $x | foreach ($x as {b:$q} | .) as $y (.b; .a?; 1))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"b":{"b":1}}"#,
            r#"try path(. as $r | .a as $x | foreach ($x as {b:$q} | .) as $y (.b; .a?; 1)) catch "caught""#,
            "\"caught\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"b":{"b":1}}"#,
            r"path(. as $r | .a as $x | foreach ($x as {b:$q} | .) as $y (.b; .a?; 1))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"b\":1}",
            5,
        ),
        (
            r#"{"a":{"b":1},"b":{"b":1}}"#,
            r"[path(. as $r | .a as $x | foreach ($x as {b:$q} | .) as $y ($r | .b | length; .a?; 1))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"b":{"b":1}}"#,
            r"[path(.a as $x | reduce ($x as {b:$q} | .) as $y (.b; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"b":{"b":1}}"#,
            r#"try ((.a as $x | reduce ($x as {b:$q} | .) as $y (.b; .)) |= 9) catch "caught""#,
            "\"caught\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"b":{"b":1}}"#,
            r#"try del(.a as $x | foreach ($x as {b:$q} | .) as $y (.b; .; .)) catch "caught""#,
            "\"caught\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"b":{"b":1}}"#,
            r"del(.a as $x | foreach ($x as {b:$q} | .) as $y (.b; .; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"b\":1}",
            5,
        ),
        (
            r#"{"a":{"b":1},"b":{"b":1}}"#,
            r"path(.b as $x | foreach ($x as {b:$q} | .) as $y (.b; .; .))",
            "",
            "Invalid path expression with result {\"b\":1}",
            5,
        ),
        (
            r#"{"a":{"b":true},"b":{"b":true}}"#,
            r"[path(. as $r | .a as $x | foreach ($x as {b:$q} | .) as $y (.b; .a?; 1))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5095: the same in a plain pipe, where an untracked stage carries the register (`.b | "lit" | ..`): a `$x` bound
/// at another position is another node, so `?`/`try` catch the refusal of its step as jq's do. Positions that name
/// one node (`.[1]` and `.[-1]` of a two-element array) and a `null` register (identical to a `null` `$x` by kind)
/// still read `$x` as the register and answer. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_pipe_destructure_of_an_equal_sibling_is_caught_5095() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1},"b":{"b":1}}"#,
            r#"[path(.a as $x | .b | "lit" | $x as {b:$q} | .)?]"#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"b":{"b":1}}"#,
            r#"try del(.a as $x | .b | "lit" | $x as {b:$q} | .) catch "caught""#,
            "\"caught\"\n",
            "",
            0,
        ),
        (
            r#"{"a":[1],"b":[1]}"#,
            r"[path(.a as $x | .b | 1 | $x as [$q] | .)?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"[{"b":1},{"b":1}]"#,
            r"[path(.[0] as $x | .[-1] | 1 | $x as {b:$q} | .)?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"[{"b":1},{"b":1}]"#,
            r"path(.[1] as $x | .[-1] | 1 | $x as {b:$q} | $q)",
            "[-1,\"b\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":null}"#,
            r"path(.a as $x | .b | 1 | $x as {b:$q} | $q)",
            "[\"b\",\"b\"]\n",
            "",
            0,
        ),
    ])
}

/// #5095: a `reduce` nested in a fold SOURCE that mentions `$x` drops the register it was entered with (#3984),
/// and recorded it lost *anywhere*, so a refusal of its root accumulator (`.c`, `.b`, `.a.c` on `.`) was a guess
/// that escaped the `?`. The frame carried the register's value (`.a`'s node), and jq's register only moves down
/// from there, so the root is not it and the refusal is caught as jq's is. Uncaught, the refusal is as before.
/// Rows captured from jq 1.7.1, both routes.
#[test]
fn test_nested_reduce_refusal_of_a_root_accumulator_is_caught_5095() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"[path(.a as $x | foreach ((reduce ($x as {b:$q} | .) as $w (.; .c))?) as $y (.a; .; .))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach ((reduce ($x as {b:$q} | .) as $w (.; .c))) as $y (.a; .; .))",
            "",
            "Invalid path expression near attempt to access element \"c\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r#"try del(.a as $x | foreach ((reduce ($x as {b:$q} | .) as $w (.; .c))) as $y (.a; .; .)) catch "caught""#,
            "\"caught\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"[path(.a as $x | foreach ((reduce ($x as {b:$q} | .) as $w (.; .b))?) as $y (.a; .; .))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"[path(.a as $x | foreach ((reduce ($x as {b:$q} | .) as $w (.; .a.c))?) as $y (.a; .; .))]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5095 review, #5103: a `$x` that is the register's own node spelled through a slice (`.a[0:]` of a non-empty array
/// *is* `.a` in jq) or at another depth (`.a[0:][0]` is `.a[0]`) is read as that node, so its destructure passes and jq's
/// answer follows (`[["a",0,"b"]]`, and `del` writes `{"a":[{},{"b":2}]}`), where #5095 kept the refusal loud and `main`
/// before it answered `[]` on the trackable stage. Never a caught refusal, the silent wrong answer. Both routes.
#[test]
fn test_destructure_of_the_register_spelled_through_a_slice_answers_as_jq_5095() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":[{"b":1},{"b":2}]}"#,
            r"[path(.a[0:] as $x | .a | 1 | $x as [{b:$q}] | $q)?]",
            "[[\"a\",0,\"b\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":2}]}"#,
            r#"try del(.a[0:] as $x | .a | 1 | $x as [{b:$q}] | $q) catch "caught""#,
            "{\"a\":[{},{\"b\":2}]}\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":2}]}"#,
            r"[path(.a as $x | .a[0:] | 1 | $x as [{b:$q}] | $q)?]",
            "[[\"a\",{\"start\":0,\"end\":null},0,\"b\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":2}]}"#,
            r"[path(.a[0:][0] as $x | .a[0] | 1 | $x as {b:$q} | $q)?]",
            "[[\"a\",0,\"b\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":2}]}"#,
            r"[path(.a[0:] as $x | reduce ($x as [{b:$q}] | $q) as $y (.a; .))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r#"try path(.a as $x | .a[0:] | foreach ($x as [$q] | .) as $y (.; .a?; 1)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":2}]}"#,
            r"[path(.a[0:] as $x | .a | $x as [{b:$q}] | $q)?]",
            "[[\"a\",0,\"b\"]]\n",
            "",
            0,
        ),
    ])
}
