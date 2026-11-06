//! #6434: a `succinctly yq` message that embeds a cut value is jq's wording, not yq's (yq says
//! `cannot get keys of !!str`), and cuts a multi-byte character the way jq's does: one U+FFFD
//! before the `...`.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn a_cut_value_in_a_message_writes_one_replacement_6434() -> Result<()> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr("keys", "\"あああああああああ\"\n", &[])?;
    assert_eq!((stdout.as_str(), code), ("", 1));
    assert_eq!(
        stderr.trim_end(),
        "Error: string (\"あああ\u{FFFD}...) has no keys"
    );
    Ok(())
}
