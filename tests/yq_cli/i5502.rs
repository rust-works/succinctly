//! #5502: `succinctly jq` compiles lookaround with a second engine; `succinctly yq` keeps Go's
//! `regexp`, which refuses it, and Go's `null` for a group that took no part in an empty match.
//! Captured from yq v4.53.3.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

#[test]
fn test_yq_still_refuses_lookahead_5502() -> Result<()> {
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr(r#".a | test("(?=u)")"#, "a: qux\n", &[])?;
    assert_eq!((stdout.as_str(), code), ("", 1));
    assert!(
        stderr.contains("error parsing regexp: invalid or unsupported Perl syntax: `(?=`"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn test_yq_capture_of_an_empty_match_stays_null_5502() -> Result<()> {
    let (stdout, code) = run_yq_stdin(r#".b | capture("(?P<x>a)?")"#, "b: b\n", &[])?;
    assert_eq!((stdout.as_str(), code), ("x: null\n", 0));
    Ok(())
}
