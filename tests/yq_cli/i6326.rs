//! #6326: jq steps one byte after an empty match, so a multibyte character gives extra empty
//! matches; yq (Go's `regexp`) steps by character, and `succinctly yq` keeps that. The row is
//! `yq -I0 -o json` captured from yq v4.53.3 (offsets are bytes there).

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_yq_steps_by_character_after_an_empty_match_6326() -> Result<()> {
    let (stdout, code) = run_yq_stdin(
        r#".a | [match("";"g") | .offset]"#,
        "a: é\n",
        &["-I0", "-o", "json"],
    )?;
    assert_eq!((stdout.as_str(), code), ("[0,2]\n", 0));
    Ok(())
}
