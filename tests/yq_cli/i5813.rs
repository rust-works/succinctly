//! #5813: an alias used as a mapping key (`*a : 1`) that the soundness pass prints as its text (the
//! anchor is gone, which yq cannot read back) takes the quoting of its anchor's node, so a key whose
//! anchor is the string `"true"` stays a string instead of becoming the bool key `true`. Where the
//! anchor survives the key is still `*a`, as in yq v4.53.3 (captured). The text rows have no yq
//! capture: yq prints an unresolvable `*a` there.

use anyhow::Result;

use crate::run_yq_stdin;

/// (anchor's spelling, the key's text once the alias is dropped).
const ROWS: &[(&str, &str)] = &[
    (r#""true""#, r#""true""#),
    ("'1'", "'1'"),
    ("'null'", "'null'"),
    (r#""~""#, r#""~""#),
    ("!!str 12", r#""12""#),
    ("!!str abc", "abc"),
    // A plain anchor keeps the typed key it always was.
    ("5", "5"),
    ("true", "true"),
    ("plain", "plain"),
];

#[test]
fn test_an_alias_key_printed_as_its_text_keeps_its_anchors_quoting_5813() -> Result<()> {
    for (anchor, key) in ROWS {
        let doc = format!("b: &a {anchor}\nc:\n  *a : 1\n");
        let (stdout, code) = run_yq_stdin("del(.b)", &doc, &[])?;
        assert_eq!(
            (stdout.as_str(), code),
            (format!("c:\n  {key}: 1\n").as_str(), 0),
            "{anchor}"
        );
        // The anchor survives: the key stays the alias, as in yq.
        let (stdout, code) = run_yq_stdin(".", &doc, &[])?;
        assert_eq!(
            (stdout.as_str(), code),
            (format!("b: &a {anchor}\nc:\n  *a : 1\n").as_str(), 0),
            "{anchor}"
        );
    }
    Ok(())
}

/// The printed key reads back as the type the anchor had.
#[test]
fn test_the_dropped_alias_key_reads_back_as_a_string_5813() -> Result<()> {
    for anchor in [r#""true""#, "'1'", "!!str 12"] {
        let doc = format!("b: &a {anchor}\nc:\n  *a : 1\n");
        let (printed, _) = run_yq_stdin("del(.b)", &doc, &[])?;
        let (tag, code) = run_yq_stdin(".c | keys | .[0] | tag", &printed, &[])?;
        assert_eq!((tag.as_str(), code), ("!!str\n", 0), "{anchor}");
    }
    Ok(())
}
