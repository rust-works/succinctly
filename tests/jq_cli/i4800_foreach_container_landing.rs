//! #4800: the caught foreign-`$x` `foreach` of a `foreach` UPDATE over a moving source, where the pattern's step lands on
//! a container. `FE` is `foreach (try ($x as {a:$a} | .)) as $y (.; .; .)`.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #4800: on `{"a":{"a":null}}` the step whose element is the root moves jq's register onto `{"a":null}` and leaves
/// the `null` state, so a `.a`/`(.a)?`/`.[0]` EXTRACT raises jq's own catchable `near attempt to access` error: a `?`,
/// a `try` and a `del(..)?` catch it. It raised the same text uncatchably (a guessed refusal of a register lost
/// anywhere in the root, which holds a `null`). Rows captured from jq 1.7.1 (stdout, first stderr line, exit code),
/// both routes.
#[test]
fn test_caught_foreign_foreach_landing_on_an_object_navigating_extract_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a)?))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a)?))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a)?)) catch "c""#,
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a)?))",
            "{\"a\":{\"a\":null}}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a)?))?]",
            "[{\"a\":{\"a\":null}}]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a)?)) |= 9",
            "{\"a\":{\"a\":null}}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .[0]))",
            "",
            "Invalid path expression near attempt to access element 0 of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .[0]))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .[0])) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .[0]))",
            "",
            "Invalid path expression near attempt to access element 0 of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .[0]))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .[0])) |= 9",
            "",
            "Invalid path expression near attempt to access element 0 of null",
            5,
        ),
    ])
}

/// #4800: the same with the pattern landing on an array (`{"a":[1]}`). The uncaught message is jq's own, as it was;
/// what is pinned is that it is still that message once the register is placed on the array.
#[test]
fn test_caught_foreign_foreach_landing_on_an_array_navigating_extract_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":[1]}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":[1]}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":[1]}"#,
            r"[del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r"(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":[1]}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a)?))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a)?))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a)?)) catch "c""#,
            "",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a)?))",
            "{\"a\":[1]}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r"[del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a)?))?]",
            "[{\"a\":[1]}]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r"(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a)?)) |= 9",
            "{\"a\":[1]}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .[0]))",
            "",
            "Invalid path expression near attempt to access element 0 of null",
            5,
        ),
        (
            r#"{"a":[1]}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .[0]))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .[0])) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .[0]))",
            "",
            "Invalid path expression near attempt to access element 0 of null",
            5,
        ),
        (
            r#"{"a":[1]}"#,
            r"[del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .[0]))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r"(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .[0])) |= 9",
            "",
            "Invalid path expression near attempt to access element 0 of null",
            5,
        ),
    ])
}

/// #4800: with a `.`, `$i` or `empty` EXTRACT the same fold answers as before: the terminal refusal names the
/// emitted `null` or `$i`, and an EXTRACT that emits nothing has no output to check.
#[test]
fn test_caught_foreign_foreach_landing_on_a_container_other_extracts_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))",
            "",
            "Invalid path expression with result {\"a\":{\"a\":null}}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))",
            "",
            "Invalid path expression with result {\"a\":{\"a\":null}}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); empty))",
            "{\"a\":{\"a\":null}}\n",
            "",
            0,
        ),
    ])
}

/// #4800: the same `foreach` behind the shapes that already read it for a `null` landing: a `null` or `not` head
/// (the `foreach` is handed a `null` or boolean, never the container), a `.` stage, a `label`, `first`, `try` and an
/// emits-nothing comma sibling, with a `.a` EXTRACT on `{"a":{"a":null}}`.
#[test]
fn test_caught_foreign_foreach_landing_on_a_container_behind_wrappers_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; null | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; null | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; null | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; not | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of true",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; not | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; not | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of true",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) | .; .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) | .; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) | .; .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; label $l | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; first(foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; first(foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; first(foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty; .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), empty; .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
    ])
}

/// #4800: a `null` landing, as before (`["a","a"]`, and a two-entry pattern whose absent `b` lands on `null` too); an
/// empty object landing (`{"a":{}}`), read as any container; and a `$i.a` EXTRACT, whose error names the element
/// (`$i` is the root there), caught under `?` now that the register is placed.
#[test]
fn test_caught_foreign_foreach_landing_null_empty_and_element_extract_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a,b:$b} | .)) as $y (.; .; .); .a))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a,b:$b} | .)) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i.a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i.a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i.a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":{\"a\":null}}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i.a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{},"b":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{},"b":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{},"b":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i.a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":{},\"b\":null}",
            5,
        ),
        (
            r#"{"a":{},"b":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i.a))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #4800: a `not` head hands the `foreach` a boolean (`true`, from the `null` the emptied step left), which is never the
/// container the pattern lands on either: with a `$i.a` EXTRACT, which raises on the element, `[path(..)?]` catches it
/// as jq does. Read for a `null` state only, it raised uncatchably.
#[test]
fn test_caught_foreign_foreach_landing_on_a_container_behind_a_boolean_head_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; not | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i.a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":{\"a\":null}}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; not | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i.a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; not | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i.a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":{\"a\":null}}",
            5,
        ),
        (
            r#"{"a":[1]}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; not | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i.a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":[1]}",
            5,
        ),
        (
            r#"{"a":[1]}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; not | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i.a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; not | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i.a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":[1]}",
            5,
        ),
    ])
}

/// #4800: a `try` around the `foreach` whose handler emits nothing (`catch empty`) has no output to misplace, so the
/// container landing is read through it as through a bare `try`.
#[test]
fn test_caught_foreign_foreach_landing_on_a_container_behind_a_catch_empty_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) catch empty; .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) catch empty; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) catch empty; .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) catch empty; .a)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
    ])
}

/// #4800, #5004: a handler that emits is read through too. jq runs it after backtracking to its `try`, so its output
/// carries the register the `try` entered with: the root here, so jq's `$i` is `[]` and `[path(..)?]` is `[[]]`.
/// #4800 kept this shape refused (`has_emitting_handler`), since the container reading did not state that register;
/// #5004 seeds the handler with it (`foreach_update_try_entry`), so the rows match jq.
#[test]
fn test_caught_foreign_foreach_landing_behind_an_emitting_handler_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .a)) catch (.); $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .a)) catch (.); $i))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .a)) catch (.); $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .a)) catch (.); $i)) |= 9",
            "9\n",
            "",
            0,
        ),
    ])
}
