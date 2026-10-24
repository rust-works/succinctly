//! #5030: a fold source on a known register (after a navigating INIT) whose foreign `$x` destructure sits inside a nested
//! fold under a `try`/`?`.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #5030: on `{"a":{"b":1,"c":{"b":3}},"b":2}` after INIT `.a`, `try reduce ($x as {b:$q} | .) as $w (0; .)` as a
/// `foreach` source: the inner destructure raises, jq's `try` catches it and the source is empty, so `path` emits
/// nothing and `del`/`|=` leave the document. The marker sat under the `try` and the source was read by value, so
/// `del` deleted `.a`. A handler that emits (`catch 5`) and a `try` around a comma that emits first answer as before.
/// Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_source_try_around_a_nested_fold_on_a_known_register_5030() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach (try reduce ($x as {b:$q} | .) as $w (0; .)) as $y (.a; .; .))",
            "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach (try reduce ($x as {b:$q} | .) as $w (0; .)) as $y (.a; .; .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"[path(. as $x | foreach ((reduce ($x as {b:$q} | .) as $w (0; .))?) as $y (.a; .; .))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"(. as $x | foreach (try reduce ($x as {b:$q} | .) as $w (0; .)) as $y (.a; .; .)) |= 9",
            "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach (try reduce ($x as {b:$q} | .) as $w (0; .) catch 5) as $y (.a; .; .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach (try (1, ($x as {b:$q} | .))) as $y (.a; .; .))",
            "[\"a\"]\n",
            "",
            0,
        ),
    ])
}

/// #5030: only the forms that skip a raise without emitting are read through (a handler-less `try`, `?`,
/// `catch empty`): a handler that can emit is kept, so a foreign destructure in the handler itself (`try error("x")
/// catch reduce ($x as {b:$q} | .) ..`) still raises as jq does, and a `catch empty` around the nested fold leaves the
/// source empty. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_source_handler_destructure_on_a_known_register_still_raises_5030() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r#"del(. as $x | foreach (try error("x") catch reduce ($x as {b:$q} | .) as $w (0; .)) as $y (.a; .; .))"#,
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r#"(. as $x | foreach (try error("x") catch reduce ($x as {b:$q} | .) as $w (0; .)) as $y (.a; .; .)) |= 9"#,
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r#"path(. as $x | reduce (try error("x") catch ($x as {b:$q} | .)) as $y (.a; .))"#,
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach (try reduce ($x as {b:$q} | .) as $w (0; .) catch empty) as $y (.a; .; .))",
            "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
    ])
}

/// #5030: a `$x` the fold reads as jq's register, piped into a destructure (`$x | . as P`, `$x | (. as P | ..)`,
/// `$x | . | . as P`), destructures the register as `$x as P` does. After INIT `.a` with `.a as $x`, jq's register is
/// `$x`'s node, the pattern step moves it, and the path ends off it (`with result`), where the source was read by
/// value: `del` deleted `.a` and `path` answered `["a"]`. The same holds on a `null` INIT behind a computed stage and under
/// `?`/`try`. A `reduce` source (which backtracks), a register at the root and a bare `$x | .` answer as before.
/// Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_source_register_marker_piped_into_a_destructure_5030() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(.a as $x | foreach ($x | . as {b:$q} | .) as $y (.a; .; .))",
            "",
            "Invalid path expression with result {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach ($x | . as {b:$q} | $q) as $y (.a; .; .))",
            "",
            "Invalid path expression with result {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"(.a as $x | foreach ($x | (. as {b:$q} | .)) as $y (.a; .; .)) |= 9",
            "",
            "Invalid path expression with result {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"del(.a as $x | .a | foreach ($x | . | . as {b:$q,a:$r} | .) as $y (.; .))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"b\":1}",
            5,
        ),
        (
            r#"{"a":{"b":{"c":3}},"b":{"c":4}}"#,
            r"[path(. as $x | null | foreach ($x | . as {c:$q} | $q) as $y (null; $y))?]",
            "[[\"c\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"[path(. as $x | null | foreach (($x | . as {a:$q} | $q)?) as $y (null; $y))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | 1 | foreach ($x | . | . as {c:$q} | 1) as $y (null; .b?)) = 9",
            "{\"a\":null,\"c\":{\"b\":9}}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":null},"b":null}"#,
            r#"try path(.a as $x | foreach (try ($x | (. as {c:$q} | .))) as $y ((.a,.b); .)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | reduce ($x | . as {b:$q} | .) as $y (.a; .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(.a as $x | foreach ($x | . as {b:$q} | .) as $y (.; .; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach ($x | .) as $y (.a; .; .))",
            "[\"a\"]\n",
            "",
            0,
        ),
    ])
}

/// #5030 (review): the walks through a `reduce` that decide a fold body's loud refusal do not read a listed marker in
/// a comma, an `if` branch or a `limit` as the register on every output. Inside another fold's UPDATE, `($x, $x) | .
/// as [$q]` raises jq's own catchable error and `try` catches it; reading the marker as the register there refused
/// with an error no `try` catches. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_body_marker_in_a_comma_or_branch_is_not_the_register_5030() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1}"#,
            r#"try path(. as $x | foreach 1 as $i (.; .; foreach (($x, $x) | . as [$q] | .) as $y (.; 1; .))) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(try (.a.b as $x | foreach 1 as $i (.; foreach (($x, $x) | . as {a:$q} | 1) as $y (null; .; empty); .)))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1],"b":[2]}"#,
            r#"try del(. as $x | reduce .a as $i (.; foreach (limit(1; $x) | . as $w | $w as {b:$q} | .) as $y (null; .b?; empty))) catch "D""#,
            "\"D\"\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r#"try path(. as $x | foreach 1 as $i (.; .; foreach ((if true then $x else $x end) | . as [$q] | .) as $y (.; 1; .))) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
    ])
}

/// #5030: a fold source outside every fold body, after a stage that navigated to a missing key, so that jq's register is
/// a `null` (`.a as $x | .nope | reduce ($x as {b:$b} | .) ..`). A `$x` holding a container is not that `null`, and its
/// destructure raises at the pattern step. The routing to the resolver read a `null` ambient as never holding a foreign
/// marker, so the source was read by value: `path` answered `["nope"]`, `= 9` wrote a `nope` key and `del` left the
/// document. A `null` `$x` (identical to the register by kind), a `foreach` source and a non-`null` register answer as
/// before. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_source_foreign_destructure_on_a_null_register_after_a_stage_5030() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1},"c":{"b":2}}"#,
            r"path(.a as $x | .nope | reduce ($x as {b:$b} | .) as $y (.; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"b\":1}",
            5,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":2}}"#,
            r"path(.a as $x | .nope | reduce ($x | . as {b:$b} | .) as $y (.; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"b\":1}",
            5,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":2}}"#,
            r"(. as $x | .nope | reduce (($x as {b:$q} | $q), 1) as $y (.a | .c; .b?)) = 9",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1},\"c\":{\"b\":2}}",
            5,
        ),
        (
            r#"{"a":{"b":1},"b":null}"#,
            r"(.a as $x | .nope | reduce ($x | . | . as {b:$q} | $q) as $y (.a.b; .)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"b\":1}",
            5,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":2}}"#,
            r"del(. as $x | .nope | reduce (foreach ($x | . | . as {b:{c:$q}} | empty) as $w (0; .; .)) as $y (.a; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1},\"c\":{\"b\":2}}",
            5,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":2}}"#,
            r#"try path(.a as $x | .nope | reduce ($x | . as {a:$q} | .) as $y (null; .)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | .nope | reduce (first($x as {c:$q} | 1)) as $y (.b; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":2}}"#,
            r"path(.nope as $x | .zz | reduce ($x as {b:$b} | .) as $y (.; .))",
            "[\"zz\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":2}}"#,
            r"path(.a as $x | .nope | foreach ($x as {b:$b} | .) as $y (.; .; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"b\":1}",
            5,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":2}}"#,
            r"path(.a as $x | .c | reduce ($x as {b:$b} | .) as $y (.; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"b\":1}",
            5,
        ),
    ])
}

// #5030, the `reduce` spelling of a `null` `$x` below INIT's node: every row below is captured from jq 1.7.1, both routes.

/// The issue's rows: `$x` is `null` (bound at a missing key), INIT moved the register onto `.a`, a container.
#[test]
fn test_reduce_source_destructures_a_null_x_below_inits_register_5030() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{},"b":{}}"#,
            r"path(.a.c as $x | reduce ($x as {b:$q} | .) as $y (.a; .))",
            "",
            "Invalid path expression near attempt to access element \"b\"",
            5,
        ),
        (
            r#"{"a":{},"b":{}}"#,
            r"[path(.a.c as $x | reduce ($x as {b:$q} | .) as $y (.a; .))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// Rows from a 12,000-row differential grid (bind site x INIT x source wrapper x fold kind x context x document) that `main` answered or wrote and jq raises, empties, or answers differently: a `try`/`?` around the destructure catches the raise, a bare one does not.
#[test]
fn test_reduce_source_null_x_below_init_register_in_the_grid_5030() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":null},"b":1}"#,
            r"[path(.a.b as $x | reduce (($x as {b:$q} | .)?) as $y (.a; .c?))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":null},"b":1}"#,
            r#"try path(.a.b as $x | reduce (try ($x | . as {b:$q} | .)) as $y (.b; $y)) catch "C""#,
            "[\"b\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"(.a.c.d as $x | reduce (try ($x | . as {b:$q} | .)) as $y (.b; null)) = 9",
            "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":null},"b":1}"#,
            r#"try path(.a.b as $x | reduce (($x as {b:$q} | .)?) as $y (.a; $y)) catch "C""#,
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{},"b":{}}"#,
            r"path(.a.c as $x | reduce ($x as [$q] | .) as $y (.a; .))",
            "",
            "Invalid path expression near attempt to access element 0",
            5,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":2}}"#,
            r"del(.a.c as $x | reduce (try ($x | . as {b:$q} | .)) as $y (.a.b; null))",
            "{\"a\":{},\"c\":{\"b\":2}}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"c":null},"b":{"b":4}}"#,
            r"[path(.a.b as $x | reduce (try ($x | . as {b:$q} | .)) as $y (.b; .b))?]",
            "[[\"b\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":2}}"#,
            r"(.a.c.d as $x | reduce ($x as {b:$q} | .) as $y (.a.b; .)) = 9",
            "",
            "Invalid path expression near attempt to access element \"b\"",
            5,
        ),
        (
            r#"{"a":{"c":null},"b":{"b":4}}"#,
            r"(.a.c as $x | reduce (($x as {b:$q} | .)?) as $y (.a; .b)) |= 9",
            "{\"a\":9,\"b\":{\"b\":4}}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a.c.d as $x | reduce (($x as {b:$q} | .)?) as $y (.a; .b))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":null},"b":1}"#,
            r#"try path(.nope as $x | reduce ($x as {b:$q} | $q) as $y (.b; .)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":null},"b":1}"#,
            r"del(.a.b as $x | reduce ($x | . as {b:$q} | .) as $y (.a; .))",
            "",
            "Invalid path expression near attempt to access element \"b\"",
            5,
        ),
        (
            r#"{"a":{"c":null},"b":{"b":4}}"#,
            r"path(.nope as $x | reduce (try ($x as {b:$q} | .)) as $y (.b; $y))",
            "[\"b\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(.nope as $x | reduce (($x as {b:$q} | .)?) as $y (.a.b; .b))",
            "{\"a\":{\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
    ])
}

/// Guards: a `$x` that is the register (a container at INIT's node, or a `null`/`false` where INIT left the register on one) is still answered.
#[test]
fn test_reduce_source_x_that_is_the_register_keeps_its_answer_5030() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1}}"#,
            r"path(.a as $x | reduce ($x as {b:$q} | .) as $y (.a; .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"[path(.a as $x | reduce ($x as {b:$q} | .) as $y (.a; .))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(.nope as $x | reduce ($x as {b:$q} | .) as $y (.nope; .))",
            "[\"nope\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(.a as $x | reduce ($x as {b:$q} | .) as $y (.a; .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(.a as $x | reduce ($x | . as {b:$q} | .) as $y (.a; .))",
            "{}\n",
            "",
            0,
        ),
    ])
}

/// #5030 (review 5): `$x | E as P | ..` where `E` navigates the `$x` stage's value. jq runs `E` as a subexp on `$x`'s
/// node, so `$x | .a` hands on the child `jv` of that node, and the pattern's first step passes `path_intact` when the
/// register stands there (INIT `.a`, or a stage `.a`): it moves the register onto `.a.b`, as `($x | .a) as P` does. The
/// step was refused, so a plain pipe raised where jq answers `["a","b"]`, and a `try` around it swallowed the refusal:
/// `del` left the document where jq raises at the end of the path. A head reaching another node (`.b`, or `$x | .b`
/// off the register) still raises, or is caught. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_marker_stage_navigated_into_a_destructure_on_the_register_5030() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | .a | $x | .a as {b:$q} | $q)",
            "[\"a\",\"b\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"(. as $x | .a | $x | .a as {b:$q} | $q) |= 9",
            "{\"a\":{\"b\":9,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach ($x | .a as {b:$q} | $q) as $y (.a; .; $y))",
            "[\"a\",\"b\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach (try ($x | .a as {b:$q} | .)) as $y (.a; .; .))",
            "",
            "Invalid path expression with result {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach (try ($x | .a as {b:$q} | .)) as $y (.a; .; .))",
            "",
            "Invalid path expression with result {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"[path(. as $x | foreach (try ($x | .a as {b:$q} | .)) as $y (.a; .; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach ($x | .a as {b:$q} | .) as $y (.a; .; .))",
            "",
            "Invalid path expression with result {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"a":{"b":1}},"b":{"b":1}}"#,
            r"del(.a? as $x | .a.a | $x | .a? as {b:$q} | $q | .b?)",
            "{\"a\":{\"a\":{\"b\":1}},\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | .b | $x | .a as {b:$q} | $q)",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach (try ($x | .b as {b:$q} | .)) as $y (.a; .; .))",
            "",
            "",
            0,
        ),
    ])
}

/// #5030: a fold whose INIT moves jq's register (`.a`), inside another fold's body whose `.` is the register, reads its
/// source from the register INIT left: a foreign `$x` destructure there raises at its first step, as jq's does, where it
/// was read by value (`del` deleted `.a`, `|= 9` wrote, `path` answered). A `try` around the destructure catches the
/// raise and empties the source. The same on an INIT of two outputs (`(.a, .a)`). Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_source_foreign_x_destructure_in_another_fold_body_5030() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | .) as $y (.a; .; .); .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach 1 as $i (.; .; foreach ($x as {b:$q} | .) as $y (.a; .; .)))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach 1 as $i (.; reduce ($x as {b:$q} | .) as $y (.a; .); .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach 1 as $i (.; foreach (try ($x | . as {b:$q} | .)) as $y (.a; .; .); .))",
            "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | reduce 1 as $i (.; foreach ($x as {b:$q} | .) as $y (.a; .; .)))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach (1,2) as $i (.; foreach ($x as {b:$q} | .) as $y (.a; .; .); .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"[path(. as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | .) as $y (.a; .; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"(. as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | .) as $y (.a; .; .); .)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"[path(. as $x | foreach 1 as $i (.; foreach (($x as {b:$q} | .)?) as $y (.a; .; .); .))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"(. as $x | foreach 1 as $i (.; .; foreach (($x as {b:$q} | .)?) as $y (.a; .; .))) |= 9",
            "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(.nope as $x | foreach 1 as $i (.; foreach ($x | . as {b:$q} | .) as $y (.a; .; .); .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of null",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | .) as $y ((.a, .a); .; .); .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | .) as $y ((.a, .a); .; .); .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
    ])
}

/// #5030: the converse, left as it was: where INIT leaves the register on the node `$x` is, the destructure passes as
/// jq's does, and the fold's own emission is then judged. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_source_x_that_is_the_register_in_another_fold_body_keeps_its_answer_5030() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(. as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | .) as $y (.; .; .); .))",
            "",
            "Invalid path expression with result {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | $q) as $y (.; .; .); .))",
            "",
            "Invalid path expression with result {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
    ])
}

/// #5030: a nested `foreach` in another fold's UPDATE or EXTRACT whose INIT moved jq's register onto the node a
/// container `$x` is bound to (`.a as $x | foreach 1 as $i (.; FE; .)` with `FE`'s INIT `.a`), and whose source
/// destructures that `$x`. The pattern's step passes, as it does outside a body, and moves the register onto `$x`'s
/// child, so the UPDATE's `.c?` raises where the state is not that child. The source was read by value, so the register
/// stayed on `.a`: `del` deleted `.a.c` and `path` answered `["a","c"]`. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_source_x_that_is_the_register_in_another_fold_body_walks_it_5030() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(.a as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | $q) as $y (.a; .c?; .); .))",
            "",
            "Invalid path expression near attempt to access element \"c\" of {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | $q) as $y (.a; .c; .); .))",
            "",
            "Invalid path expression near attempt to access element \"c\" of {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"[path(.a as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | $q) as $y (.a; .c?; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"(.a as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | $q) as $y (.a; .c?; .); .)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"c\" of {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | $q) as $y (.a; $y; .); .))",
            "[\"a\",\"b\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | $q) as $y (.a; .; .c); .))",
            "",
            "Invalid path expression near attempt to access element \"c\" of {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | $q) as $y (.a; .; .); .))",
            "",
            "Invalid path expression with result {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach (1,2) as $i (.; foreach ($x | . as {b:$q} | .) as $y (.a; .; .); .))",
            "",
            "Invalid path expression with result {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach 1 as $i (.; .; foreach ($x as {b:$q} | $q) as $y (.a; .c?; .)))",
            "",
            "Invalid path expression near attempt to access element \"c\" of {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(.a as $x | foreach 1 as $i (.; .; foreach ($x as {b:$q} | $q) as $y (.a; .c?; .)))",
            "",
            "Invalid path expression near attempt to access element \"c\" of {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"[path(.a as $x | foreach 1 as $i (.; foreach (first($x as {b:$q} | .)) as $y (.a; .b; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":{"b":1}},"b":{"b":1}}"#,
            r#"try path(.b as $x | foreach 1 as $i (.; .; foreach (first($x as {b:$q} | .)) as $y (.b; .c?; .))) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
    ])
}

/// #5030: a `null` `$x` is `jv_identical` to a `null` register by kind, not by node, so it is not read as the register
/// here: these rows answered as jq does before and still do. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_source_null_x_in_another_fold_body_is_not_the_register_5030() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null,"b":null}"#,
            r"del(.a? as $x | foreach 1 as $i (.; .; foreach ((1, ($x as {b:$q} | .))) as $y (.a?; .; .b)))",
            "{\"a\":null,\"b\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":null}"#,
            r"del(.b as $x | foreach 1 as $i (.; foreach (($x as {b:$q} | .)?) as $y (.a.c; .; null); .))",
            "{\"a\":null,\"b\":null}\n",
            "",
            0,
        ),
    ])
}

/// #5030 (review): the same source behind an UPDATE or EXTRACT that emits nothing, which `foreach` never judges (jq
/// answers `[]`/the document), an array `$x`, a `$x` bound below the root, and a `foreach (1,2)` outer. The source's
/// trailing `.` is the document, not the register the walk moved, and is no yield to path-check: reading it so raised
/// where jq answers. The last rows are the ones whose message named the document where jq names `$x`'s node. Rows captured
/// from jq 1.7.1, both routes.
#[test]
fn test_fold_source_x_that_is_the_register_in_another_fold_body_trailing_dot_5030() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | .) as $y (.a; empty; .); .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(.a as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | .) as $y (.a; select(false); .); .))",
            "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"(.a as $x | foreach 1 as $i (.; foreach ($x | . as {b:$q} | .) as $y (.a; empty; .); .)) |= 9",
            "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | .) as $y (.a; .; empty); .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | .) as $y (.a; .; .); .))",
            "",
            "Invalid path expression with result {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach (1,2) as $i (.; foreach ($x as {b:$q} | .) as $y (.a; empty; .); .))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach 1 as $i (.; .; foreach ($x as {b:$q} | .) as $y (.a; empty; .)))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":[1,2],"b":0}"#,
            r"path(.a as $x | foreach 1 as $i (.; foreach ($x as [$q] | $q) as $y (.a; .; .); .))",
            "",
            "Invalid path expression with result [1,2]",
            5,
        ),
        (
            r#"{"a":[1,2],"b":0}"#,
            r"del(.a as $x | foreach 1 as $i (.; foreach ($x as [$q] | .) as $y (.a; empty; .); .))",
            "{\"a\":[1,2],\"b\":0}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2],"b":0}"#,
            r"(.a as $x | foreach 1 as $i (.; foreach ($x as [$q] | $q) as $y (.a; .[0]?; .); .)) |= 9",
            "",
            "Invalid path expression near attempt to access element 0 of [1,2]",
            5,
        ),
        (
            r#"{"a":{"c":{"b":5}},"b":2}"#,
            r"path(.a.c as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | $q) as $y (.a.c; $y; .); .))",
            "[\"a\",\"c\",\"b\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"c":{"b":5}},"b":2}"#,
            r"del(.a.c as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | $q) as $y (.a.c; .b?; .); .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"b\":5}",
            5,
        ),
    ])
}

/// #5030: a `try`/`?` around the destructure of a `$x` that is the register INIT moved onto, in a nested `foreach` that is
/// the sole UPDATE of another: the source is not empty (the #5113 verdict compared `$x` with the enclosing fold's step
/// register, the root, where the register is the node INIT moved onto), the pattern's step passes and moves the register
/// onto `$x`'s child, so the UPDATE or EXTRACT is judged against it, as the uncaught spelling is. Under `path`, `?`, `|=`
/// and `del`, `try`, `?` and `try $x as P`, an array pattern; the last two rows stay empty (`$x` is another node).
/// Rows captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_fold_source_caught_x_that_is_the_register_in_another_fold_body_walks_it_5030() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"[path(.a as $x | foreach 1 as $i (.; foreach (try ($x as {b:$q} | $q)) as $y (.a; $y; .); .))]",
            "[[\"a\",\"b\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"[path(.a as $x | foreach 1 as $i (.; foreach (($x as {b:$q} | $q)?) as $y (.a; $y; .); .))]",
            "[[\"a\",\"b\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"[path(.a as $x | foreach 1 as $i (.; foreach (try $x as {b:$q} | $q) as $y (.a; $y; .); .))]",
            "[[\"a\",\"b\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach 1 as $i (.; foreach (try ($x as {b:$q} | .)) as $y (.a; .c; .); .))",
            "",
            "Invalid path expression near attempt to access element \"c\" of {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"[path(.a as $x | foreach 1 as $i (.; foreach (try ($x as {b:$q} | .)) as $y (.a; .c?; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(.a as $x | foreach 1 as $i (.; foreach (try ($x as {b:$q} | $q)) as $y (.a; .c?; .); .))",
            "",
            "Invalid path expression near attempt to access element \"c\" of {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"(.a as $x | foreach 1 as $i (.; foreach (try ($x as {b:$q} | $q)) as $y (.a; .c?; .); .)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"c\" of {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach 1 as $i (.; foreach (try ($x as {b:$q} | .)) as $y (.a; .; .c); .))",
            "",
            "Invalid path expression near attempt to access element \"c\" of {\"b\":1,\"c\":{\"b\":3}}",
            5,
        ),
        (
            r#"{"a":[1,2],"b":0}"#,
            r"(.a as $x | foreach 1 as $i (.; foreach (try ($x as [$q] | $q)) as $y (.a; .[0]?; .); .)) |= 9",
            "",
            "Invalid path expression near attempt to access element 0 of [1,2]",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach 1 as $i (.; foreach (try ($x as {b:$q} | .)) as $y (.a; .; .); .))",
            "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"[path(.b as $x | foreach 1 as $i (.; foreach (try ($x as {b:$q} | $q)) as $y (.a; $y; .); .))]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5223 Phase 3: this issue's repro rows, with `path`, `del` and `|=`
/// wherever the row is a `path(...)`, as jq 1.7.1 answers them (stdout, the
/// first error message, the exit code). The path-mode interpreter carries
/// jq's register, so they match by construction, on both routes.
#[cfg(not(feature = "unshared-containers"))]
#[test]
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5030() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#, r"del(. as $x | foreach (try reduce ($x as {b:$q} | .) as $w (0; .)) as $y (.a; .; .))", "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":2}\n", "", 0),
        (r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#, r"del(.a as $x | foreach ($x | . as {b:$q} | .) as $y (.a; .; .))", "", "Invalid path expression with result {\"b\":1,\"c\":{\"b\":3}}", 5),
        (r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#, r"path(. as $x | foreach (try ($x | . as {b:$q} | .), 1) as $y (.a; .; .))", "[\"a\"]\n", "", 0),
        (r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#, r"del(. as $x | foreach (try ($x | . as {b:$q} | .), 1) as $y (.a; .; .))", "{\"b\":2}\n", "", 0),
        (r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#, r"(. as $x | foreach (try ($x | . as {b:$q} | .), 1) as $y (.a; .; .)) |= 9", "{\"a\":9,\"b\":2}\n", "", 0),
        (r#"{"a":{"b":1},"c":{"b":2}}"#, r"path(.a as $x | .nope | reduce ($x as {b:$b} | .) as $y (.; .))", "", "Invalid path expression near attempt to access element \"b\" of {\"b\":1}", 5),
        (r#"{"a":{"b":1},"c":{"b":2}}"#, r"del(.a as $x | .nope | reduce ($x as {b:$b} | .) as $y (.; .))", "", "Invalid path expression near attempt to access element \"b\" of {\"b\":1}", 5),
        (r#"{"a":{"b":1},"c":{"b":2}}"#, r"(.a as $x | .nope | reduce ($x as {b:$b} | .) as $y (.; .)) |= 9", "", "Invalid path expression near attempt to access element \"b\" of {\"b\":1}", 5),
        (r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#, r"path(. as $x | foreach ($x as {b:$q} | .) as $y (.a|tostring; .; .))", "", "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...", 5),
        (r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#, r"del(. as $x | foreach ($x as {b:$q} | .) as $y (.a|tostring; .; .))", "", "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...", 5),
        (r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#, r"(. as $x | foreach ($x as {b:$q} | .) as $y (.a|tostring; .; .)) |= 9", "", "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...", 5),
        (r#"{"a":{"a":{"b":1}},"b":{"b":1}}"#, r"path(.a.c as $x | foreach (try ($x | . as {b:$q} | .)) as $y (.a; .; $y))", "", "", 0),
        (r#"{"a":{"a":{"b":1}},"b":{"b":1}}"#, r"del(.a.c as $x | foreach (try ($x | . as {b:$q} | .)) as $y (.a; .; $y))", "{\"a\":{\"a\":{\"b\":1}},\"b\":{\"b\":1}}\n", "", 0),
        (r#"{"a":{"a":{"b":1}},"b":{"b":1}}"#, r"(.a.c as $x | foreach (try ($x | . as {b:$q} | .)) as $y (.a; .; $y)) |= 9", "{\"a\":{\"a\":{\"b\":1}},\"b\":{\"b\":1}}\n", "", 0),
    ])
}
