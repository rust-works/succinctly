//! #6976: a mapping key that merely starts with `---` (`----`, `---a`, `---`) is no document marker.
//! The block-scalar reader took any line starting with `---` for a `--- >` marker line, where the
//! scalar is the document's own node and may sit at column 0, so an empty `>`/`|` scalar after such a
//! key read every following column-0 line as its text, as well as an entry of its own.
//!
//! Every row is captured from yq v4.53.3 (`-o=json -I=0 .`).

use anyhow::Result;

use crate::run_yq_stdin;

/// `(document, expected JSON output)`; the same under yq and `succinctly yq`.
const ROWS: &[(&str, &str)] = &[
    ("----: >\n\nb: 1\n", "{\"----\":\"\",\"b\":1}\n"),
    ("----: |\n\nb: 1\n", "{\"----\":\"\",\"b\":1}\n"),
    ("---a: >\n\nb: 1\n", "{\"---a\":\"\",\"b\":1}\n"),
    ("---a: |\nb: 1\n", "{\"---a\":\"\",\"b\":1}\n"),
    ("---: >\n\nb: 1\n", "{\"---\":\"\",\"b\":1}\n"),
    // No blank line between the header and the next entry.
    ("----: >\nb: 1\n", "{\"----\":\"\",\"b\":1}\n"),
    // Chomping: `+` keeps the one break, `-` and clip keep nothing.
    ("----: >+\n\nb: 1\n", "{\"----\":\"\\n\",\"b\":1}\n"),
    ("----: >-\n\nb: 1\n", "{\"----\":\"\",\"b\":1}\n"),
    // A scalar with content is unchanged.
    ("----: >\n  x\n\nb: 1\n", "{\"----\":\"x\\n\",\"b\":1}\n"),
    // Nested, and keys that never agreed.
    (
        "x:\n  ---a: >\n\n  b: 1\n",
        "{\"x\":{\"---a\":\"\",\"b\":1}}\n",
    ),
    ("--: >\n\nb: 1\n", "{\"--\":\"\",\"b\":1}\n"),
    ("a: >\n\nb: 1\n", "{\"a\":\"\",\"b\":1}\n"),
    ("---: x\nb: 1\n", "{\"---\":\"x\",\"b\":1}\n"),
    ("----:\n  - a\nb: 1\n", "{\"----\":[\"a\"],\"b\":1}\n"),
];

#[test]
fn a_key_starting_with_dashes_is_no_document_marker_for_an_empty_block_scalar_6976() -> Result<()> {
    for (doc, want) in ROWS {
        let (out, code) = run_yq_stdin(".", doc, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{doc:?} was refused: {out}");
        assert_eq!(out, *want, "{doc:?}");
    }
    Ok(())
}

#[test]
fn the_entry_after_the_empty_scalar_is_read_once_6976() -> Result<()> {
    let doc = "----: >\n\nb: 1\n";
    let (out, code) = run_yq_stdin(".b", doc, &[])?;
    assert_eq!((code, out.as_str()), (0, "1\n"));
    let (out, code) = run_yq_stdin(".\"----\" | length", doc, &[])?;
    assert_eq!((code, out.as_str()), (0, "0\n"));
    // YAML output re-quotes the key and keeps the scalar empty.
    let (out, code) = run_yq_stdin(".", doc, &[])?;
    assert_eq!((code, out.as_str()), (0, "'----': \"\"\nb: 1\n"));
    Ok(())
}
