//! #5112: a later step of a `foreach` UPDATE comma whose state makes an `if` condition raise.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// `FE = foreach (try ($x as {a:$a} | .)) as $y (.; .; .)`, `Q = . as $x | foreach (.a,.a) as $i (.; (7, if .a then FE
// else 1 end); $i)`. Step 1 ends on the `1` the `else` stores; step 2 runs `7` and then `.a` of that `1` raises. jq
// has already emitted `7`'s path by then, so `[path(Q)?]` is three paths, where the whole comma was resolved and
// refused before `7` emitted (two).

/// #5112: the issue's row on `{"a":null}` in `[path(..)?]`; the raising `if` as the first item (nothing emits ahead of
/// it) and a three-element source (the raise comes on step 2 of 3). On both routes. Captured from `/usr/bin/jq` 1.7.1.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5112_foreach_update_comma_later_step_whose_if_condition_raises() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.a) as $i (.; (7, if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else 1 end); $i))?]",
            "[[\"a\"],[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.a) as $i (.; (if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else 1 end, 7); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.a,.a) as $i (.; (7, if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else 1 end); $i))?]",
            "[[\"a\"],[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
    ])
}
