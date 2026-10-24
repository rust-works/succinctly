//! #5013: a fold whose INIT navigates moves jq's register before its SOURCE runs, so a `$x` frozen where the fold
//! entered is no longer the register, and a destructure of it in SOURCE raises at its first step.

use crate::{assert_path_rows_both_routes_3749, run_jq_full};
use anyhow::Result;

/// #5013: `foreach`/`reduce` SOURCE `$x as {b:$q} | .` after INIT `.a`: jq raises `near attempt to access element "b"`
/// at the pattern's step. The by-value drive answered `["a"]`, so `del` deleted `.a` and `|= 9`/`= 9` wrote `.a`
/// where jq leaves the document and errors. Rows captured from jq 1.7.1 (stdout, first stderr line, exit code), both
/// routes.
#[test]
fn test_fold_source_foreign_destructure_after_a_navigating_init_raises_5013() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"path(. as $x | foreach ($x as {b:$q} | .) as $y (.a; .; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1},\"b\":2}",
            5,
        ),
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"del(. as $x | foreach ($x as {b:$q} | .) as $y (.a; .; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1},\"b\":2}",
            5,
        ),
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"(. as $x | foreach ($x as {b:$q} | .) as $y (.a; .; .)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1},\"b\":2}",
            5,
        ),
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"(. as $x | reduce ($x as {b:$q} | .) as $y (.a; .)) = 9",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1},\"b\":2}",
            5,
        ),
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"path(. as $x | reduce ($x as {b:$q} | .) as $y (.a; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1},\"b\":2}",
            5,
        ),
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"del(. as $x | foreach ($x as {b:$q} | .) as $y (.a.b; .; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1},\"b\":2}",
            5,
        ),
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"[path(. as $x | foreach ($x as {b:$q} | .) as $y (.a; .; .))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5013: a `try`/`?` around that destructure catches the raise and leaves the source empty, so a `foreach` emits
/// nothing and a `reduce` answers its INIT (`del` deleted `.a` where jq leaves the document); a handler that emits
/// supplies the element instead. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_source_caught_foreign_destructure_after_a_navigating_init_empties_the_source_5013(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"del(. as $x | foreach (try ($x as {b:$q} | .)) as $y (.a; .; .))",
            "{\"a\":{\"b\":1},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"path(. as $x | foreach (try ($x as {b:$q} | .)) as $y (.a; .; .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"[path(. as $x | foreach (($x as {b:$q} | .)?) as $y (.a; .; .))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"path(. as $x | foreach (try ($x as {b:$q} | .) catch 5) as $y (.a; .; .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"path(. as $x | reduce (try ($x as {b:$q} | .)) as $y (.a; .))",
            "[\"a\"]\n",
            "",
            0,
        ),
    ])
}

/// #5013: unchanged: an INIT that does not move the register (`.`, `0`), a SOURCE that navigates rather than
/// destructures (it already raised), and a `$x` bound at INIT's own node. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_source_destructure_controls_5013() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"path(. as $x | foreach ($x as {b:$q} | .) as $y (.; .; .))",
            "",
            "Invalid path expression with result {\"a\":{\"b\":1},\"b\":2}",
            5,
        ),
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"path(foreach (.b) as $y (.a; .; .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1},\"b\":2}",
            5,
        ),
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"path(. as $x | foreach ($x as {b:$q} | .) as $y (0; .; .))",
            "",
            "Invalid path expression with result 0",
            5,
        ),
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"path(.a as $x | foreach (.) as $y (.a; .; $x))",
            "[\"a\"]\n",
            "",
            0,
        ),
    ])
}

/// #5013: an INIT on a later fork (`(., .a)`, `(.a,.b)`), and an INIT that lands on a `null` below the root, move
/// the register too, and a caught destructure of a foreign `$x` empties the source there as well. Each was read by
/// value and wrote (`del` of the whole document, `|= 9` of `.b`/`.c`). Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_source_foreign_destructure_after_a_later_or_null_init_raises_5013() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | reduce ($x as {b:$q} | .) as $y ((., .a); .))",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...",
            5,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"(. as $x | foreach (try ($x as {b:$q} | .)) as $y ((.a,.b); .; .)) |= 9",
            "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach (try ($x as {b:$q} | .)) as $y ((.b,.a); .; .))",
            "{\"a\":{\"b\":1,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(.a as $x | reduce ($x as {b:$b} | .) as $y (.c; .)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"b\":1}",
            5,
        ),
        (
            r#"{"a":{"b":1},"c":null}"#,
            r"(. as $x | reduce ($x as {b:$b} | .) as $y (.c; .)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1},\"c\":null}",
            5,
        ),
    ])
}

/// #5013: a `?//` chain of destructures of a foreign `$x` after a navigating INIT raises too, so no write goes
/// through (it was read by value, and `del` deleted `.a`). jq names the last alternative's step (`element 0`), the
/// resolver the first's: the exit code and the empty output agree, the wording does not.
#[test]
fn test_fold_source_alternative_chain_after_a_navigating_init_refuses_5013() -> Result<()> {
    for (input, filter) in [
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach ($x as {b:$q} ?// [$q] | .) as $y (.a; .; .))",
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | reduce ($x as {b:$q} ?// [$q] | .) as $y (.a; .))",
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(. as $x | foreach ($x as [$q] ?// {b:$q} | .) as $y (.a; .; .))",
        ),
        (
            r#"{"a":{"a":1}}"#,
            r"(. as $x | foreach ($x as {a:$q} ?// [$q] | 1) as $y ((.a,.b); .b?)) = 9",
        ),
        (
            r#"{"a":{"b":null},"b":null}"#,
            r"((. as $x | foreach ($x as {a:$q} ?// [$q] | 1) as $y ((.a,.b); .b?))) |= 9",
        ),
    ] {
        for (stdout, stderr, code) in [
            run_jq_full(&["-c", filter], Some(input))?,
            run_jq_full(&["-nc", &format!("{input} | {filter}")], None)?,
        ] {
            assert_eq!(
                (stdout.as_str(), code),
                ("", 5),
                "{input} | {filter}: stderr {stderr:?}"
            );
            assert!(
                stderr.contains("Invalid path expression near attempt to access element"),
                "{input} | {filter}: {stderr:?}"
            );
        }
    }
    Ok(())
}

/// #5013: inside another fold's body an INIT that lands on a `null` below the root keeps the live `null` on the fold's
/// own frame, as before: the register's frame is not used there. Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_source_null_init_inside_a_fold_body_unchanged_5013() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1}"#,
            r"path(foreach (1) as $o (.; reduce (.a) as $y (.c; .); .))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":1}",
            5,
        ),
        (
            r#"{"a":1}"#,
            r"[path(foreach (1,2) as $o (.; foreach (.b?) as $y (.c; .; .); .))]",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":1}",
            5,
        ),
        (
            r#"{"a":1}"#,
            r"path(reduce (1) as $o (.; reduce (1) as $y (.c; .)))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":1}"#,
            r"[path(foreach (1) as $o (.; foreach (1) as $y (.c; .; .); .))]",
            "[[\"c\"]]\n",
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
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5013() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r#"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.a?; .; error("x"))) catch (.); $i))?]"#,
            "[[\"a\"],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":true}}"#,
            r"(. as $x | foreach (.,.a) as $i (.; (try (foreach ($x as {a:$a} | .) as $y (.a?; empty; error($x))) catch null)?; $i)) = 9",
            "",
            "Cannot index number with string \"a\"",
            5,
        ),
    ])
}
