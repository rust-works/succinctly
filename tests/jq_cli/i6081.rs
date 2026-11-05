//! #6081: `eval(expr)` is yq's operator (jq has no such builtin), so `succinctly jq` leaves the
//! name undefined, as jq 1.7.1 does: `jq -n 'eval(".a")'` is a compile error, exit 3.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_eval_is_not_defined_in_jq_mode_6081() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "eval(\".a\")"], None)?;
    assert_eq!((stdout.as_str(), code), ("", 3));
    assert!(stderr.contains("eval/1 is not defined"), "{stderr}");
    Ok(())
}
