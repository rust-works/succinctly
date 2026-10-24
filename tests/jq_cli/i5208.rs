//! #5208: a nested `foreach` source that destructures an outer `$x` without raising, on a step whose state is the register.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;
#[cfg(not(feature = "unshared-containers"))]
use crate::run_jq_stdin_streams;

// #5177 let the resolver decide a step whose state is the register and whose pattern RAISES, which is jq's own
// catchable error whichever node `$x` is. A pattern that does not raise passes `path_intact` only where `$x` *is* the
// register's node, and the destructure was refused as the resolver's guess nothing catches
// (`Invalid path expression with result <document>`), so `del`/`|=` failed where jq writes. A `$x` frozen from `.` itself
// ([`Origin::Snapshot`]) is certified by value equality, which no other node of the register's tree can have, so the
// resolver now decides those. Any other marker of an equal value may be a copy, and a handler that hands on a rebuilt state
// keeps the refusal.

/// #5208: the issue's four forms over `{"a":{"a":null}}`, the same `del` with the other INITs (the INIT was never the
/// cause), and a pattern that lands on a child of an array and of an object. Every row captured from `/usr/bin/jq` 1.7.1,
/// both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5208_nested_foreach_destructure_that_passes_on_a_state_that_is_the_register() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.,.) as $i (0; try (foreach ($x as {a:$q} | .) as $y (0; .a?; $y)) catch $i; .))?]",
            "[[],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.,.) as $i (0; try (foreach ($x as {a:$q} | .) as $y (0; .a?; $y)) catch $i; .))",
            "[]\n[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.,.) as $i (0; try (foreach ($x as {a:$q} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.,.) as $i (0; try (foreach ($x as {a:$q} | .) as $y (0; .a?; $y)) catch $i; .)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.,.) as $i (null; try (foreach ($x as {a:$q} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.,.) as $i (.; try (foreach ($x as {a:$q} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.,.) as $i ([]; try (foreach ($x as {a:$q} | .) as $y (0; .a?; $y)) catch $i; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"[{"a":1}]"#,
            r"(. as $x | foreach (.,.) as $i (1; try (foreach (try ($x as [$q] | .)) as $y (.; .a?; $y)) catch ($i); .)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"[{"a":1}]"#,
            r"[path(. as $x | foreach (.,.,.) as $i (.; try (foreach (($x as [$q] | .)?) as $y (0; .a?; $y)) catch ($i); .))?]",
            "[[],[],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":1}}"#,
            r"path(. as $x | foreach (.,.) as $i (0; try (foreach ($x as {a:{a:$b}} | .) as $y (0; .a?; $y)) catch ($x); .))",
            "[]\n[]\n",
            "",
            0,
        ),
    ])
}

/// #5208: the shapes that must keep their refusal. A handler that hands on a rebuilt copy of the root (`tojson|fromjson`,
/// `[$i][0]`) is not `$x`'s node, so jq's step fails `path_intact` and the resolver must not read it as passing.
///
/// #5223: the path-mode interpreter carries jq's register, so the rows the shipped resolver
/// refused here now hold jq 1.7.1's answers.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5208_a_rebuilt_state_is_not_the_node_x_is() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.,.) as $i (0; try (foreach ($x as {a:$q} | .) as $y (0; .a?; $y)) catch ($i|tojson|fromjson); .)) |= 9",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.,.) as $i (0; try (foreach ($x as {a:$q} | .) as $y (0; .a?; $y)) catch ([$i][0]); .)) |= 9",
            "",
            "Invalid path expression near attempt to access element 0",
            5,
        ),
    ])
}

/// #5208: a handler that navigates (`catch ($i | .a?)`) kept the earlier refusal: its step is followed by one the resolver
/// placed wrongly (#5213). The path-mode interpreter (#5223) raises jq's own error (`Cannot index number with string "a"`).
#[cfg(not(feature = "unshared-containers"))]
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5208_a_navigating_handler_raises_as_jq_does() -> Result<()> {
    let (stdout, stderr, code) = run_jq_stdin_streams(
        r"((. as $x | foreach (.,.,.a) as $i (.; try (foreach ($x as {a:$q} | .) as $y (0; .a?; $y)) catch ($i | .a?); .))?) |= 9",
        r#"{"a":{"a":null},"c":1}"#,
        &["-c"],
    )?;
    assert_eq!((stdout.as_str(), code), ("", 5));
    assert!(
        stderr.contains(r#"Cannot index number with string "a""#),
        "{stderr}"
    );
    Ok(())
}
