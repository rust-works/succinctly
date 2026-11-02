//! #5203: rows the path-mode interpreter (#5223) answers as jq 1.7.1 does.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #5223 Phase 3: this issue's repro rows, with `path`, `del` and `|=`
/// wherever the row is a `path(...)`, as jq 1.7.1 answers them (stdout, the
/// first error message, the exit code). The path-mode interpreter carries
/// jq's register, so they match by construction, on both routes.
#[test]
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5203() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#, r"del(.a as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | $q) as $y (.a; .c?; .); .))", "", "Invalid path expression near attempt to access element \"c\" of {\"b\":1,\"c\":{\"b\":3}}", 5),
        (r#"{"n":null,"m":null}"#, r"path(.n as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | $q) as $y (.m; .; .); .))", "[\"m\",\"b\"]\n", "", 0),
        (r#"{"n":null,"m":null}"#, r"del(.n as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | $q) as $y (.m; .; .); .))", "{\"n\":null,\"m\":null}\n", "", 0),
        (r#"{"n":null,"m":null}"#, r"(.n as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | $q) as $y (.m; .; .); .)) |= 9", "{\"n\":null,\"m\":{\"b\":9}}\n", "", 0),
        (r#"{"a":{"c":null},"b":{"b":4}}"#, r"path(.b as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | .) as $y (.b; .; .); .))", "", "Invalid path expression with result {\"b\":4}", 5),
        (r#"{"a":{"c":null},"b":{"b":4}}"#, r"del(.b as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | .) as $y (.b; .; .); .))", "", "Invalid path expression with result {\"b\":4}", 5),
        (r#"{"a":{"c":null},"b":{"b":4}}"#, r"(.b as $x | foreach 1 as $i (.; foreach ($x as {b:$q} | .) as $y (.b; .; .); .)) |= 9", "", "Invalid path expression with result {\"b\":4}", 5),
    ])
}
