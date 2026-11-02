//! #5106: an `INIT null` `foreach` whose EXTRACT is a caught foreign-`$x` `foreach`, alone, behind a `try`/`?`, or the live
//! arm of an `if` whose other arm emits nothing. On the step where `$x` is jq's register (the source's element `.`), the
//! nested `foreach`'s destructure passes, jq's register moves to the bound child, and its emission is made there:
//! `path(..)` answers the child's path. The binary answered the root's `null` and refused it. On a step where `$x` is
//! not the register the destructure raises, the `try` swallows it, and the `foreach` is empty.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// `FE = foreach (try ($x as {a:$a} | .)) as $y (.; .; .)`, `Q = . as $x | foreach SRC as $i (null; .; EXTRACT)`.

/// #5106: the issue's rows on `{"a":null,"b":2}` (the sources `.`, `(.,.)`, `(.a,.)`, `(.,.a)`, `(.b,.)`, `(.a,.a)`,
/// `(.,.,.)`, and `(.,.b)`/`(.,.[])` whose later steps are not the register), the EXTRACT spelt bare, behind a `try` or
/// `?`, and as the live arm of an `if` (`else empty`, `then empty`), in `[path(..)?]`, `path`, `try .. catch`, `del`,
/// `|=` and `=`; INIT `false`; the `[$a]` pattern, which does not pass on an object; and the rows that must stay as they
/// are: an `if` that is false, INIT `{}`/`.a`, and a `$x` that is not the register (`.a as $x`, `.nope as $x`, the
/// source `.a` or `(.a,.b)`). Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5106_extract_caught_bind_foreach_restates_at_the_bound_child() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach . as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach (.,.) as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach (.a,.) as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach (.,.a) as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach (.b,.) as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach (.a,.a) as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach (.,.,.) as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[[\"a\"],[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach (.,.b) as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach (.,.[]) as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r#"try path(. as $x | foreach (.,.b) as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end)) catch "C""#,
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach (.,.b) as $i (false; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r#"try path(. as $x | foreach (.,.[]) as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end)) catch "C""#,
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach (.,.[]) as $i (false; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r#"try path(. as $x | foreach (.,.a) as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end)) catch "C""#,
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach (.,.a) as $i (false; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"path(. as $x | foreach . as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"path(. as $x | foreach . as $i (null; .; foreach (try ($x as {a:$a} | .)) as $y (.; .; .)))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"path(. as $x | foreach . as $i (null; .; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .))))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach . as $i (null; .; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .))?))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach . as $i (null; .; if false then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach . as $i (null; .; if . then empty else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach (.,.,.) as $i (0; null; if . then empty else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end))?] | length",
            "3\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r#"try path(. as $x | foreach (.,.) as $i (null; .; foreach (try ($x as {a:$a} | .)) as $y (.; .; .))) catch "C""#,
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"del(. as $x | foreach . as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))",
            "{\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"(. as $x | foreach . as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end)) |= 9",
            "{\"a\":9,\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"(. as $x | foreach . as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end)) = 9",
            "{\"a\":9,\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach . as $i (null; .; if true then foreach (try ($x as [$a] | .)) as $y (.; .; .) else empty end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[5],"b":2}"#,
            r"[path(. as $x | foreach . as $i (null; .; if true then foreach (try ($x as [$a] | .)) as $y (.; .; .) else empty end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[5],"b":2}"#,
            r"[path(. as $x | foreach . as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach . as $i ({}; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach . as $i (.a; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(.a as $x | foreach . as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(.a as $x | foreach (.,.a) as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(.nope as $x | foreach . as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach .a as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach (.a,.b) as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5106 / #5126: the residual rows, once ignored as known gaps and answered as jq answers since the path-mode interpreter
/// (#5223) carries jq's register: INIT `.`, an UPDATE that stores `$x`, and a source `$i` over a `false` element. Captured
/// from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5106_former_known_gaps_match_jq() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach . as $i (.; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else empty end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach . as $i ({}; $x; foreach (try ($x as {a:$a} | .)) as $y (.; .; .)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"path(. as $x | foreach . as $i ({}; $x; foreach (try ($x as {a:$a} | .)) as $y (.; .; .)))",
            "",
            "Invalid path expression with result {\"a\":null,\"b\":2}",
            5,
        ),
        (
            r#"{"a":false}"#,
            r"path(. as $x | foreach (.,.a) as $i (0; false; try (foreach ($i as {a:$a} | .) as $y (.; .; .))))",
            "[\"a\"]\n",
            "",
            0,
        ),
    ])
}
