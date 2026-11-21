//! #7536: `succinctly yq` keeps its own sentence for an index write past 2^29 elements, where
//! `succinctly jq` says jq 1.8.2's `Array index too large`. The mode decides (ADR-0018): yq has
//! no sentence to copy (it allocates until the host kills it), so the wording is succinctly's
//! own, as for the lower sequence caps in `docs/compliance/yq/limitations.md`.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn test_yq_index_write_past_2_pow_29_keeps_its_own_sentence_7536() -> Result<()> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(".a[536870912] = 1", "a: [1]\n", &[])?;
    assert_eq!(stdout, "");
    assert_eq!(stderr, "Error: Cannot grow array to 536870913 elements\n");
    assert_ne!(code, 0);
    Ok(())
}
