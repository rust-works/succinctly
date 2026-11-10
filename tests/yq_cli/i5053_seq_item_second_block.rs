//! #5053: the second comment block of a run, after a blank line, that dedents to the next item of a sequence is
//! the foot of that item's first key in yq, not of an outer key (`s:` / `- a: 1` / ... / `# c1` / blank / `# c2` /
//! blank / `- b: 2` prints `# c2` after `- b: 2` with a blank line below it). GitHub's default CodeQL workflow has
//! exactly this shape after `languages:`. Each row is a live yq v4.53.3 capture.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, expected output).
const ROWS: &[(&str, &str)] = &[
    ("s:\n- a: 1\n  w:\n    l: x\n    # c1\n\n    # c2\n\n- b: 2\n  u: 3\n", "s:\n  - a: 1\n    w:\n      l: x\n      # c1\n  - b: 2\n    # c2\n\n    u: 3\n"),
    ("s:\n  - a: 1\n    w:\n      l: x\n      # c1\n\n      # c2\n\n  - b: 2\n    u: 3\n", "s:\n  - a: 1\n    w:\n      l: x\n      # c1\n  - b: 2\n    # c2\n\n    u: 3\n"),
    ("s:\n- a: 1\n  w:\n    l: x\n    # c1\n\n    # c2\n- b: 2\n  u: 3\n", "s:\n  - a: 1\n    w:\n      l: x\n      # c1\n  - b: 2\n    # c2\n\n    u: 3\n"),
    ("s:\n- a: 1\n  w:\n    l: x\n    # c1\n\n    # c2\n\n- b: 2\n", "s:\n  - a: 1\n    w:\n      l: x\n      # c1\n  - b: 2\n    # c2\n"),
    ("s:\n- a: 1\n  w:\n    l: x\n    # c1\n\n    # c2\n\n    # c3\n\n- b: 2\n  u: 3\n", "s:\n  - a: 1\n    w:\n      l: x\n      # c1\n  - b: 2\n    # c2\n\n    # c3\n\n    u: 3\n"),
    ("s:\n- a: 1\n  w:\n    l: x\n    # c1\n\n    # c2\n\n- b\n", "s:\n  - a: 1\n    w:\n      l: x\n      # c1\n  - b\n  # c2\n"),
    ("s:\n- a: 1\n  # c1\n\n  # c2\n\n- b: 2\n  u: 3\n", "s:\n  - a: 1\n    # c1\n  - b: 2\n    # c2\n\n    u: 3\n"),
    ("s:\n- a: 1\n  w:\n    l: x\n    # c1\n\n    # c2\n\n- - b\n", "s:\n  - a: 1\n    w:\n      l: x\n      # c1\n  - - b\n    # c2\n"),
    ("    steps:\n    - name: Checkout repository\n      uses: actions/checkout@v4\n\n    # Initializes the CodeQL tools for scanning.\n    - name: Initialize CodeQL\n      uses: github/codeql-action/init@v3\n      with:\n        languages: ${{ matrix.language }}\n        # If you wish to specify custom queries, you can do so here or in a config file.\n        # By default, queries listed here will override any specified in a config file.\n        # Prefix the list here with \"+\" to use these queries and those in the config file.\n\n        # For more details on CodeQL's query packs, refer to: https://docs.github.com/x\n        # queries: security-extended,security-and-quality\n\n    - name: Perform CodeQL Analysis\n      uses: github/codeql-action/analyze@v3\n", "steps:\n  - name: Checkout repository\n    uses: actions/checkout@v4\n  # Initializes the CodeQL tools for scanning.\n  - name: Initialize CodeQL\n    uses: github/codeql-action/init@v3\n    with:\n      languages: ${{ matrix.language }}\n      # If you wish to specify custom queries, you can do so here or in a config file.\n      # By default, queries listed here will override any specified in a config file.\n      # Prefix the list here with \"+\" to use these queries and those in the config file.\n  - name: Perform CodeQL Analysis\n    # For more details on CodeQL's query packs, refer to: https://docs.github.com/x\n    # queries: security-extended,security-and-quality\n\n    uses: github/codeql-action/analyze@v3\n"),
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
