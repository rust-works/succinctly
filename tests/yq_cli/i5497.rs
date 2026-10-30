//! #5497: `succinctly jq` writes `debug`'s `["DEBUG:", v]` as one value, so `v` meets jq's print
//! depth a level sooner. yq's writer has no print depth, and wrapping its value would bring
//! its own value-tree ceiling (384) a level closer, so yq keeps writing the line around the
//! value as before: a value 383 levels deep still prints in full.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn test_debug_keeps_the_whole_value_at_the_value_tree_ceiling_5497() -> Result<()> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(
        "reduce range(383) as $i (0; [.]) | debug | length",
        "",
        &["-n", "--jq-extensions"],
    )?;
    assert_eq!((stdout.as_str(), code), ("1\n", 0), "{stderr}");
    assert_eq!(
        stderr,
        format!("[\"DEBUG:\",{}0{}]\n", "[".repeat(383), "]".repeat(383))
    );
    Ok(())
}
