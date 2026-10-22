//! #5042: a fold source's foreign `$x` destructure after an INIT that navigated and then computed.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #5042: INIT `$r | .a | tostring` navigates jq's register to `.a` and then computes, behind a computed stage
/// (`null |`, `1 |`) or on a fold entered at the document. jq's destructure of `$x` (bound at `.a`, the register's own
/// node) passes its step and the path ends `with result` INIT's string; SOURCE ran on a frame that named no register
/// and so reported the pattern's access (`near attempt to access element "b"`). An INIT that lands on a `null`
/// is not read (jq's register is identical to every `null`). Rows captured from jq 1.7.1, both routes.
#[test]
fn test_fold_source_destructure_after_a_navigate_then_compute_init_names_the_path_end_5042(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (r#"{"a":{"b":{"c":3}},"b":{"c":4}}"#, r"path(. as $r | .a as $x | null | foreach ($x as {b:$q} | 1) as $y ($r | .a | tostring; .))", "", "Invalid path expression with result \"{\\\"b\\\":{\\\"c\\\":3}}\"", 5),
        (r#"{"a":{"b":{"c":3}},"b":{"c":4}}"#, r"del(. as $r | .a as $x | null | foreach ($x as {b:$q} | 1) as $y ($r | .a | tostring; .))", "", "Invalid path expression with result \"{\\\"b\\\":{\\\"c\\\":3}}\"", 5),
        (r#"{"a":{"b":{"c":3}},"b":{"c":4}}"#, r"(. as $r | .a as $x | null | foreach ($x as {b:$q} | 1) as $y ($r | .a | tostring; .)) |= 9", "", "Invalid path expression with result \"{\\\"b\\\":{\\\"c\\\":3}}\"", 5),
        (r#"{"a":{"b":{"c":3}},"b":{"c":4}}"#, r"path(. as $r | .a as $x | null | reduce ($x as {b:$q} | 1) as $y ($r | .a | tostring; .))", "", "Invalid path expression with result \"{\\\"b\\\":{\\\"c\\\":3}}\"", 5),
        (r#"{"a":{"b":{"c":3}},"b":{"c":4}}"#, r"path(. as $r | .a as $x | 1 | foreach ($x | . as {b:$q} | 1) as $y ($r | .a | tostring; .))", "", "Invalid path expression with result \"{\\\"b\\\":{\\\"c\\\":3}}\"", 5),
        (r#"{"a":{"b":{"c":3}},"b":{"c":4}}"#, r"path(. as $r | .a as $x | null | foreach ($x as [$q] | 1) as $y ($r | .a | tostring; .))", "", "Cannot index object with number", 5),
        (r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#, r"path(. as $x | foreach ($x as {b:$q} | .) as $y (.a | tostring; .; .))", "", "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...", 5),
        (r#"{"a":{"b":1,"c":{"b":3}},"b":2}"#, r"del(. as $x | foreach ($x as {b:$q} | .) as $y (.a | tostring; .; .))", "", "Invalid path expression near attempt to access element \"b\" of {\"a\":{\"b\":1,\"c\":{\"b\":3}},\"...", 5),
        (r#"{"a":true,"b":false}"#, r"path(. as $x | . as $r | 1 | foreach ($x | . as {b:$q} | 1) as $y ($r | .a | tostring; .; 1))", "", "Invalid path expression near attempt to access element \"b\" of {\"a\":true,\"b\":false}", 5),
    ])
}
