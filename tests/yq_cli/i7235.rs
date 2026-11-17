//! #7235: `.a line_comment = "x"` on a block collection. yq's encoder cannot write the comment on
//! the key's line, holds it and writes it on a later one (`b: 6 # x`), or loses it when nothing
//! follows, as it does for a collection a write put where a commented scalar was (#7230).
//! Each row is `(document, filter, stdout)` as yq v4.53.3 answers.

use anyhow::Result;

use crate::run_yq_stdin;

const ROWS: &[(&str, &str, &str)] = &[
    (
        "a:\n  c: 1\nb: 6\n",
        ".a line_comment = \"x\"",
        "a:\n  c: 1\nb: 6 # x\n",
    ),
    (
        "a:\n  - 1\nb: 6\n",
        ".a line_comment = \"x\"",
        "a:\n  - 1\nb: 6 # x\n",
    ),
    (
        "a:\n  c: 1\nb: 6\nd: 7\n",
        ".a line_comment = \"x\"",
        "a:\n  c: 1\nb: 6 # x\nd: 7\n",
    ),
    (
        "a:\n  c: 1\nb:\n  - 2\n",
        ".a line_comment = \"x\"",
        "a:\n  c: 1\nb: # x\n  - 2\n",
    ),
    ("a:\n  c: 1\n", ".a line_comment = \"x\"", "a:\n  c: 1\n"),
    (
        "a:\n  c: 1\nb: 6 # k\n",
        ".a line_comment = \"x\"",
        "a:\n  c: 1\nb: 6 # k\n",
    ),
    (
        "a:\n  c: 1 # z\nb: 6\n",
        ".a line_comment = \"x\"",
        "a:\n  c: 1 # z\nb: 6 # x\n",
    ),
    (
        "x:\n  a:\n    c: 1\n  b: 6\n",
        ".x.a line_comment = \"x\"",
        "x:\n  a:\n    c: 1\n  b: 6 # x\n",
    ),
    (
        "- a:\n    c: 1\n  b: 6\n",
        ".[0].a line_comment = \"x\"",
        "- a:\n    c: 1\n  b: 6 # x\n",
    ),
    (
        "a:\n  c: 1\nb: 6\n",
        ".a line_comment |= \"x\"",
        "a:\n  c: 1\nb: 6 # x\n",
    ),
    (
        "a:\n  c: 1\nb: 6\n",
        ".a line_comment = \"m\\nl\"",
        "a:\n  c: 1\nb: 6 # m\n\n# l\n",
    ),
    (
        "a:\n  c: 1\nb: {p: 1}\n",
        ".a line_comment = \"x\"",
        "a:\n  c: 1\nb: {p: 1}\n",
    ),
    (
        "a: {p: 1}\nb: 2\n",
        ".a line_comment = \"x\"",
        "a: {p: 1} # x\nb: 2\n",
    ),
    (
        "a: 1 # o\nb: 2\n",
        ".a line_comment = \"x\"",
        "a: 1 # x\nb: 2\n",
    ),
    (
        "a:\n  c: 1\nb: 6\n",
        ".a line_comment = \"\"",
        "a:\n  c: 1\nb: 6\n",
    ),
    (
        "a:\n  c: 1\nb: 6\n",
        ".a line_comment = \"x\" | .b = 1",
        "a:\n  c: 1\nb: 1 # x\n",
    ),
];

#[test]
fn test_line_comment_on_a_block_collection_is_held_for_a_later_line_7235() -> Result<()> {
    for &(document, filter, stdout) in ROWS {
        let (out, code) = run_yq_stdin(filter, document, &[])?;
        assert_eq!(
            (out.as_str(), code),
            (stdout, 0),
            "{filter} on {document:?}"
        );
    }
    Ok(())
}
