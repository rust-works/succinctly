//! #5034: a `foreach` UPDATE `if C then FE else B end` (`B` other than `.`) whose then-branch is the caught foreign-`$x`
//! `foreach` `FE = foreach (try ($x as {a:$a} | .)) as $y (.; .; .)`, under an EXTRACT that emits nothing.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #5034: on `{"a":1,"b":2}`, the first step (register on `["a"]`) takes the `then` branch, where `$x` is the root and
/// not the register: `FE`'s pattern step raises, the `try` catches it and the step emits nothing, which leaves jq's
/// state `null`. The second step takes the `else` branch and indexes that `null`, which jq refuses
/// (`near attempt to access element "a" of null`, exit 5). `succinctly` read the `if` as one whose `else` is not `.`
/// and so not an UPDATE that is empty with `FE`: `FE` was driven by value, emitted, and the state stayed the root
/// (`path` printed nothing, `del` and `|= 9` echoed the document). Every row captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_if_with_a_navigating_else_empties_with_the_caught_foreach_5034() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; empty))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"del(. as $x | foreach (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; empty))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"(. as $x | foreach (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; empty)) |= 9",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach (.a,.b) as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; empty))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach .[] as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; empty))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach .[]? as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; empty))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; (.a)?))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach (.a,.a) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; empty))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .b end; empty))",
            "",
            r#"Invalid path expression near attempt to access element "b""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach (.a,.b) as $i (.; if length > 0 then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) elif .b then 1 else .a end; empty))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach (.a,.b) as $i (.; if . then empty else .a end; empty))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach (.a,.b) as $i (.; if . then try error(1) else .a end; empty))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach (.a,.b) as $i (.; if . then foreach (try ($x as [$a] | .)) as $y (.; .; .) else .a end; empty))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else (.a,.b) end; empty))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else limit(1; .[]) end; empty))",
            "",
            r"Invalid path expression near attempt to iterate through null",
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach (.a,.b) as $i (.; if . then foreach ($x as {a:$a} | .) as $y (.; .; .) else .a end; empty))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
    ])
}

/// #5034: the rows that keep their answer. An `else` that is `empty`, an `error`, a `foreach` whose pattern
/// raises on the type (`[$a]`), a `null`/`false`/`{"a":null}` document (the first state is falsy and takes the `else`) were already read; an `elif` chain whose later arms hold a `foreach` of their own keeps its reading (the `any_subexpr` exclusion in `emptied_update`); and when a later step reaches the register (`(.a,.)`, the root as
/// the second element) the bind passes there and `FE` emits, so `if true then FE else .a end` is `FE` (the `else`
/// never runs) and answers as the bare `FE` does, where reading the `else` left that step refused. Every row captured
/// from jq 1.7.1, both routes.
#[test]
fn test_foreach_update_if_with_a_navigating_else_keeps_the_rows_that_matched_5034() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1,"b":2}"#,
            r"[path(. as $x | foreach (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; (.a)?))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r"{}",
            r"[path(. as $x | foreach (.a,.) as $i (null; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; null))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r"{}",
            r"[path(. as $x | foreach (.a,.) as $i (null; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r"{}",
            r"path(. as $x | foreach (.a,.) as $i (null; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; null))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (null; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) elif .b then .a else .a end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r#"path(. as $x | foreach (.a,.b) as $i (.; if "x" then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; empty))"#,
            "",
            "",
            0,
        ),
        (
            r"null",
            r"[path(. as $x | foreach (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r"false",
            r"[path(. as $x | foreach (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach (.a,.b) as $i (.; if 1 then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else (foreach (1,2) as $k (.; .; .)) end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (null; if . then try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) elif .b then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end; .a)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
    ])
}

/// #5034, #5044: where the `if` is still not read, the answer is what it was (jq's is in each comment). A `foreach`
/// EXTRACT keeps its reading of `if . then FE else .a end` ([`EmptiedUpdate::else_beyond_dot`] is not honoured there):
/// jq prints nothing, succinctly refuses naming the state. The mirrored `if C then B else FE end` is read only in a
/// `reduce` UPDATE ([`EmptiedUpdate::mirrored`]): in a `foreach` UPDATE it answers `"C"` under a `try` where jq's
/// `del` leaves the document, because the step after an emptied one cannot be placed at the register the bind moved
/// (7 of 12,000 grid rows that matched jq turned into refusals when it was read). Recorded in `limitations.md`; these
/// rows move when those readings do.
///
/// #5223: the path-mode interpreter carries jq's register, so the rows the shipped resolver
/// refused here now hold jq 1.7.1's answers.
#[test]
fn test_foreach_update_if_with_a_navigating_else_keeps_the_extract_and_mirrored_foreach_reading_5034(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach (.a,.b) as $i (.; .; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r#"try del(. as $x | foreach (null,.a) as $i (.; if .a then .b else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end; .a)) catch "C""#,
            "{\"a\":1}\n",
            "",
            0,
        ),
    ])
}
