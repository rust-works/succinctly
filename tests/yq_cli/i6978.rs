//! #6978: a block scalar whose header follows a key, a `- ` or a flow key on a `---` marker line is
//! not the document's own node, so its content may not sit at column 0. The reader took every header
//! on a marker line for `--- >`, where the scalar is the document and may; an empty scalar after
//! `--- x: >` then read every following column-0 line as its text, as well as an entry of its own.
//!
//! Every row is captured from yq v4.53.3 (`-o=json -I=0 .`).

use anyhow::Result;

use crate::run_yq_stdin;

/// `(document, expected JSON output)`; the same under yq and `succinctly yq`.
const ROWS: &[(&str, &str)] = &[
    ("--- x: >\n\nb: 1\n", "{\"x\":\"\",\"b\":1}\n"),
    ("--- x: |\n\nb: 1\n", "{\"x\":\"\",\"b\":1}\n"),
    ("--- x: >+\n\nb: 1\n", "{\"x\":\"\",\"b\":1}\n"),
    ("--- x: >-\n\nb: 1\n", "{\"x\":\"\",\"b\":1}\n"),
    ("--- x: >2\n\nb: 1\n", "{\"x\":\"\",\"b\":1}\n"),
    ("--- x: >\r\n\r\nb: 1\r\n", "{\"x\":\"\",\"b\":1}\n"),
    ("---   x: >\n\nb: 1\n", "{\"x\":\"\",\"b\":1}\n"),
    // A property before the key, a sequence item and a flow key.
    ("--- &a x: >\n\nb: 1\n", "{\"x\":\"\",\"b\":1}\n"),
    ("--- !!str x: >\n\nb: 1\n", "{\"x\":\"\",\"b\":1}\n"),
    ("--- - >\n\n- b\n", "[\"\",\"b\"]\n"),
    ("--- [a]: >\n\nb: 1\n", "{\"\":\"\",\"b\":1}\n"),
    // The scalar has content: unchanged.
    ("--- x: >\n\n  y\nb: 1\n", "{\"x\":\"y\\n\",\"b\":1}\n"),
    // The header on the line after the marker is the document's own scalar.
    ("---\n>\n y\n", "\"y\\n\"\n"),
    // An explicit key whose value is the empty scalar.
    ("--- ? >\n\n: 1\n", "{\"\":1}\n"),
];

#[test]
fn a_header_after_a_key_on_a_marker_line_is_not_the_documents_scalar_6978() -> Result<()> {
    for (doc, want) in ROWS {
        let (out, code) = run_yq_stdin(".", doc, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{doc:?} was refused: {out}");
        assert_eq!(out, *want, "{doc:?}");
    }
    Ok(())
}

#[test]
fn the_entry_after_the_empty_scalar_is_read_once_6978() -> Result<()> {
    let doc = "--- x: >\n\nb: 1\n";
    let (out, code) = run_yq_stdin(".b", doc, &[])?;
    assert_eq!((code, out.as_str()), (0, "1\n"));
    let (out, code) = run_yq_stdin(".x | length", doc, &[])?;
    assert_eq!((code, out.as_str()), (0, "0\n"));
    Ok(())
}
