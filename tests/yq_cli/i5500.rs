//! #5500: the array-growth bound `succinctly jq` gained applies to `succinctly yq`, which shares
//! the helper. yq itself has no sentence to match, so the wording is succinctly's own.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn test_yq_index_write_past_the_bound_is_an_error_5500() -> Result<()> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(".a[1000000000000] = 1", "a: []\n", &[])?;
    assert_eq!(stdout, "");
    assert!(stderr.contains("Cannot grow array to"), "{stderr}");
    assert_ne!(code, 0);
    Ok(())
}

#[test]
fn test_yq_index_write_within_the_bound_still_pads_5500() -> Result<()> {
    let (stdout, _, code) = run_yq_stdin_with_stderr(".a[2] = 1", "a: []\n", &["-o=json", "-I=0"])?;
    assert_eq!((stdout.as_str(), code), ("{\"a\":[null,null,1]}\n", 0));
    Ok(())
}
