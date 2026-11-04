//! #6069 is jq mode only: Go's `regexp`, like the `regex` crate, ends a repeat without an empty
//! iteration, so yq v4.53.3 reports the last non-empty one and `succinctly yq` must too.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn yq_keeps_the_last_non_empty_iteration() -> Result<()> {
    let (out, code) = run_yq_stdin(r#""ab" | match("(a*)*b")"#, "", &["-n", "-o=json", "-I=0"])?;
    assert_eq!(
        (out.trim_end(), code),
        (
            r#"{"string":"ab","offset":0,"length":2,"captures":[{"string":"a","offset":0,"length":1}]}"#,
            0
        )
    );
    Ok(())
}
