//! #4996 (item 6 of the #4099 sweep): a standalone comment directly under `key:` followed by a blank line, then the key's
//! first child, is that child's head comment in yq -- the blank line is dropped and the comment
//! stays where it was written. It was read as the foot of `key:` and moved to the next sibling.
//! Each row is a live yq v4.53.3 capture.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, expected output).
const ROWS: &[(&str, &str, &str)] = &[
    (
        "m:\n  # a\n\n  k: 1\nz: 2\n",
        ".",
        "m:\n  # a\n  k: 1\nz: 2\n",
    ),
    (
        "m:\n  # a\n\n  k: 1\nz: 2\n",
        ".zz = 1",
        "m:\n  # a\n  k: 1\nz: 2\nzz: 1\n",
    ),
    (
        "x: 0\nm:\n  # a\n\n  k: 1\nz: 2\n",
        ".",
        "x: 0\nm:\n  # a\n  k: 1\nz: 2\n",
    ),
    (
        "x: 0\nm:\n  # a\n\n  k: 1\nz: 2\n",
        ".zz = 1",
        "x: 0\nm:\n  # a\n  k: 1\nz: 2\nzz: 1\n",
    ),
    (
        "m:\n  # a\n\n  - 1\nz: 2\n",
        ".",
        "m:\n  # a\n  - 1\nz: 2\n",
    ),
    (
        "m:\n  # a\n\n  - 1\nz: 2\n",
        ".zz = 1",
        "m:\n  # a\n  - 1\nz: 2\nzz: 1\n",
    ),
    (
        "m:\n  n:\n    # a\n\n    k: 1\n",
        ".",
        "m:\n  n:\n    # a\n    k: 1\n",
    ),
    (
        "m:\n  n:\n    # a\n\n    k: 1\n",
        ".zz = 1",
        "m:\n  n:\n    # a\n    k: 1\nzz: 1\n",
    ),
    (
        "- m:\n    # a\n\n    k: 1\n",
        ".",
        "- m:\n    # a\n    k: 1\n",
    ),
    (
        "m:\n    # a\n\n  k: 1\nz: 2\n",
        ".",
        "m:\n  # a\n  k: 1\nz: 2\n",
    ),
    (
        "m:\n    # a\n\n  k: 1\nz: 2\n",
        ".zz = 1",
        "m:\n  # a\n  k: 1\nz: 2\nzz: 1\n",
    ),
    (
        "m:\n  # a\n\n\n  k: 1\nz: 2\n",
        ".",
        "m:\n  # a\n  k: 1\nz: 2\n",
    ),
    (
        "m:\n  # a\n\n\n  k: 1\nz: 2\n",
        ".zz = 1",
        "m:\n  # a\n  k: 1\nz: 2\nzz: 1\n",
    ),
    (
        "m:\n# a\n\n  k: 1\nz: 2\n",
        ".",
        "m:\n  # a\n  k: 1\nz: 2\n",
    ),
    (
        "m:\n# a\n\n  k: 1\nz: 2\n",
        ".zz = 1",
        "m:\n  # a\n  k: 1\nz: 2\nzz: 1\n",
    ),
    (
        "m:\n  # a\n  # b\n\n  k: 1\nz: 2\n",
        ".",
        "m:\n  # a\n  # b\n  k: 1\nz: 2\n",
    ),
    (
        "m:\n  # a\n  # b\n\n  k: 1\nz: 2\n",
        ".zz = 1",
        "m:\n  # a\n  # b\n  k: 1\nz: 2\nzz: 1\n",
    ),
    (
        "m:\n  # a\n\n  k: 1\n  j: 2\nz: 2\n",
        ".",
        "m:\n  # a\n  k: 1\n  j: 2\nz: 2\n",
    ),
    (
        "m:\n  # a\n\n  k: 1\n  j: 2\nz: 2\n",
        ".zz = 1",
        "m:\n  # a\n  k: 1\n  j: 2\nz: 2\nzz: 1\n",
    ),
    (
        "m: &x\n  # a\n\n  k: 1\nz: 2\n",
        ".",
        "m: &x\n  # a\n  k: 1\nz: 2\n",
    ),
    (
        "m: &x\n  # a\n\n  k: 1\nz: 2\n",
        ".zz = 1",
        "m: &x\n  # a\n  k: 1\nz: 2\nzz: 1\n",
    ),
];

#[test]
fn test_comment_before_a_blank_line_heads_the_first_child_4996() -> Result<()> {
    for (doc, filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{filter} over {doc:?}"
        );
    }
    Ok(())
}

/// The comment is the first child key's head, not the parent's foot. Its text ends in the line
/// break the blank line leaves (`"a\n"`, printed `a` and two line ends), as in yq (#7034).
#[test]
fn test_comment_before_a_blank_line_is_the_first_childs_head_comment_4996() -> Result<()> {
    let doc = "m:\n  # a\n\n  k: 1\nz: 2\n";
    for (filter, expected) in [
        (".m.k | key | head_comment", "a\n\n"),
        (".m | key | foot_comment", "\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}");
    }
    Ok(())
}
