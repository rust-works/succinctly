//! #5179: the two shapes #5012 left unread when it placed a `foreach` UPDATE's `catch` handler on every step. Whether jq's
//! register is the state when the nested `foreach`'s source destructures `$x` is a question of node identity, which a
//! value cannot settle: an earlier step's `catch $i` leaves the node `.a` as the state, and a later step's pattern may land
//! on that very node. The state's position (where the accumulator was last reached) settles it, and so does a one-entry
//! nested pattern that does not raise, which lands on the node the innermost entry reaches.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #5179: The issue's first row, and the same shape with more steps, a six-step run, a nested pattern, an array
/// pattern and a `.z |` wrapper. The state an earlier step's `catch $i` left is the node `$x`'s pattern lands on, so
/// the pattern's `.` emits it on the register. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_try_handler_landing_equal_to_the_state_is_that_node_5179() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1}}"#,
            r"[path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"a\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"a\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            "Cannot index number with string \"a\"",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"[path(. as $x | foreach (.a,.,.a,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"a\"],[],[\"a\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (.a,.,.a,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"a\"]\n[]\n[\"a\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | foreach (.a,.,.a,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | foreach (.a,.,.a,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            "Cannot index number with string \"a\"",
            5,
        ),
        (
            r#"{"a":{"b":{"c":1}}}"#,
            r"[path(. as $x | foreach (.a.b,.,.a.b) as $i (null; try (foreach ($x as {a:{b:$a}} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"a\",\"b\"],[],[\"a\",\"b\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":{"c":1}}}"#,
            r"path(. as $x | foreach (.a.b,.,.a.b) as $i (null; try (foreach ($x as {a:{b:$a}} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"a\",\"b\"]\n[]\n[\"a\",\"b\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":{"c":1}}}"#,
            r"del(. as $x | foreach (.a.b,.,.a.b) as $i (null; try (foreach ($x as {a:{b:$a}} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":{"c":1}}}"#,
            r"(. as $x | foreach (.a.b,.,.a.b) as $i (null; try (foreach ($x as {a:{b:$a}} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            "Cannot index number with string \"a\"",
            5,
        ),
        (
            r#"{"a":[{"b":1}]}"#,
            r"[path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:[$a]} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"a\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1}]}"#,
            r"path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:[$a]} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"a\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1}]}"#,
            r"del(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:[$a]} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1}]}"#,
            r"(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:[$a]} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            "Cannot index number with string \"a\"",
            5,
        ),
        (
            r#"{"z":{"a":{"b":1}}}"#,
            r"[path(.z | . as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"z\",\"a\"],[\"z\"],[\"z\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"z":{"a":{"b":1}}}"#,
            r"path(.z | . as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"z\",\"a\"]\n[\"z\"]\n[\"z\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"z":{"a":{"b":1}}}"#,
            r"del(.z | . as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"z":{"a":{"b":1}}}"#,
            r"(.z | . as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            "Cannot index number with string \"a\"",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"[path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (0; .a?; $y)) catch $i; .))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | foreach (.a,.,.a) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | foreach (.a,.,.a) as $i (null; try (foreach (try ($x as {a:$a} | .)) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "9\n",
            "",
            0,
        ),
    ])
}

/// #5179: the row #5012 pinned as refused: the root `catch $i` left on the first step is the node the second step's
/// pattern lands on. jq emits `["a"]` and then raises its catchable `with result` error; `main` raised the same error
/// uncatchably. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_try_handler_landing_equal_to_the_state_with_a_container_state_5179(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (foreach ($x as {a:$a} | .) as $y (.; .a?; .)) catch $i; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; try (foreach ($x as {a:$a} | .) as $y (.; .a?; .)) catch $i; $i))",
            "[\"a\"]\n",
            "Invalid path expression with result {\"a\":{\"a\":null}}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; try (foreach ($x as {a:$a} | .) as $y (.; .a?; .)) catch $i; $i))",
            "",
            "Invalid path expression with result {\"a\":{\"a\":null}}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.a,.) as $i (.; try (foreach ($x as {a:$a} | .) as $y (.; .a?; .)) catch $i; $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":{\"a\":null}}",
            5,
        ),
    ])
}

/// #5179: A landing equal to the state by value at another position is a different node: the pattern's `.` emits the
/// state off the register, as for a landing that differs. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_try_handler_landing_equal_to_the_state_by_value_at_another_position_5179(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1},"c":{"b":1}}"#,
            r"[path(. as $x | foreach (.c,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"c\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":1}}"#,
            r"path(. as $x | foreach (.c,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"c\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":1}}"#,
            r"del(. as $x | foreach (.c,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":1}}"#,
            r"(. as $x | foreach (.c,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            "Cannot index number with string \"a\"",
            5,
        ),
        (
            r#"{"a":[1],"c":[1]}"#,
            r"[path(. as $x | foreach (.c,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"c\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1],"c":[1]}"#,
            r"path(. as $x | foreach (.c,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"c\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1],"c":[1]}"#,
            r"del(. as $x | foreach (.c,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":[1],"c":[1]}"#,
            r"(. as $x | foreach (.c,.,.a) as $i (null; try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            "Cannot index number with string \"a\"",
            5,
        ),
    ])
}

/// #5179: The issue's second row, and a nested pattern whose innermost key is absent. The one-entry nested pattern
/// lands on a value the state is not, so the pattern's `.` emits the state off the register. Rows captured from jq
/// 1.7.1, both routes.
#[test]
fn test_foreach_update_try_handler_nested_pattern_that_does_not_raise_5179() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r#"[path(. as $x | foreach (.a,.) as $i (.; not | try (foreach ($x as {a:{b:$b}} | .) as $y (null; error("x"); $y)) catch 7; $i))?]"#,
            "[[\"a\"],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"path(. as $x | foreach (.a,.) as $i (.; not | try (foreach ($x as {a:{b:$b}} | .) as $y (null; error("x"); $y)) catch 7; $i))"#,
            "[\"a\"]\n[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"del(. as $x | foreach (.a,.) as $i (.; not | try (foreach ($x as {a:{b:$b}} | .) as $y (null; error("x"); $y)) catch 7; $i))"#,
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"(. as $x | foreach (.a,.) as $i (.; not | try (foreach ($x as {a:{b:$b}} | .) as $y (null; error("x"); $y)) catch 7; $i)) |= 9"#,
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"[path(. as $x | foreach (.a.b,.a,.) as $i (null; try (foreach ($x as {a:{a:$b}} | .) as $y (.; .a?; .)) catch $i; $i))?]",
            "[[\"a\",\"b\"],[\"a\"],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (.a.b,.a,.) as $i (null; try (foreach ($x as {a:{a:$b}} | .) as $y (.; .a?; .)) catch $i; $i))",
            "[\"a\",\"b\"]\n[\"a\"]\n[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | foreach (.a.b,.a,.) as $i (null; try (foreach ($x as {a:{a:$b}} | .) as $y (.; .a?; .)) catch $i; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | foreach (.a.b,.a,.) as $i (null; try (foreach ($x as {a:{a:$b}} | .) as $y (.; .a?; .)) catch $i; $i)) |= 9",
            "9\n",
            "",
            0,
        ),
    ])
}

/// #5179: Not read, and as before: a head before the nested `foreach` hands it a value that is not the state, so
/// identity by position says nothing. Pinned so a change is seen. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_try_handler_landing_test_not_read_behind_a_head_5179() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1}}"#,
            r"[path(. as $x | foreach (.a,.,.a) as $i (null; not | try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"a\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (.a,.,.a) as $i (null; not | try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"a\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | foreach (.a,.,.a) as $i (null; not | try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | foreach (.a,.,.a) as $i (null; not | try (foreach ($x as {a:$a} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            "Cannot index number with string \"a\"",
            5,
        ),
    ])
}

/// #5179: The landing test reads an array pattern (bare, or under an object entry) and a nested pattern that reaches
/// below a `null`: a one-element array pattern lands on the element it names, a one-entry or one-element pattern below
/// an absent key lands on a `null`, and a pattern of two entries or two elements is answered by an earlier reading, so
/// the answer is jq's. Covers a landing at the state's position (`.a[0]` after `catch $i` left it) and at another
/// position (`.c`, an equal value). Rows captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_try_handler_array_and_multi_entry_patterns_5179() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{}}"#,
            r"[path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:{c:$c}}} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"a\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{}}"#,
            r"path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:{c:$c}}} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"a\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{}}"#,
            r"del(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:{c:$c}}} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{}}"#,
            r"(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:{c:$c}}} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            r#"Cannot index number with string "a""#,
            5,
        ),
        (
            r#"{"a":{}}"#,
            r"[path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:[$c]}} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"a\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{}}"#,
            r"path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:[$c]}} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"a\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{}}"#,
            r"del(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:[$c]}} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{}}"#,
            r"(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:[$c]}} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            r#"Cannot index number with string "a""#,
            5,
        ),
        (
            r#"{"a":[{"b":1}]}"#,
            r"[path(. as $x | foreach (.a[0],.,.a[0]) as $i (null; try (foreach ($x as {a:[$a]} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"a\",0],[],[\"a\",0]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1}]}"#,
            r"path(. as $x | foreach (.a[0],.,.a[0]) as $i (null; try (foreach ($x as {a:[$a]} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"a\",0]\n[]\n[\"a\",0]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1}]}"#,
            r"del(. as $x | foreach (.a[0],.,.a[0]) as $i (null; try (foreach ($x as {a:[$a]} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1}]}"#,
            r"(. as $x | foreach (.a[0],.,.a[0]) as $i (null; try (foreach ($x as {a:[$a]} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            r#"Cannot index number with string "a""#,
            5,
        ),
        (
            r#"{"a":[{"b":1}],"c":{"b":1}}"#,
            r"[path(. as $x | foreach (.c,.,.c) as $i (null; try (foreach ($x as {a:[$a]} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"c\"],[],[\"c\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1}],"c":{"b":1}}"#,
            r"path(. as $x | foreach (.c,.,.c) as $i (null; try (foreach ($x as {a:[$a]} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"c\"]\n[]\n[\"c\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1}],"c":{"b":1}}"#,
            r"del(. as $x | foreach (.c,.,.c) as $i (null; try (foreach ($x as {a:[$a]} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1}],"c":{"b":1}}"#,
            r"(. as $x | foreach (.c,.,.c) as $i (null; try (foreach ($x as {a:[$a]} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            r#"Cannot index number with string "c""#,
            5,
        ),
        (
            r#"[{"b":1}]"#,
            r"[path(. as $x | foreach (.[0],.,.[0]) as $i (null; try (foreach ($x as [$a] | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[0],[],[0]]\n",
            "",
            0,
        ),
        (
            r#"[{"b":1}]"#,
            r"path(. as $x | foreach (.[0],.,.[0]) as $i (null; try (foreach ($x as [$a] | .) as $y (0; .a?; $y)) catch $i; .))",
            "[0]\n[]\n[0]\n",
            "",
            0,
        ),
        (
            r#"[{"b":1}]"#,
            r"del(. as $x | foreach (.[0],.,.[0]) as $i (null; try (foreach ($x as [$a] | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"[{"b":1}]"#,
            r"(. as $x | foreach (.[0],.,.[0]) as $i (null; try (foreach ($x as [$a] | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            r"Cannot index number with number",
            5,
        ),
        (
            r#"{"a":{"b":1},"c":2}"#,
            r"[path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:$a,c:$c} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"a\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"c":2}"#,
            r"path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:$a,c:$c} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"a\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"c":2}"#,
            r"del(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:$a,c:$c} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"c":2}"#,
            r"(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:$a,c:$c} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            r#"Cannot index number with string "a""#,
            5,
        ),
        (
            r#"[{"b":1},2]"#,
            r"[path(. as $x | foreach (.[0],.,.[0]) as $i (null; try (foreach ($x as [$a,$c] | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[0],[],[0]]\n",
            "",
            0,
        ),
        (
            r#"[{"b":1},2]"#,
            r"path(. as $x | foreach (.[0],.,.[0]) as $i (null; try (foreach ($x as [$a,$c] | .) as $y (0; .a?; $y)) catch $i; .))",
            "[0]\n[]\n[0]\n",
            "",
            0,
        ),
        (
            r#"[{"b":1},2]"#,
            r"del(. as $x | foreach (.[0],.,.[0]) as $i (null; try (foreach ($x as [$a,$c] | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"[{"b":1},2]"#,
            r"(. as $x | foreach (.[0],.,.[0]) as $i (null; try (foreach ($x as [$a,$c] | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            r"Cannot index number with number",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":2}}"#,
            r"[path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:$b,c:$c}} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"a\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":2}}"#,
            r"path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:$b,c:$c}} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"a\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":2}}"#,
            r"del(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:$b,c:$c}} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":2}}"#,
            r"(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:$b,c:$c}} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            r#"Cannot index number with string "a""#,
            5,
        ),
        (
            r#"{"a":[1,2]}"#,
            r"[path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:[$b,$c]} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"a\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2]}"#,
            r"path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:[$b,$c]} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"a\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2]}"#,
            r"del(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:[$b,$c]} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2]}"#,
            r"(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:[$b,$c]} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            r#"Cannot index number with string "a""#,
            5,
        ),
    ])
}

/// #5179: Not read, and as on `main`: a computed key (`{("a"):$a}`), a destructure of a value computed from `$x`
/// (`$x.a as {a:$q}`) and a nested pattern of several entries or elements below a `null` leave the landing test
/// unread, so the earlier answer stands: jq answers with every step's path where this refuses, or drops the
/// steps under a `?`. Each row was checked equal on the merge-base of this change. Pinned so a change is seen.
/// Both routes.
///
/// #5223: the path-mode interpreter carries jq's register, so the rows the shipped resolver
/// refused here now hold jq 1.7.1's answers.
#[test]
fn test_foreach_update_try_handler_landing_shapes_the_test_does_not_read_stay_refused_5179(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1}}"#,
            r"[path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x.a as {a:$q} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"a\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x.a as {a:$q} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"a\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x.a as {a:$q} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x.a as {a:$q} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            r#"Cannot index number with string "a""#,
            5,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":1}}"#,
            r#"[path(. as $x | foreach (.,.a,.) as $i (.; try (foreach ($x as {("a"):$a} | .) as $y (0; .a?; $y)) catch $i; .))?]"#,
            "[[],[\"a\"],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":1}}"#,
            r#"path(. as $x | foreach (.,.a,.) as $i (.; try (foreach ($x as {("a"):$a} | .) as $y (0; .a?; $y)) catch $i; .))"#,
            "[]\n[\"a\"]\n[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":1}}"#,
            r#"del(. as $x | foreach (.,.a,.) as $i (.; try (foreach ($x as {("a"):$a} | .) as $y (0; .a?; $y)) catch $i; .))"#,
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":1}}"#,
            r#"(. as $x | foreach (.,.a,.) as $i (.; try (foreach ($x as {("a"):$a} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9"#,
            "",
            r#"Cannot index number with string "a""#,
            5,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":1}}"#,
            r"[path(. as $x | foreach (.a,.,.a) as $i (.; try (foreach ($x.a as {a:$q} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"a\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":1}}"#,
            r"path(. as $x | foreach (.a,.,.a) as $i (.; try (foreach ($x.a as {a:$q} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"a\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":1}}"#,
            r"del(. as $x | foreach (.a,.,.a) as $i (.; try (foreach ($x.a as {a:$q} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"c":{"b":1}}"#,
            r"(. as $x | foreach (.a,.,.a) as $i (.; try (foreach ($x.a as {a:$q} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            r#"Cannot index number with string "a""#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r#"[path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {("a"):$a} | .) as $y (0; .a?; $y)) catch $i; .))?]"#,
            "[[\"a\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r#"path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {("a"):$a} | .) as $y (0; .a?; $y)) catch $i; .))"#,
            "[\"a\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r#"del(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {("a"):$a} | .) as $y (0; .a?; $y)) catch $i; .))"#,
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r#"(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {("a"):$a} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9"#,
            "",
            r#"Cannot index number with string "a""#,
            5,
        ),
        (
            r#"{"a":{}}"#,
            r"[path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:{c:$c,d:$d}}} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"a\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{}}"#,
            r"path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:{c:$c,d:$d}}} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"a\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{}}"#,
            r"del(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:{c:$c,d:$d}}} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{}}"#,
            r"(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:{c:$c,d:$d}}} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            r#"Cannot index number with string "a""#,
            5,
        ),
        (
            r#"{"a":{}}"#,
            r"[path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:[$c,$d]}} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[\"a\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{}}"#,
            r"path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:[$c,$d]}} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[\"a\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{}}"#,
            r"del(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:[$c,$d]}} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{}}"#,
            r"(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{b:[$c,$d]}} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "",
            r#"Cannot index number with string "a""#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r#"[path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{("b"):$b}} | .) as $y (0; .a?; $y)) catch $i; .))?]"#,
            "[[\"a\"],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r#"path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{("b"):$b}} | .) as $y (0; .a?; $y)) catch $i; .))"#,
            "[\"a\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r#"del(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{("b"):$b}} | .) as $y (0; .a?; $y)) catch $i; .))"#,
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r#"(. as $x | foreach (.a,.,.a) as $i (null; try (foreach ($x as {a:{("b"):$b}} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9"#,
            "",
            r#"Cannot index number with string "a""#,
            5,
        ),
    ])
}
