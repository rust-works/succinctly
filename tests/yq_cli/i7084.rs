//! #7084: behind a UTF-8 byte-order mark yq sets no header aside (#6762), so the comment block at the top
//! of the first document is placed by go-yaml's ordinary rules: forward onto the first node, except the
//! blocks a blank line separates from what follows, which are the document node's own head. Each row is a
//! live yq v4.53.3 capture (`printf '\xef\xbb\xbf<text>' | yq <flags>`), the filter `F` being
//! `[head_comment, foot_comment]` of the document and of each key and item.

use anyhow::Result;

use crate::run_yq_stdin;

const BOM: &str = "\u{feff}";

/// The head and foot comment of the document, of each mapping key and of each sequence item.
const F: &str = r#"[head_comment, foot_comment] + [.. | select(tag=="!!map") | keys[] | [head_comment, foot_comment]] + [.. | select(tag=="!!seq") | .[] | [head_comment, foot_comment]]"#;

/// `(text after the mark, `"F"` or the flags before `.`, yq's output)`.
const ROWS: &[(&str, &str, &str)] = &[
    (
        "# lead\na: 1 # one\n",
        "F",
        "[\"\",\"\",[\"lead\",\"\"],[]]\n",
    ),
    (
        "# l1\n# l2\na: 1\n",
        "F",
        "[\"\",\"\",[\"l1\\nl2\",\"\"],[]]\n",
    ),
    (
        "# lead\n---\na: 1\n",
        "F",
        "[\"\",\"\",[\"lead\",\"\"],[]]\n",
    ),
    (
        "# lead\na:\n  b: 1\n",
        "F",
        "[\"\",\"\",[\"lead\",\"\"],[\"\",\"\"],[]]\n",
    ),
    // A block a blank line detaches is the document's head, and the next one the first node's.
    (
        "# l1\n\n# l2\na: 1\n",
        "F",
        "[\"l1\",\"\",[\"l2\",\"\"],[]]\n",
    ),
    ("# lead\n\na: 1\n", "F", "[\"lead\",\"\",[\"\",\"\"],[]]\n"),
    (
        "# l1\n\n# l2\n\na: 1\n",
        "F",
        "[\"l1\\n\\nl2\",\"\",[\"\",\"\"],[]]\n",
    ),
    (
        "# l1\n# l2\n\n# l3\na: 1\n",
        "F",
        "[\"l1\\nl2\",\"\",[\"l3\",\"\"],[]]\n",
    ),
    (
        "# l1\n\n# l2\n\n# l3\na: 1\n",
        "F",
        "[\"l1\\n\\nl2\",\"\",[\"l3\",\"\"],[]]\n",
    ),
    (
        "# l1\n\n# l2\nk:\n  a: 1\n",
        "F",
        "[\"l1\",\"\",[\"l2\",\"\"],[\"\",\"\"],[]]\n",
    ),
    // A sequence: the first item takes the block that is not detached.
    ("# lead\n- 1\n", "F", "[\"\",\"\",[],[\"lead\",\"\"]]\n"),
    (
        "# l1\n\n# l2\n- 1\n",
        "F",
        "[\"l1\",\"\",[],[\"l2\",\"\"]]\n",
    ),
    ("# l1\n\n- 1\n", "F", "[\"l1\",\"\",[],[\"\",\"\"]]\n"),
    // Under an explicit marker the blocks go forward together, blank lines kept; one above the marker
    // keeps the line break a blank line leaves under it.
    (
        "---\n# l1\n\n# l2\na: 1\n",
        "F",
        "[\"\",\"\",[\"l1\\n\\nl2\",\"\"],[]]\n",
    ),
    (
        "# c1\n\n---\na: 1\n",
        "F",
        "[\"\",\"\",[\"c1\\n\",\"\"],[]]\n",
    ),
    (
        "# c1\n---\n# c2\na: 1\n",
        "F",
        "[\"\",\"\",[\"c1\\nc2\",\"\"],[]]\n",
    ),
    (
        "# c1\n\n---\n# c2\na: 1\n",
        "F",
        "[\"\",\"\",[\"c1\\n\\nc2\",\"\"],[]]\n",
    ),
    // What is printed: a scalar document drops its head, and the blank line between two blocks goes
    // with the comment that is the document's own.
    ("# lead\n5\n", ".", "5\n"),
    ("# lead\n\n5\n", ".", "5\n"),
    ("# l1\n\n# l2\na: 1\n", ".", "# l1\n# l2\na: 1\n"),
    ("# lead\na: 1\n", ".", "# lead\na: 1\n"),
    ("# l1\n\n# l2\na: 1\n", "-P", "# l1\n# l2\na: 1\n"),
    ("# lead\na: 1 # one\n", "-o=props", "# lead one\na = 1\n"),
    ("# lead\n- 1\n", "-o=props", "# lead\n0 = 1\n"),
];

#[test]
fn the_first_documents_leading_block_is_placed_by_the_ordinary_rules_7084() -> Result<()> {
    for (text, mode, want) in ROWS {
        let input = format!("{BOM}{text}");
        let (stdout, code) = match *mode {
            "F" => run_yq_stdin(F, &input, &["-o=json", "-I0"])?,
            "." => run_yq_stdin(".", &input, &[])?,
            flag => run_yq_stdin(".", &input, &[flag])?,
        };
        assert_eq!((stdout.as_str(), code), (*want, 0), "{mode} on {text:?}");
    }
    Ok(())
}

/// A leading comment the mark's absence reads as the header is unchanged: the same text without the
/// mark has it as the document's head, whatever follows.
#[test]
fn without_a_mark_the_leading_block_is_the_documents_head_7084() -> Result<()> {
    for (text, want) in [
        ("# lead\na: 1\n", "[\"lead\",\"\",[\"\",\"\"]]\n"),
        (
            "# l1\n\n# l2\na: 1\n",
            "[\"l1\\n\\nl2\",\"\",[\"\",\"\"]]\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(
            r#"[head_comment, foot_comment] + [.. | select(tag=="!!map") | keys[] | [head_comment, foot_comment]]"#,
            text,
            &["-o=json", "-I0"],
        )?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{text:?}");
    }
    Ok(())
}
