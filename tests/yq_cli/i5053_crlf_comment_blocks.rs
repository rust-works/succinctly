//! #5053: in a CRLF file, go-yaml reads the `\r\n` that ends a standalone comment line as two breaks, so yq
//! prints a blank line after every such comment the source never had: a block of comments has a blank line
//! between its lines, and a lone comment between two siblings is followed by one. A comment after a node
//! (`a: 1 # t`), a `#` line inside a block scalar and an LF file are not affected. Yq on a CRLF document is yq
//! on the LF document with one blank line added after each standalone comment line (196 documents over seven
//! shapes, one to three comments and every blank-line gap, no difference), which is how the rows were chosen.
//! Each row is a live yq v4.53.3 capture, on the streaming route (`.`), the DOM route (`-P`) and a write
//! (`.z = 1`).

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, flags, expected output).
const ROWS: &[(&str, &str, &[&str], &str)] = &[
    // lone comment between siblings
    ("a: 1\r\n# mid\r\nb: 2\r\n", ".", &[], "a: 1\n# mid\n\nb: 2\n"),
    // lone comment between siblings
    ("a: 1\r\n# mid\r\nb: 2\r\n", ".", &["-P"], "a: 1\n# mid\n\nb: 2\n"),
    // lone comment between siblings
    ("a: 1\r\n# mid\r\nb: 2\r\n", ".z = 1", &[], "a: 1\n# mid\n\nb: 2\nz: 1\n"),
    // two comments between siblings
    ("a: 1\r\n# c1\r\n# c2\r\nb: 2\r\n", ".", &[], "a: 1\n# c1\n\n# c2\nb: 2\n"),
    // two comments between siblings
    ("a: 1\r\n# c1\r\n# c2\r\nb: 2\r\n", ".", &["-P"], "a: 1\n# c1\n\n# c2\nb: 2\n"),
    // two comments between siblings
    ("a: 1\r\n# c1\r\n# c2\r\nb: 2\r\n", ".z = 1", &[], "a: 1\n# c1\n\n# c2\nb: 2\nz: 1\n"),
    // three comments between siblings
    ("a: 1\r\n# c1\r\n# c2\r\n# c3\r\nb: 2\r\n", ".", &[], "a: 1\n# c1\n\n# c2\n\n# c3\nb: 2\n"),
    // three comments between siblings
    ("a: 1\r\n# c1\r\n# c2\r\n# c3\r\nb: 2\r\n", ".", &["-P"], "a: 1\n# c1\n\n# c2\n\n# c3\nb: 2\n"),
    // three comments between siblings
    ("a: 1\r\n# c1\r\n# c2\r\n# c3\r\nb: 2\r\n", ".z = 1", &[], "a: 1\n# c1\n\n# c2\n\n# c3\nb: 2\nz: 1\n"),
    // blank above two comments
    ("a: 1\r\n\r\n# c1\r\n# c2\r\nb: 2\r\n", ".", &[], "a: 1\n# c1\n\n# c2\nb: 2\n"),
    // blank above two comments
    ("a: 1\r\n\r\n# c1\r\n# c2\r\nb: 2\r\n", ".", &["-P"], "a: 1\n# c1\n\n# c2\nb: 2\n"),
    // blank above two comments
    ("a: 1\r\n\r\n# c1\r\n# c2\r\nb: 2\r\n", ".z = 1", &[], "a: 1\n# c1\n\n# c2\nb: 2\nz: 1\n"),
    // blank between two comments
    ("a: 1\r\n# c1\r\n\r\n# c2\r\nb: 2\r\n", ".", &[], "a: 1\n# c1\n\n# c2\nb: 2\n"),
    // blank between two comments
    ("a: 1\r\n# c1\r\n\r\n# c2\r\nb: 2\r\n", ".", &["-P"], "a: 1\n# c1\n\n# c2\nb: 2\n"),
    // blank between two comments
    ("a: 1\r\n# c1\r\n\r\n# c2\r\nb: 2\r\n", ".z = 1", &[], "a: 1\n# c1\n\n# c2\nb: 2\nz: 1\n"),
    // blank below two comments
    ("a: 1\r\n# c1\r\n# c2\r\n\r\nb: 2\r\n", ".", &[], "a: 1\n# c1\n\n# c2\nb: 2\n"),
    // nested siblings
    ("x:\r\n  k: 1\r\n  # c1\r\n  # c2\r\n  j: 2\r\n", ".", &[], "x:\n  k: 1\n  # c1\n\n  # c2\n  j: 2\n"),
    // nested siblings
    ("x:\r\n  k: 1\r\n  # c1\r\n  # c2\r\n  j: 2\r\n", ".", &["-P"], "x:\n  k: 1\n  # c1\n\n  # c2\n  j: 2\n"),
    // nested siblings
    ("x:\r\n  k: 1\r\n  # c1\r\n  # c2\r\n  j: 2\r\n", ".z = 1", &[], "x:\n  k: 1\n  # c1\n\n  # c2\n  j: 2\nz: 1\n"),
    // first child
    ("x:\r\n  # c1\r\n  # c2\r\n  k: 1\r\n", ".", &[], "x:\n  # c1\n\n  # c2\n  k: 1\n"),
    // first child
    ("x:\r\n  # c1\r\n  # c2\r\n  k: 1\r\n", ".", &["-P"], "x:\n  # c1\n\n  # c2\n  k: 1\n"),
    // first child
    ("x:\r\n  # c1\r\n  # c2\r\n  k: 1\r\n", ".z = 1", &[], "x:\n  # c1\n\n  # c2\n  k: 1\nz: 1\n"),
    // first child, one comment
    ("x:\r\n  # c1\r\n  k: 1\r\n", ".", &[], "x:\n  # c1\n  k: 1\n"),
    // first child, one comment
    ("x:\r\n  # c1\r\n  k: 1\r\n", ".", &["-P"], "x:\n  # c1\n  k: 1\n"),
    // first child, one comment
    ("x:\r\n  # c1\r\n  k: 1\r\n", ".z = 1", &[], "x:\n  # c1\n  k: 1\nz: 1\n"),
    // sequence items
    ("- a\r\n# c1\r\n# c2\r\n- b\r\n", ".", &[], "- a\n# c1\n\n# c2\n- b\n"),
    // sequence items
    ("- a\r\n# c1\r\n# c2\r\n- b\r\n", ".", &["-P"], "- a\n# c1\n\n# c2\n- b\n"),
    // sequence under a key
    ("s:\r\n  - a\r\n  # c1\r\n  # c2\r\n  - b\r\n", ".", &[], "s:\n  - a\n  # c1\n\n  # c2\n  - b\n"),
    // sequence under a key
    ("s:\r\n  - a\r\n  # c1\r\n  # c2\r\n  - b\r\n", ".", &["-P"], "s:\n  - a\n  # c1\n\n  # c2\n  - b\n"),
    // sequence under a key
    ("s:\r\n  - a\r\n  # c1\r\n  # c2\r\n  - b\r\n", ".z = 1", &[], "s:\n  - a\n  # c1\n\n  # c2\n  - b\nz: 1\n"),
    // end of document
    ("a: 1\r\n# c1\r\n# c2\r\n", ".", &[], "a: 1\n# c1\n# c2\n"),
    // end of document
    ("a: 1\r\n# c1\r\n# c2\r\n", ".", &["-P"], "a: 1\n# c1\n# c2\n"),
    // end of document
    ("a: 1\r\n# c1\r\n# c2\r\n", ".z = 1", &[], "a: 1\n# c1\n\nz: 1\n# c2\n"),
    // end of document, three
    ("a: 1\r\n# c1\r\n# c2\r\n# c3\r\n", ".", &[], "a: 1\n# c1\n# c2\n\n# c3\n"),
    // end of document, three
    ("a: 1\r\n# c1\r\n# c2\r\n# c3\r\n", ".", &["-P"], "a: 1\n# c1\n# c2\n\n# c3\n"),
    // end of document, three
    ("a: 1\r\n# c1\r\n# c2\r\n# c3\r\n", ".z = 1", &[], "a: 1\n# c1\n\nz: 1\n# c2\n\n# c3\n"),
    // dedent to a top-level key
    ("x:\r\n  k: 1\r\n# c1\r\n# c2\r\ny: 2\r\n", ".", &[], "x:\n  k: 1\n# c1\n\n# c2\ny: 2\n"),
    // dedent to a top-level key
    ("x:\r\n  k: 1\r\n# c1\r\n# c2\r\ny: 2\r\n", ".", &["-P"], "x:\n  k: 1\n# c1\n\n# c2\ny: 2\n"),
    // dedent to a top-level key
    ("x:\r\n  k: 1\r\n# c1\r\n# c2\r\ny: 2\r\n", ".z = 1", &[], "x:\n  k: 1\n# c1\n\n# c2\ny: 2\nz: 1\n"),
    // dedent, three comments
    ("x:\r\n  k: 1\r\n# c1\r\n# c2\r\n# c3\r\ny: 2\r\n", ".", &[], "x:\n  k: 1\n# c1\n\n# c2\n\n# c3\ny: 2\n"),
    // dedent, three comments
    ("x:\r\n  k: 1\r\n# c1\r\n# c2\r\n# c3\r\ny: 2\r\n", ".", &["-P"], "x:\n  k: 1\n# c1\n\n# c2\n\n# c3\ny: 2\n"),
    // dedent, three comments
    ("x:\r\n  k: 1\r\n# c1\r\n# c2\r\n# c3\r\ny: 2\r\n", ".z = 1", &[], "x:\n  k: 1\n# c1\n\n# c2\n\n# c3\ny: 2\nz: 1\n"),
    // dedent with blanks
    ("x:\r\n  k: 1\r\n# c1\r\n\r\n# c2\r\ny: 2\r\n", ".", &[], "x:\n  k: 1\n# c1\n\n# c2\ny: 2\n"),
    // dedent with blanks
    ("x:\r\n  k: 1\r\n# c1\r\n\r\n# c2\r\ny: 2\r\n", ".", &["-P"], "x:\n  k: 1\n# c1\n\n# c2\ny: 2\n"),
    // dedent with blanks
    ("x:\r\n  k: 1\r\n# c1\r\n\r\n# c2\r\ny: 2\r\n", ".z = 1", &[], "x:\n  k: 1\n# c1\n\n# c2\ny: 2\nz: 1\n"),
    // trailing comment is not standalone
    ("a: 1 # t\r\nb: 2\r\n", ".", &[], "a: 1 # t\nb: 2\n"),
    // trailing comment is not standalone
    ("a: 1 # t\r\nb: 2\r\n", ".", &["-P"], "a: 1 # t\nb: 2\n"),
    // trailing comment is not standalone
    ("a: 1 # t\r\nb: 2\r\n", ".z = 1", &[], "a: 1 # t\nb: 2\nz: 1\n"),
    // trailing comment then a comment
    ("a: 1 # t\r\n# c1\r\n# c2\r\nb: 2\r\n", ".", &[], "a: 1 # t\n# c1\n\n# c2\nb: 2\n"),
    // trailing comment then a comment
    ("a: 1 # t\r\n# c1\r\n# c2\r\nb: 2\r\n", ".", &["-P"], "a: 1 # t\n# c1\n\n# c2\nb: 2\n"),
    // trailing comment then a comment
    ("a: 1 # t\r\n# c1\r\n# c2\r\nb: 2\r\n", ".z = 1", &[], "a: 1 # t\n# c1\n\n# c2\nb: 2\nz: 1\n"),
    // block scalar lines that look like comments
    ("k: |\r\n  # not\r\n  # a comment\r\nj: 1\r\n", ".", &[], "k: |\n  # not\n  # a comment\nj: 1\n"),
    // block scalar lines that look like comments
    ("k: |\r\n  # not\r\n  # a comment\r\nj: 1\r\n", ".", &["-P"], "k: |\n  # not\n  # a comment\nj: 1\n"),
    // block scalar lines that look like comments
    ("k: |\r\n  # not\r\n  # a comment\r\nj: 1\r\n", ".z = 1", &[], "k: |\n  # not\n  # a comment\nj: 1\nz: 1\n"),
    // block scalar then comments
    ("k: |\r\n  text\r\n# c1\r\n# c2\r\nj: 1\r\n", ".", &[], "k: |\n  text\n# c1\n\n# c2\nj: 1\n"),
    // block scalar then comments
    ("k: |\r\n  text\r\n# c1\r\n# c2\r\nj: 1\r\n", ".", &["-P"], "k: |\n  text\n# c1\n\n# c2\nj: 1\n"),
    // block scalar then comments
    ("k: |\r\n  text\r\n# c1\r\n# c2\r\nj: 1\r\n", ".z = 1", &[], "k: |\n  text\n# c1\n\n# c2\nj: 1\nz: 1\n"),
    // comment above the first key only
    ("# c1\r\na: 1\r\nb: 2\r\n", ".", &[], "# c1\r\na: 1\nb: 2\n"),
    // comment above the first key only
    ("# c1\r\na: 1\r\nb: 2\r\n", ".", &["-P"], "# c1\r\na: 1\nb: 2\n"),
    // comment above the first key only
    ("# c1\r\na: 1\r\nb: 2\r\n", ".z = 1", &[], "# c1\r\na: 1\nb: 2\nz: 1\n"),
    // workflow, run after comments
    ("jobs:\r\n  b:\r\n    steps:\r\n      - name: n\r\n        shell: bash\r\n        # Point cargo at a copy\r\n        # at 1.10.0.\r\n        # supported from 1.12.0\r\n        run: x\r\n", ".", &[], "jobs:\n  b:\n    steps:\n      - name: n\n        shell: bash\n        # Point cargo at a copy\n\n        # at 1.10.0.\n\n        # supported from 1.12.0\n        run: x\n"),
    // workflow, run after comments
    ("jobs:\r\n  b:\r\n    steps:\r\n      - name: n\r\n        shell: bash\r\n        # Point cargo at a copy\r\n        # at 1.10.0.\r\n        # supported from 1.12.0\r\n        run: x\r\n", ".", &["-P"], "jobs:\n  b:\n    steps:\n      - name: n\n        shell: bash\n        # Point cargo at a copy\n\n        # at 1.10.0.\n\n        # supported from 1.12.0\n        run: x\n"),
    // workflow, run after comments
    ("jobs:\r\n  b:\r\n    steps:\r\n      - name: n\r\n        shell: bash\r\n        # Point cargo at a copy\r\n        # at 1.10.0.\r\n        # supported from 1.12.0\r\n        run: x\r\n", ".z = 1", &[], "jobs:\n  b:\n    steps:\n      - name: n\n        shell: bash\n        # Point cargo at a copy\n\n        # at 1.10.0.\n\n        # supported from 1.12.0\n        run: x\nz: 1\n"),
];

#[test]
fn test_a_crlf_after_a_standalone_comment_line_is_a_blank_line_5053() -> Result<()> {
    for (doc, filter, flags, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, doc, flags)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{doc:?} {filter} {flags:?}"
        );
    }
    Ok(())
}

/// The LF rows that a column-0 comment block, split by a blank line after nested content, used to fail: the
/// first block is the foot of the key that owns the open mapping at its column (`x`), not of the innermost
/// key (`k`), so it prints at column 0 (yq v4.53.3). The CRLF form of the same documents is a blank line
/// after every comment line, and prints the same.
#[test]
fn test_a_dedented_comment_block_split_by_a_blank_line_is_the_outer_keys_foot_5053() -> Result<()> {
    let rows: &[(&str, &str)] = &[
        (
            "x:\n  k: 1\n# c1\n\n# c2\ny: 2\n",
            "x:\n  k: 1\n# c1\n\n# c2\ny: 2\n",
        ),
        (
            "x:\n  k: 1\n# c1\n\n# c2\n\n# c3\ny: 2\n",
            "x:\n  k: 1\n# c1\n\n# c2\n\n# c3\ny: 2\n",
        ),
        (
            "x:\n  k: 1\n\n# c1\n\n# c2\ny: 2\n",
            "x:\n  k: 1\n# c1\n\n# c2\ny: 2\n",
        ),
    ];
    for (doc, expected) in rows {
        let (stdout, code) = run_yq_stdin(".", doc, &[])?;
        assert_eq!((stdout.as_str(), code), (*expected, 0), "{doc:?}");
    }
    Ok(())
}
