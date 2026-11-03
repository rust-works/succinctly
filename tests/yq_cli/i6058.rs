//! #6058: a single-quoted scalar with an empty line decodes to a value with a `\n`, and yq writes it
//! back across lines in single quotes: each `\n` after a non-break character is written twice, the
//! text after a break is indented, and a break at the very end leaves the closing quote on its own
//! line. `succinctly yq` wrote one break per `\n` with no indent, which a YAML reader folds back to a
//! space, so the output read back as another value. Every row is a live yq v4.53.3 capture.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, expected output).
const ROWS: &[(&str, &str, &str)] = &[
    ("a: 'x\n\ny'\n", ".", "a: 'x\n\n  y'\n"),
    ("a: 'x\n\n\ny'\n", ".", "a: 'x\n\n\n  y'\n"),
    ("a: 'x\n\n\n\ny'\n", ".", "a: 'x\n\n\n\n  y'\n"),
    ("a: 'x\n\ny\n\nz'\n", ".", "a: 'x\n\n  y\n\n  z'\n"),
    ("a: 'x\n\n'\n", ".", "a: 'x\n\n'\n"),
    ("a: '\n\nx'\n", ".", "a: '\n\n  x'\n"),
    ("a: 'it''s\n\nx'\n", ".", "a: 'it''s\n\n  x'\n"),
    ("- 'x\n\ny'\n", ".", "- 'x\n\n  y'\n"),
    ("- - 'x\n\n    y'\n", ".", "- - 'x\n\n    y'\n"),
    ("k:\n  a: 'x\n\n    y'\n", ".", "k:\n  a: 'x\n\n    y'\n"),
    (
        "k:\n  j:\n    a: 'x\n\n      y'\n",
        ".",
        "k:\n  j:\n    a: 'x\n\n      y'\n",
    ),
    (
        "- a: 'x\n\ny'\n  b: 1\n",
        ".",
        "- a: 'x\n\n    y'\n  b: 1\n",
    ),
    ("a: ['x\n\n  y']\n", ".", "a: ['x\n\n    y']\n"),
    ("a: {b: 'x\n\n  y'}\n", ".", "a: {b: 'x\n\n    y'}\n"),
    ("a: 'x\n\ny' # c\n", ".", "a: 'x\n\n  y' # c\n"),
    ("a: !!str 'x\n\ny'\n", ".", "a: !!str 'x\n\n  y'\n"),
    (
        "a: &an 'x\n\ny'\nb: *an\n",
        ".",
        "a: &an 'x\n\n  y'\nb: *an\n",
    ),
    (
        "---\na: 'x\n\ny'\n---\nb: 'p\n\nq'\n",
        ".",
        "---\na: 'x\n\n  y'\n---\nb: 'p\n\n  q'\n",
    ),
    // Lines broken by a lone CR, and a CRLF source, read as the same breaks.
    ("a: 'x\r\r y'\n", ".", "a: 'x\n\n  y'\n"),
    ("a: 'x\r\n\r\ny'\n", ".", "a: 'x\n\n  y'\n"),
    // A trailing break inside a nested mapping leaves the closing quote at the start of its line.
    ("k:\n  a: 'x\n\n'\n", ".", "k:\n  a: 'x\n\n'\n"),
    // A tab beside the break stops single quotes.
    ("a: 'x\n\ny\tz'\n", ".", "a: \"x\\ny\\tz\"\n"),
    // A flow item rewritten with `style=`.
    (
        "a: [x]\n",
        ".a[0] = \"p\\nq\" | .a[0] style=\"single\"",
        "a: ['p\n\n    q']\n",
    ),
    // A space next to the break stops single quotes (go-yaml's analysis): the value is the same.
    ("a: 'x \n\ny'\n", ".", "a: 'x\n\n  y'\n"),
    // A write elsewhere in the document keeps the scalar, and `style=` re-spells it.
    ("a: 'x\n\ny'\nb: 2\n", ".b = 3", "a: 'x\n\n  y'\nb: 3\n"),
    ("a: 'x\n\ny'\n", ".a |= .", "a: 'x\n\n  y'\n"),
    ("a: 'x\n\ny'\n", ".a += \"z\"", "a: 'x\n\n  yz'\n"),
    (
        "k:\n  a: 'x\n\ny'\n  b: 1\n",
        ".k.b = 2",
        "k:\n  a: 'x\n\n    y'\n  b: 2\n",
    ),
    (
        "a: ['x\n\n  y', 2]\n",
        ".a[1] = 3",
        "a: ['x\n\n    y', 3]\n",
    ),
    ("a: \"x\n\ny\"\n", ".a style=\"single\"", "a: 'x\n\n  y'\n"),
    // Neighbours that must not change: the value, the JSON output, and the other styles.
    ("a: 'x\n\ny'\n", ".a", "x\ny\n"),
    ("a: 'x\n  y'\n", ".", "a: 'x y'\n"),
    ("a: \"x\n\n  y\"\n", ".", "a: \"x\\ny\"\n"),
    ("a: 'x\n\ny'\n", ".a style=\"double\"", "a: \"x\\ny\"\n"),
];

#[test]
fn a_single_quoted_scalar_with_an_empty_line_is_written_across_lines_as_yq_writes_it() -> Result<()>
{
    for (doc, filter, want) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!(code, 0, "`{filter}` over {doc:?} failed");
        assert_eq!(stdout, *want, "`{filter}` over {doc:?}");
    }
    Ok(())
}

#[test]
fn the_yaml_output_of_such_a_scalar_reads_back_as_the_same_value() -> Result<()> {
    for doc in [
        "a: 'x\n\ny'\n",
        "a: 'x\n\n\ny'\n",
        "- 'x\n\ny'\n",
        "k:\n  a: 'x\n\n    y'\n",
    ] {
        let (written, code) = run_yq_stdin(".", doc, &[])?;
        assert_eq!(code, 0);
        let (before, _) = run_yq_stdin(".", doc, &["-o=json", "-I0"])?;
        let (after, _) = run_yq_stdin(".", &written, &["-o=json", "-I0"])?;
        assert_eq!(before, after, "{doc:?} was written as {written:?}");
    }
    Ok(())
}
