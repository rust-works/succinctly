//! #5539: `succinctly jq` splices a destructured container as the node's own storage so that
//! `$x == $a[0]` is jq's `jv_identical` answer. `succinctly yq` is unchanged: real yq has no
//! destructuring `as` at all (`RHS of 'as' operator must be a variable name`), so this is
//! `--jq-extensions` surface, and its `==` on two sequences compares their contents.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_yq_destructured_container_is_unchanged_5539() -> Result<()> {
    let (stdout, code) = run_yq_stdin(
        "[[1]] as $a | $a as [$x] | $x == $a[0]",
        "a: 1\n",
        &["--jq-extensions", "-o", "json", "-I0"],
    )?;
    assert_eq!((stdout.as_str(), code), ("false\n", 0));
    Ok(())
}
