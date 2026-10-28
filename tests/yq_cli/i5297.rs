//! #5297: `@html` is a succinctly extension in yq mode -- real yq's lexer rejects it, so it needs
//! `--jq-extensions` (#5444) -- and shares jq mode's formatter, so it spells `'` as `&apos;` too,
//! as jq 1.7.1 does. This pins that the yq side follows the one helper rather than keeping a
//! spelling of its own.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_html_escapes_apostrophe_as_apos_in_yq_mode_5297() -> Result<()> {
    let (out, code) = run_yq_stdin(".a | @html", "a: \"it's <b>\"\n", &["--jq-extensions"])?;
    assert_eq!((out.as_str(), code), ("it&apos;s &lt;b&gt;\n", 0));
    Ok(())
}
