//! #6434: a `succinctly yq` message that embeds a cut value is jq's wording, not yq's (yq says
//! `cannot get keys of !!str`), and cuts a multi-byte character the way jq 1.8's does: back to the
//! character start, with no U+FFFD (#7524 reversed #6434's replacement; pin moved).

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn a_cut_value_in_a_message_backs_up_to_a_character_start_7524() -> Result<()> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr("keys", "\"あああああああああ\"\n", &[])?;
    assert_eq!((stdout.as_str(), code), ("", 1));
    assert_eq!(stderr.trim_end(), "Error: string (\"あああ...) has no keys");
    Ok(())
}
