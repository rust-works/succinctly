//! #5209: a nested one-entry pattern in the source of a caught `foreach` in a `foreach` UPDATE (`$x as {a:{c:$b}} | .`) that
//! lands on a `null` child. The step's state is the `null` an emptied step left, which is `jv_identical` by kind to the
//! `null` the pattern's last access reaches, so the pattern's `.` body emits it *on* jq's register and the nested
//! `foreach`'s UPDATE raises a path error the step's `catch $i` handles. The flat pattern (`{c:$b}`) was read this way
//! since #4800; the nested one was not, so the step was counted as empty and the run lost the path the handler emits.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #5209: The issue's rows, the writes over them, a deeper and an array pattern, and the same landing on a `false`/`true`
/// child or state. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_try_handler_nested_pattern_landing_on_a_null_child_5209() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.,.a,.) as $i (0; try (foreach (try ($x as {a:{c:$b}} | .)) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.,.a,.) as $i (0; try (foreach (try ($x as {a:{c:$b}} | .)) as $y (0; .a?; $y)) catch $i; .))",
            "[]\n[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (0; try (foreach (try ($x as {a:{c:$b}} | .)) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.,.a) as $i (0; try (foreach (try ($x as {a:{c:$b}} | .)) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.,.a,.) as $i (0; try (foreach (try ($x as {a:{c:$b}} | .)) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.,.a,.) as $i (0; try (foreach (try ($x as {a:{c:$b}} | .)) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":{"c":null}}"#,
            r"try path(. as $x | foreach (.a,.,.a) as $i (null; try (foreach (try ($x as {a:{c:$b}} | .)) as $y (null; .a?; $y)) catch $i; .)) catch 5",
            "[\"a\",\"c\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.,.a,.) as $i (0; try (foreach (try ($x as [[$b]] | .)) as $y (0; .a?; $y)) catch $i; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.,.a,.) as $i (0; try (foreach (try ($x as {a:{c:{d:$b}}} | .)) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":{"a":null}}}"#,
            r#"[path(. as $x | foreach (.,.a.a,.) as $i (null; try (foreach (try ($x as {a:{c:{d:$b}}} | .)) as $y (.; 1; $y)) catch "c"; .))?]"#,
            "[[\"a\",\"c\",\"d\"],[\"a\",\"c\",\"d\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"c":false}}"#,
            r#"try path(. as $x | foreach (.,.a) as $i (true; not | try (foreach (try ($x as {a:{c:$b}} | .)) as $y (.; .; $y)) catch "c"; .)) catch 5"#,
            "[\"a\",\"c\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":true}}"#,
            r#"[path(. as $x | foreach (.,.a) as $i (null; try (foreach (try ($x as {c:{d:$b}} | .)) as $y (.; 1; $y)) catch "c"; .))?]"#,
            "[[\"c\",\"d\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":false}}"#,
            r"(. as $x | foreach (.[0]?,.) as $i (0; not | try (foreach (try ($x as {a:{a:$b}} | $b)) as $y (null; .a; $i)) catch $x; .)) = 9",
            "9\n",
            "",
            0,
        ),
    ])
}
