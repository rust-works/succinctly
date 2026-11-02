//! #5225: a nested `foreach` whose source destructures the register `$x`, caught or alt-wrapped, answers as jq does.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #5223 Phase 3: this issue's repro rows, with `path`, `del` and `|=`
/// wherever the row is a `path(...)`, as jq 1.7.1 answers them (stdout, the
/// first error message, the exit code). The path-mode interpreter carries
/// jq's register, so they match by construction, on both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5225() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach 1 as $i (.a; foreach (try ($x as {b:$q} | $q)) as $y (.; $y; .); .))",
            "[\"a\",\"b\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(.a as $x | foreach 1 as $i (.a; foreach (try ($x as {b:$q} | $q)) as $y (.; $y; .); .))",
            "{\"a\":{\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"(.a as $x | foreach 1 as $i (.a; foreach (try ($x as {b:$q} | $q)) as $y (.; $y; .); .)) |= 9",
            "{\"a\":{\"b\":9,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach 1 as $i (.; (.a | foreach (try ($x as {b:$q} | $q)) as $y (.; $y; .)); .))",
            "[\"a\",\"b\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(.a as $x | foreach 1 as $i (.; (.a | foreach (try ($x as {b:$q} | $q)) as $y (.; $y; .)); .))",
            "{\"a\":{\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"(.a as $x | foreach 1 as $i (.; (.a | foreach (try ($x as {b:$q} | $q)) as $y (.; $y; .)); .)) |= 9",
            "{\"a\":{\"b\":9,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach 1 as $i (.; foreach (try ($x as {b:$q} | $q) // empty) as $y (.a; $y; .); .))",
            "[\"a\",\"b\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(.a as $x | foreach 1 as $i (.; foreach (try ($x as {b:$q} | $q) // empty) as $y (.a; $y; .); .))",
            "{\"a\":{\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"(.a as $x | foreach 1 as $i (.; foreach (try ($x as {b:$q} | $q) // empty) as $y (.a; $y; .); .)) |= 9",
            "{\"a\":{\"b\":9,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach 1 as $i (.a; foreach ($x as {b:$q} | $q) as $y (.; $y; .); .))",
            "[\"a\",\"b\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(.a as $x | foreach 1 as $i (.a; foreach ($x as {b:$q} | $q) as $y (.; $y; .); .))",
            "{\"a\":{\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"(.a as $x | foreach 1 as $i (.a; foreach ($x as {b:$q} | $q) as $y (.; $y; .); .)) |= 9",
            "{\"a\":{\"b\":9,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"path(.a as $x | foreach 1 as $i (.; (.a | foreach ($x as {b:$q} | $q) as $y (.; $y; .)); .))",
            "[\"a\",\"b\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"del(.a as $x | foreach 1 as $i (.; (.a | foreach ($x as {b:$q} | $q) as $y (.; $y; .)); .))",
            "{\"a\":{\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#,
            r"(.a as $x | foreach 1 as $i (.; (.a | foreach ($x as {b:$q} | $q) as $y (.; $y; .)); .)) |= 9",
            "{\"a\":{\"b\":9,\"c\":{\"b\":3}},\"b\":2}\n",
            "",
            0,
        ),
    ])
}
