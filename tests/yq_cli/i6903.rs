//! #6903: yq's head comment of the first document's root is the document's whole preamble: the `---`
//! marker and the comments before and after it. A write to the root's head (`head_comment`,
//! `comments`, and `...` or `..` reaching the root) replaces all of it, marker included, so
//! `... comments=""` strips a leading `---` and the comments around it. Later documents keep their
//! `---` as separators. Every row is captured from yq v4.53.3: the source document, the filter and
//! the whole YAML output.

use anyhow::Result;

use crate::run_yq_stdin;

const ROWS: &[(&str, &str, &str)] = &[
    ("---\na: 1\n", "... comments=\"\"", "a: 1\n"),
    ("---\na: 1\n", ". head_comment=\"\"", "a: 1\n"),
    ("---\na: 1\n", ". head_comment=\"hi\"", "# hi\na: 1\n"),
    ("---\na: 1\n", ". comments=\"hi\"", "# hi\na: 1\n# hi\n"),
    ("---\na: 1\n", ". comments=\"\"", "a: 1\n"),
    ("---\na: 1\n", ". foot_comment=\"f\"", "---\na: 1\n# f\n"),
    ("---\na: 1\n", ". foot_comment=\"\"", "---\na: 1\n"),
    ("---\na: 1\n", ". line_comment=\"\"", "---\na: 1\n"),
    ("---\na: 1\n", "... head_comment=\"\"", "a: 1\n"),
    ("---\na: 1\n", ".. comments=\"\"", "a: 1\n"),
    (
        "---\na: 1\n",
        ". head_comment=\"x\" | .a = 2",
        "# x\na: 2\n",
    ),
    ("---\na: 1\n", ".a = 3", "---\na: 3\n"),
    ("---\na: 1\n", ". head_comment=\"\"  | .a = 3", "a: 3\n"),
    ("---\n# c\na: 1\n", "... comments=\"\"", "a: 1\n"),
    ("---\n# c\na: 1\n", ". head_comment=\"\"", "a: 1\n"),
    ("---\n# c\na: 1\n", ". head_comment=\"hi\"", "# hi\na: 1\n"),
    (
        "---\n# c\na: 1\n",
        ". comments=\"hi\"",
        "# hi\na: 1\n# hi\n",
    ),
    ("---\n# c\na: 1\n", ". comments=\"\"", "a: 1\n"),
    (
        "---\n# c\na: 1\n",
        ". foot_comment=\"f\"",
        "---\n# c\na: 1\n# f\n",
    ),
    (
        "---\n# c\na: 1\n",
        ". foot_comment=\"\"",
        "---\n# c\na: 1\n",
    ),
    (
        "---\n# c\na: 1\n",
        ". line_comment=\"\"",
        "---\n# c\na: 1\n",
    ),
    ("---\n# c\na: 1\n", "... head_comment=\"\"", "a: 1\n"),
    ("---\n# c\na: 1\n", ".. comments=\"\"", "a: 1\n"),
    (
        "---\n# c\na: 1\n",
        ". head_comment=\"x\" | .a = 2",
        "# x\na: 2\n",
    ),
    ("---\n# c\na: 1\n", ".a = 3", "---\n# c\na: 3\n"),
    (
        "---\n# c\na: 1\n",
        ". head_comment=\"\"  | .a = 3",
        "a: 3\n",
    ),
    ("# top\n---\na: 1\n", "... comments=\"\"", "a: 1\n"),
    ("# top\n---\na: 1\n", ". head_comment=\"\"", "a: 1\n"),
    (
        "# top\n---\na: 1\n",
        ". head_comment=\"hi\"",
        "# hi\na: 1\n",
    ),
    (
        "# top\n---\na: 1\n",
        ". comments=\"hi\"",
        "# hi\na: 1\n# hi\n",
    ),
    ("# top\n---\na: 1\n", ". comments=\"\"", "a: 1\n"),
    (
        "# top\n---\na: 1\n",
        ". foot_comment=\"f\"",
        "# top\n---\na: 1\n# f\n",
    ),
    (
        "# top\n---\na: 1\n",
        ". foot_comment=\"\"",
        "# top\n---\na: 1\n",
    ),
    (
        "# top\n---\na: 1\n",
        ". line_comment=\"\"",
        "# top\n---\na: 1\n",
    ),
    ("# top\n---\na: 1\n", "... head_comment=\"\"", "a: 1\n"),
    ("# top\n---\na: 1\n", ".. comments=\"\"", "a: 1\n"),
    (
        "# top\n---\na: 1\n",
        ". head_comment=\"x\" | .a = 2",
        "# x\na: 2\n",
    ),
    ("# top\n---\na: 1\n", ".a = 3", "# top\n---\na: 3\n"),
    (
        "# top\n---\na: 1\n",
        ". head_comment=\"\"  | .a = 3",
        "a: 3\n",
    ),
    (
        "# top\n# two\n---\n# c\na: 1\n",
        "... comments=\"\"",
        "a: 1\n",
    ),
    (
        "# top\n# two\n---\n# c\na: 1\n",
        ". head_comment=\"\"",
        "a: 1\n",
    ),
    (
        "# top\n# two\n---\n# c\na: 1\n",
        ". head_comment=\"hi\"",
        "# hi\na: 1\n",
    ),
    (
        "# top\n# two\n---\n# c\na: 1\n",
        ". comments=\"hi\"",
        "# hi\na: 1\n# hi\n",
    ),
    (
        "# top\n# two\n---\n# c\na: 1\n",
        ". comments=\"\"",
        "a: 1\n",
    ),
    (
        "# top\n# two\n---\n# c\na: 1\n",
        ". foot_comment=\"f\"",
        "# top\n# two\n---\n# c\na: 1\n# f\n",
    ),
    (
        "# top\n# two\n---\n# c\na: 1\n",
        ". foot_comment=\"\"",
        "# top\n# two\n---\n# c\na: 1\n",
    ),
    (
        "# top\n# two\n---\n# c\na: 1\n",
        ". line_comment=\"\"",
        "# top\n# two\n---\n# c\na: 1\n",
    ),
    (
        "# top\n# two\n---\n# c\na: 1\n",
        "... head_comment=\"\"",
        "a: 1\n",
    ),
    (
        "# top\n# two\n---\n# c\na: 1\n",
        ".. comments=\"\"",
        "a: 1\n",
    ),
    (
        "# top\n# two\n---\n# c\na: 1\n",
        ". head_comment=\"x\" | .a = 2",
        "# x\na: 2\n",
    ),
    (
        "# top\n# two\n---\n# c\na: 1\n",
        ".a = 3",
        "# top\n# two\n---\n# c\na: 3\n",
    ),
    (
        "# top\n# two\n---\n# c\na: 1\n",
        ". head_comment=\"\"  | .a = 3",
        "a: 3\n",
    ),
    (
        "---\na: 1\n---\nb: 2\n",
        "... comments=\"\"",
        "a: 1\n---\nb: 2\n",
    ),
    (
        "---\na: 1\n---\nb: 2\n",
        ". head_comment=\"\"",
        "a: 1\n---\nb: 2\n",
    ),
    (
        "---\na: 1\n---\nb: 2\n",
        ". head_comment=\"hi\"",
        "# hi\na: 1\n---\n# hi\nb: 2\n",
    ),
    (
        "---\na: 1\n---\nb: 2\n",
        ". comments=\"hi\"",
        "# hi\na: 1\n# hi\n---\n# hi\nb: 2\n# hi\n",
    ),
    (
        "---\na: 1\n---\nb: 2\n",
        ". comments=\"\"",
        "a: 1\n---\nb: 2\n",
    ),
    (
        "---\na: 1\n---\nb: 2\n",
        ". foot_comment=\"f\"",
        "---\na: 1\n# f\n---\nb: 2\n# f\n",
    ),
    (
        "---\na: 1\n---\nb: 2\n",
        ". foot_comment=\"\"",
        "---\na: 1\n---\nb: 2\n",
    ),
    (
        "---\na: 1\n---\nb: 2\n",
        ". line_comment=\"\"",
        "---\na: 1\n---\nb: 2\n",
    ),
    (
        "---\na: 1\n---\nb: 2\n",
        "... head_comment=\"\"",
        "a: 1\n---\nb: 2\n",
    ),
    (
        "---\na: 1\n---\nb: 2\n",
        ".. comments=\"\"",
        "a: 1\n---\nb: 2\n",
    ),
    (
        "---\na: 1\n---\nb: 2\n",
        ". head_comment=\"x\" | .a = 2",
        "# x\na: 2\n---\n# x\nb: 2\na: 2\n",
    ),
    (
        "---\na: 1\n---\nb: 2\n",
        ".a = 3",
        "---\na: 3\n---\nb: 2\na: 3\n",
    ),
    (
        "---\na: 1\n---\nb: 2\n",
        ". head_comment=\"\"  | .a = 3",
        "a: 3\n---\nb: 2\na: 3\n",
    ),
    ("# c\na: 1\n", "... comments=\"\"", "a: 1\n"),
    ("# c\na: 1\n", ". head_comment=\"\"", "a: 1\n"),
    ("# c\na: 1\n", ". head_comment=\"hi\"", "# hi\na: 1\n"),
    ("# c\na: 1\n", ". comments=\"hi\"", "# hi\na: 1\n# hi\n"),
    ("# c\na: 1\n", ". comments=\"\"", "a: 1\n"),
    ("# c\na: 1\n", ". foot_comment=\"f\"", "# c\na: 1\n# f\n"),
    ("# c\na: 1\n", ". foot_comment=\"\"", "# c\na: 1\n"),
    ("# c\na: 1\n", ". line_comment=\"\"", "# c\na: 1\n"),
    ("# c\na: 1\n", "... head_comment=\"\"", "a: 1\n"),
    ("# c\na: 1\n", ".. comments=\"\"", "a: 1\n"),
    (
        "# c\na: 1\n",
        ". head_comment=\"x\" | .a = 2",
        "# x\na: 2\n",
    ),
    ("# c\na: 1\n", ".a = 3", "# c\na: 3\n"),
    ("# c\na: 1\n", ". head_comment=\"\"  | .a = 3", "a: 3\n"),
    (
        "a: 1\n---\nb: 2\n",
        "... comments=\"\"",
        "a: 1\n---\nb: 2\n",
    ),
    (
        "a: 1\n---\nb: 2\n",
        ". head_comment=\"\"",
        "a: 1\n---\nb: 2\n",
    ),
    (
        "a: 1\n---\nb: 2\n",
        ". head_comment=\"hi\"",
        "# hi\na: 1\n---\n# hi\nb: 2\n",
    ),
    (
        "a: 1\n---\nb: 2\n",
        ". comments=\"hi\"",
        "# hi\na: 1\n# hi\n---\n# hi\nb: 2\n# hi\n",
    ),
    ("a: 1\n---\nb: 2\n", ". comments=\"\"", "a: 1\n---\nb: 2\n"),
    (
        "a: 1\n---\nb: 2\n",
        ". foot_comment=\"f\"",
        "a: 1\n# f\n---\nb: 2\n# f\n",
    ),
    (
        "a: 1\n---\nb: 2\n",
        ". foot_comment=\"\"",
        "a: 1\n---\nb: 2\n",
    ),
    (
        "a: 1\n---\nb: 2\n",
        ". line_comment=\"\"",
        "a: 1\n---\nb: 2\n",
    ),
    (
        "a: 1\n---\nb: 2\n",
        "... head_comment=\"\"",
        "a: 1\n---\nb: 2\n",
    ),
    ("a: 1\n---\nb: 2\n", ".. comments=\"\"", "a: 1\n---\nb: 2\n"),
    (
        "a: 1\n---\nb: 2\n",
        ". head_comment=\"x\" | .a = 2",
        "# x\na: 2\n---\n# x\nb: 2\na: 2\n",
    ),
    ("a: 1\n---\nb: 2\n", ".a = 3", "a: 3\n---\nb: 2\na: 3\n"),
    (
        "a: 1\n---\nb: 2\n",
        ". head_comment=\"\"  | .a = 3",
        "a: 3\n---\nb: 2\na: 3\n",
    ),
    ("---\n- x\n- y\n", "... comments=\"\"", "- x\n- y\n"),
    ("---\n- x\n- y\n", ". head_comment=\"\"", "- x\n- y\n"),
    (
        "---\n- x\n- y\n",
        ". head_comment=\"hi\"",
        "# hi\n- x\n- y\n",
    ),
    (
        "---\n- x\n- y\n",
        ". comments=\"hi\"",
        "# hi\n- x\n- y\n# hi\n",
    ),
    ("---\n- x\n- y\n", ". comments=\"\"", "- x\n- y\n"),
    (
        "---\n- x\n- y\n",
        ". foot_comment=\"f\"",
        "---\n- x\n- y\n# f\n",
    ),
    ("---\n- x\n- y\n", ". foot_comment=\"\"", "---\n- x\n- y\n"),
    ("---\n- x\n- y\n", ". line_comment=\"\"", "---\n- x\n- y\n"),
    ("---\n- x\n- y\n", "... head_comment=\"\"", "- x\n- y\n"),
    ("---\n- x\n- y\n", ".. comments=\"\"", "- x\n- y\n"),
    ("---\n# k\n- x\n- y\n", "... comments=\"\"", "- x\n- y\n"),
    ("---\n# k\n- x\n- y\n", ". head_comment=\"\"", "- x\n- y\n"),
    (
        "---\n# k\n- x\n- y\n",
        ". head_comment=\"hi\"",
        "# hi\n- x\n- y\n",
    ),
    (
        "---\n# k\n- x\n- y\n",
        ". comments=\"hi\"",
        "# hi\n- x\n- y\n# hi\n",
    ),
    ("---\n# k\n- x\n- y\n", ". comments=\"\"", "- x\n- y\n"),
    (
        "---\n# k\n- x\n- y\n",
        ". foot_comment=\"f\"",
        "---\n# k\n- x\n- y\n# f\n",
    ),
    (
        "---\n# k\n- x\n- y\n",
        ". foot_comment=\"\"",
        "---\n# k\n- x\n- y\n",
    ),
    (
        "---\n# k\n- x\n- y\n",
        ". line_comment=\"\"",
        "---\n# k\n- x\n- y\n",
    ),
    (
        "---\n# k\n- x\n- y\n",
        "... head_comment=\"\"",
        "- x\n- y\n",
    ),
    ("---\n# k\n- x\n- y\n", ".. comments=\"\"", "- x\n- y\n"),
    ("---\na: 1 # l\n# f\n", "... comments=\"\"", "a: 1\n"),
    (
        "---\na: 1 # l\n# f\n",
        ". head_comment=\"\"",
        "a: 1 # l\n# f\n",
    ),
    (
        "---\na: 1 # l\n# f\n",
        ". head_comment=\"hi\"",
        "# hi\na: 1 # l\n# f\n",
    ),
    (
        "---\na: 1 # l\n# f\n",
        ". comments=\"hi\"",
        "# hi\na: 1 # l\n# hi\n",
    ),
    ("---\na: 1 # l\n# f\n", ". comments=\"\"", "a: 1 # l\n"),
    (
        "---\na: 1 # l\n# f\n",
        ". foot_comment=\"f\"",
        "---\na: 1 # l\n# f\n",
    ),
    (
        "---\na: 1 # l\n# f\n",
        ". foot_comment=\"\"",
        "---\na: 1 # l\n",
    ),
    (
        "---\na: 1 # l\n# f\n",
        ". line_comment=\"\"",
        "---\na: 1 # l\n# f\n",
    ),
    (
        "---\na: 1 # l\n# f\n",
        "... head_comment=\"\"",
        "a: 1 # l\n# f\n",
    ),
    ("---\na: 1 # l\n# f\n", ".. comments=\"\"", "a: 1\n"),
    (
        "---\na: 1 # l\n# f\n",
        ". head_comment=\"x\" | .a = 2",
        "# x\na: 2 # l\n# f\n",
    ),
    ("---\na: 1 # l\n# f\n", ".a = 3", "---\na: 3 # l\n# f\n"),
    (
        "---\na: 1 # l\n# f\n",
        ". head_comment=\"\"  | .a = 3",
        "a: 3 # l\n# f\n",
    ),
    (
        "---\n# c\na: 1\nb: 2\n",
        "... comments=\"\"",
        "a: 1\nb: 2\n",
    ),
    (
        "---\n# c\na: 1\nb: 2\n",
        ". head_comment=\"\"",
        "a: 1\nb: 2\n",
    ),
    (
        "---\n# c\na: 1\nb: 2\n",
        ". head_comment=\"hi\"",
        "# hi\na: 1\nb: 2\n",
    ),
    (
        "---\n# c\na: 1\nb: 2\n",
        ". comments=\"hi\"",
        "# hi\na: 1\nb: 2\n# hi\n",
    ),
    ("---\n# c\na: 1\nb: 2\n", ". comments=\"\"", "a: 1\nb: 2\n"),
    (
        "---\n# c\na: 1\nb: 2\n",
        ". foot_comment=\"f\"",
        "---\n# c\na: 1\nb: 2\n# f\n",
    ),
    (
        "---\n# c\na: 1\nb: 2\n",
        ". foot_comment=\"\"",
        "---\n# c\na: 1\nb: 2\n",
    ),
    (
        "---\n# c\na: 1\nb: 2\n",
        ". line_comment=\"\"",
        "---\n# c\na: 1\nb: 2\n",
    ),
    (
        "---\n# c\na: 1\nb: 2\n",
        "... head_comment=\"\"",
        "a: 1\nb: 2\n",
    ),
    ("---\n# c\na: 1\nb: 2\n", ".. comments=\"\"", "a: 1\nb: 2\n"),
    (
        "---\n# c\na: 1\nb: 2\n",
        ". head_comment=\"x\" | .a = 2",
        "# x\na: 2\nb: 2\n",
    ),
    ("---\n# c\na: 1\nb: 2\n", ".a = 3", "---\n# c\na: 3\nb: 2\n"),
    (
        "---\n# c\na: 1\nb: 2\n",
        ". head_comment=\"\"  | .a = 3",
        "a: 3\nb: 2\n",
    ),
    ("---\n# c\n\na: 1\n", "... comments=\"\"", "a: 1\n"),
    ("---\n# c\n\na: 1\n", ". head_comment=\"\"", "a: 1\n"),
    (
        "---\n# c\n\na: 1\n",
        ". head_comment=\"hi\"",
        "# hi\na: 1\n",
    ),
    (
        "---\n# c\n\na: 1\n",
        ". comments=\"hi\"",
        "# hi\na: 1\n# hi\n",
    ),
    ("---\n# c\n\na: 1\n", ". comments=\"\"", "a: 1\n"),
    (
        "---\n# c\n\na: 1\n",
        ". foot_comment=\"f\"",
        "---\n# c\n\na: 1\n# f\n",
    ),
    (
        "---\n# c\n\na: 1\n",
        ". foot_comment=\"\"",
        "---\n# c\n\na: 1\n",
    ),
    (
        "---\n# c\n\na: 1\n",
        ". line_comment=\"\"",
        "---\n# c\n\na: 1\n",
    ),
    ("---\n# c\n\na: 1\n", "... head_comment=\"\"", "a: 1\n"),
    ("---\n# c\n\na: 1\n", ".. comments=\"\"", "a: 1\n"),
    (
        "---\n# c\n\na: 1\n",
        ". head_comment=\"x\" | .a = 2",
        "# x\na: 2\n",
    ),
    ("---\n# c\n\na: 1\n", ".a = 3", "---\n# c\n\na: 3\n"),
    (
        "---\n# c\n\na: 1\n",
        ". head_comment=\"\"  | .a = 3",
        "a: 3\n",
    ),
];

#[test]
fn a_root_head_write_replaces_the_first_documents_preamble_6903() -> Result<()> {
    for (input, filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(code, 0, "{filter} over {input:?}");
        assert_eq!(out, *want, "{filter} over {input:?}");
    }
    Ok(())
}
