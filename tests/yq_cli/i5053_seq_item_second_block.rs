//! #5053: the second comment block of a run, after a blank line, that dedents to the next item of a sequence is
//! the foot of that item's first key in yq, not of an outer key (`s:` / `- a: 1` / ... / `# c1` / blank / `# c2` /
//! blank / `- b: 2` prints `# c2` after `- b: 2` with a blank line below it). GitHub's default CodeQL workflow has
//! exactly this shape after `languages:`. The rows also pin what must not change: a next item that is a sibling in the
//! sequence the first block closed, a block at or left of the item's column, and a scalar item, in LF and CRLF alike. Each
//! row is a live yq v4.53.3 capture.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, expected output).
const ROWS: &[(&str, &str)] = &[
    ("s:\n- a: 1\n  w:\n    l: x\n    # c1\n\n    # c2\n\n- b: 2\n  u: 3\n", "s:\n  - a: 1\n    w:\n      l: x\n      # c1\n  - b: 2\n    # c2\n\n    u: 3\n"),
    ("s:\r\n- a: 1\r\n  w:\r\n    l: x\r\n    # c1\r\n\r\n    # c2\r\n\r\n- b: 2\r\n  u: 3\r\n", "s:\n  - a: 1\n    w:\n      l: x\n      # c1\n  - b: 2\n    # c2\n\n    u: 3\n"),
    ("s:\n- a: 1\n  w:\n    l: x\n    # c1\n\n    # c2\n- b: 2\n  u: 3\n", "s:\n  - a: 1\n    w:\n      l: x\n      # c1\n  - b: 2\n    # c2\n\n    u: 3\n"),
    ("s:\r\n- a: 1\r\n  w:\r\n    l: x\r\n    # c1\r\n\r\n    # c2\r\n- b: 2\r\n  u: 3\r\n", "s:\n  - a: 1\n    w:\n      l: x\n      # c1\n  - b: 2\n    # c2\n\n    u: 3\n"),
    ("s:\n  - a: 1\n    w:\n      l: x\n      # c1\n\n      # c2\n\n  - b: 2\n    u: 3\n", "s:\n  - a: 1\n    w:\n      l: x\n      # c1\n  - b: 2\n    # c2\n\n    u: 3\n"),
    ("s:\r\n  - a: 1\r\n    w:\r\n      l: x\r\n      # c1\r\n\r\n      # c2\r\n\r\n  - b: 2\r\n    u: 3\r\n", "s:\n  - a: 1\n    w:\n      l: x\n      # c1\n  - b: 2\n    # c2\n\n    u: 3\n"),
    ("w:\n  l:\n  - x\n  # c1\n\n    # c2\n\n  - y\n", "w:\n  l:\n    - x\n    # c1\n\n    # c2\n    - y\n"),
    ("w:\r\n  l:\r\n  - x\r\n  # c1\r\n\r\n    # c2\r\n\r\n  - y\r\n", "w:\n  l:\n    - x\n    # c1\n\n    # c2\n    - y\n"),
    ("w:\n  l:\n  - x\n  # c1\n\n    # c2\n\n  - y: 1\n", "w:\n  l:\n    - x\n    # c1\n\n    # c2\n    - y: 1\n"),
    ("w:\r\n  l:\r\n  - x\r\n  # c1\r\n\r\n    # c2\r\n\r\n  - y: 1\r\n", "w:\n  l:\n    - x\n    # c1\n\n    # c2\n    - y: 1\n"),
    ("w:\n  l:\n  - x\n  # c1\n\n      # c2\n\n  - y\n", "w:\n  l:\n    - x\n    # c1\n\n    # c2\n    - y\n"),
    ("w:\r\n  l:\r\n  - x\r\n  # c1\r\n\r\n      # c2\r\n\r\n  - y\r\n", "w:\n  l:\n    - x\n    # c1\n\n    # c2\n    - y\n"),
    ("s:\n  - a\n  # c1\n\n  # c2\n\n  - b\n", "s:\n  - a\n  # c1\n\n  # c2\n  - b\n"),
    ("s:\r\n  - a\r\n  # c1\r\n\r\n  # c2\r\n\r\n  - b\r\n", "s:\n  - a\n  # c1\n\n  # c2\n  - b\n"),
    ("s:\n- a\n# c1\n\n# c2\n\n- b\n", "s:\n  - a\n  # c1\n\n  # c2\n  - b\n"),
    ("s:\r\n- a\r\n# c1\r\n\r\n# c2\r\n\r\n- b\r\n", "s:\n  - a\n  # c1\n\n  # c2\n  - b\n"),
];

#[test]
fn test_a_second_comment_block_before_the_next_sequence_item_is_its_first_keys_foot_5053(
) -> Result<()> {
    for (doc, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(".", doc, &[])?;
        assert_eq!((stdout.as_str(), code), (*expected, 0), "{doc:?}");
    }
    Ok(())
}
