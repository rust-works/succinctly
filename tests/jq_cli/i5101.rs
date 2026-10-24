//! #5101: rows the path-mode interpreter (#5223) answers as jq 1.7.1 does.
//!
//! The interpreter reads storage identity, which the `unshared-containers` holdout does not have, so these pin the
//! shipped shape only.
#![cfg(not(feature = "unshared-containers"))]

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #5223 Phase 3: this issue's repro rows, with `path`, `del` and `|=`
/// wherever the row is a `path(...)`, as jq 1.7.1 answers them (stdout, the
/// first error message, the exit code). The path-mode interpreter carries
/// jq's register, so they match by construction, on both routes.
#[test]
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5101() -> Result<()> {
    assert_path_rows_both_routes_3749(&[(
        r"null",
        r#"try path(. as $x | foreach (.a?,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (null; 7; .)), 7) | $x) catch "C""#,
        "[\"a\",\"a\"]\n[\"a\"]\n[\"a\"]\n[]\n",
        "",
        0,
    )])
}
