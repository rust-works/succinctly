//! #5178: a value head (`not |`, `1 |`) ahead of a nested `foreach` in a `foreach` UPDATE whose source moved the register onto a
//! `null` element.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// The head hands the nested `foreach` a value that is no node of the register (a boolean is never `jv_identical` to a `null`
// register). The `$x` bound at the root is foreign to it, so its pattern's first step raises and the `try` runs its handler,
// whose output the end of the path refuses. The resolver saw an untracked ambient and read no `$x` as foreign, so the source
// drove by value, the nested `foreach` emitted nothing and `path`/`del`/`|=` answered where jq raises.

/// #5178: the issue's rows and their `1 |` and `"q" |` twins, over `path`, `del` and `|=`. Every row captured from
/// `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5178_value_head_ahead_of_a_nested_foreach_on_a_null_register() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r#"path(. as $x | foreach (.a) as $i (null; not | try (foreach ($x as {a:$a} | .) as $y ("s"; empty; .)) catch $x; .))"#,
            "",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r#"del(. as $x | foreach (.a) as $i (null; not | try (foreach ($x as {a:$a} | .) as $y ("s"; empty; .)) catch $x; .))"#,
            "",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r#"path(. as $x | foreach (.a) as $i (null; 1 | try (foreach ($x as {a:$a} | .) as $y ("s"; empty; .)) catch $x; .))"#,
            "",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r#"del(. as $x | foreach (.a) as $i (null; 1 | try (foreach ($x as {a:$a} | .) as $y ("s"; empty; .)) catch $x; .))"#,
            "",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r#"path(. as $x | foreach (.a) as $i (null; "q" | try (foreach ($x as {a:$a} | .) as $y ("s"; empty; .)) catch $x; .))"#,
            "",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r#"del(. as $x | foreach (.a) as $i (null; "q" | try (foreach ($x as {a:$a} | .) as $y ("s"; empty; .)) catch $x; .))"#,
            "",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r#"(. as $x | foreach (.a) as $i (null; not | try (foreach ($x as {a:$a} | .) as $y ("s"; empty; .)) catch $x; .)) |= 1"#,
            "",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r#"[path(. as $x | foreach (.a) as $i (null; not | try (foreach ($x as {a:$a} | .) as $y ("s"; empty; .)) catch $x; .))?]"#,
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5178: the issue's two-step row. The root element answers `["a"]`, and the `null` element's step raises at its
/// handler's `$x.a`, a `$x` that is not the register.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5178_two_step_source_raises_on_the_null_element() -> Result<()> {
    assert_path_rows_both_routes_3749(&[(
        r#"{"a":null}"#,
        r"path(. as $x | foreach (.,.a) as $i (0; not | try (foreach ($x as {a:$a,b:$b} | .) as $y (.; empty; .a)) catch $x.a; .))",
        "[\"a\"]\n",
        "Invalid path expression near attempt to access element \"a\" of {\"a\":null}",
        5,
    )])
}
