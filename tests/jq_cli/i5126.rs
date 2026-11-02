//! #5126: the caught-bind `foreach` EXTRACT of #5106 behind an `if` with a constant condition whose other arm is not `empty`.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// #5106 restated an EXTRACT that is the caught foreign-`$x` `foreach` `FE` (alone, or the live arm of an `if` whose other arm
// emits nothing). A constant condition names the live arm just as well, whatever the other arm holds
// (`if true then FE else . end`): jq runs only that arm, so the `else .` never emits.

/// #5126 row 1: the issue's row and its siblings (`false`, `null` and parenthesised conditions, a live arm that is `empty`, the fold in the `else`, `path`, `[path()?]`, `del`),
/// captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5126_constant_condition_names_the_live_extract_arm() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach . as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"path(. as $x | foreach . as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach . as $i (null; .; if false then . else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"del(. as $x | foreach . as $i (null; .; if true then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else 1 end))",
            "{\"b\":2}\n",
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
            r"[path(. as $x | foreach . as $i (null; .; if true then empty else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach . as $i (null; .; if (true) then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | foreach . as $i (null; .; if null then . else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
    ])
}
