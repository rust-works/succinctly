//! #5053: a comment block that a blank line separates from the next one, after a nested block sequence, is the
//! foot of the key that owns the sequence in yq (`steps:` / `- run: x` / `# t1` / blank / `# t2` prints `# t1` at
//! `steps`' column), not of the innermost key, which printed it at the item's column. A Windows-authored GitHub
//! Actions file (`rust-1.12.yml` of the `vcpkg` crate) hits it at the end of the file, where CRLF makes every comment
//! line a block. Each row is a live yq v4.53.3 capture, in LF and CRLF.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, expected output).
const ROWS: &[(&str, &str)] = &[
    ("j:\n  b:\n    steps:\n      - run: x\n      # t1\n\n      # t2\n\n      #   t3\n", "j:\n  b:\n    steps:\n      - run: x\n    # t1\n# t2\n\n#   t3\n"),
    ("j:\r\n  b:\r\n    steps:\r\n      - run: x\r\n      # t1\r\n\r\n      # t2\r\n\r\n      #   t3\r\n", "j:\n  b:\n    steps:\n      - run: x\n    # t1\n# t2\n\n#   t3\n"),
    ("j:\n  b:\n    steps:\n      - run: x\n      # t1\n\n      # t2\n", "j:\n  b:\n    steps:\n      - run: x\n    # t1\n# t2\n"),
    ("j:\r\n  b:\r\n    steps:\r\n      - run: x\r\n      # t1\r\n\r\n      # t2\r\n", "j:\n  b:\n    steps:\n      - run: x\n    # t1\n# t2\n"),
    ("j:\n  - run: x\n  # t1\n\n  # t2\n", "j:\n  - run: x\n# t1\n# t2\n"),
    ("j:\r\n  - run: x\r\n  # t1\r\n\r\n  # t2\r\n", "j:\n  - run: x\n# t1\n# t2\n"),
    ("j:\n  k:\n    - run: x\n    # t1\n\n    # t2\n\n    # t3\n", "j:\n  k:\n    - run: x\n  # t1\n# t2\n\n# t3\n"),
    ("j:\r\n  k:\r\n    - run: x\r\n    # t1\r\n\r\n    # t2\r\n\r\n    # t3\r\n", "j:\n  k:\n    - run: x\n  # t1\n# t2\n\n# t3\n"),
    ("j:\n  - run: x\n  # t1\n  # t2\n", "j:\n  - run: x\n# t1\n# t2\n"),
    ("j:\r\n  - run: x\r\n  # t1\r\n  # t2\r\n", "j:\n  - run: x\n# t1\n# t2\n"),
    ("j:\n  k:\n    - run: |\n        t\n    # t1\n\n    # t2\n", "j:\n  k:\n    - run: |\n        t\n  # t1\n# t2\n"),
    ("j:\r\n  k:\r\n    - run: |\r\n        t\r\n    # t1\r\n\r\n    # t2\r\n", "j:\n  k:\n    - run: |\n        t\n  # t1\n# t2\n"),
];

#[test]
fn test_a_blank_split_comment_block_after_a_sequence_is_its_owner_keys_foot_5053() -> Result<()> {
    for (doc, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(".", doc, &[])?;
        assert_eq!((stdout.as_str(), code), (*expected, 0), "{doc:?}");
    }
    Ok(())
}
