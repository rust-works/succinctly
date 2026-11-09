//! #6772: yq's `head_comment` of a document keeps the blank lines of the comment block: one after the
//! block ends the text with a line feed, one inside it stays, and a first document's comment above a
//! leading `---` is one text with the comment under it (`# c` / `---` / `# d` is `c\nd`), where the
//! first key the comment under the marker was indexed on has none of its own. `succinctly yq` answered
//! the comment lines alone. Every answer was captured from yq v4.53.3 (`-o=json -I=0`).

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_yq_head_comment_keeps_the_blank_lines_and_joins_a_leading_marker_6772() -> Result<()> {
    for (doc, filter, want) in [
        ("# c\na: 1\n", "head_comment", "\"c\"\n"),
        ("# c\na: 1\n", ".. | head_comment", "\"c\"\n\"\"\n"),
        ("# c\na: 1\n", ".a | key | head_comment", "\"\"\n"),
        ("# c\n\na: 1\n", "head_comment", "\"c\\n\"\n"),
        ("# c\n\na: 1\n", ".. | head_comment", "\"c\\n\"\n\"\"\n"),
        ("# c\n\na: 1\n", ".a | key | head_comment", "\"\"\n"),
        ("# c\n# d\n\na: 1\n", "head_comment", "\"c\\nd\\n\"\n"),
        (
            "# c\n# d\n\na: 1\n",
            ".. | head_comment",
            "\"c\\nd\\n\"\n\"\"\n",
        ),
        ("# c\n# d\n\na: 1\n", ".a | key | head_comment", "\"\"\n"),
        ("# c\n\n\na: 1\n", "head_comment", "\"c\\n\\n\"\n"),
        (
            "# c\n\n\na: 1\n",
            ".. | head_comment",
            "\"c\\n\\n\"\n\"\"\n",
        ),
        ("# c\n\n\na: 1\n", ".a | key | head_comment", "\"\"\n"),
        ("# c\n\n# d\na: 1\n", "head_comment", "\"c\\n\\nd\"\n"),
        (
            "# c\n\n# d\na: 1\n",
            ".. | head_comment",
            "\"c\\n\\nd\"\n\"\"\n",
        ),
        ("# c\n\n# d\na: 1\n", ".a | key | head_comment", "\"\"\n"),
        ("# c\n---\na: 1\n", "head_comment", "\"c\"\n"),
        ("# c\n---\na: 1\n", ".. | head_comment", "\"c\"\n\"\"\n"),
        ("# c\n---\na: 1\n", ".a | key | head_comment", "\"\"\n"),
        ("# c\n---\n# d\na: 1\n", "head_comment", "\"c\\nd\"\n"),
        (
            "# c\n---\n# d\na: 1\n",
            ".. | head_comment",
            "\"c\\nd\"\n\"\"\n",
        ),
        ("# c\n---\n# d\na: 1\n", ".a | key | head_comment", "\"\"\n"),
        ("# c\n\n---\n# d\na: 1\n", "head_comment", "\"c\\n\\nd\"\n"),
        (
            "# c\n\n---\n# d\na: 1\n",
            ".. | head_comment",
            "\"c\\n\\nd\"\n\"\"\n",
        ),
        (
            "# c\n\n---\n# d\na: 1\n",
            ".a | key | head_comment",
            "\"\"\n",
        ),
        ("# c\n---\n\na: 1\n", "head_comment", "\"c\\n\"\n"),
        (
            "# c\n---\n\na: 1\n",
            ".. | head_comment",
            "\"c\\n\"\n\"\"\n",
        ),
        ("# c\n---\n\na: 1\n", ".a | key | head_comment", "\"\"\n"),
        ("# c\n\n\n---\na: 1\n", "head_comment", "\"c\\n\\n\"\n"),
        (
            "# c\n\n\n---\na: 1\n",
            ".. | head_comment",
            "\"c\\n\\n\"\n\"\"\n",
        ),
        ("# c\n\n\n---\na: 1\n", ".a | key | head_comment", "\"\"\n"),
        (
            "# c\n---\n# d\na: 1\n---\n# e\na: 2\n",
            "head_comment",
            "\"c\\nd\"\n\"\"\n",
        ),
        (
            "# c\n---\n# d\na: 1\n---\n# e\na: 2\n",
            ".. | head_comment",
            "\"c\\nd\"\n\"\"\n\"\"\n\"\"\n",
        ),
        (
            "# c\n---\n# d\na: 1\n---\n# e\na: 2\n",
            ".a | key | head_comment",
            "\"\"\n\"e\"\n",
        ),
        ("a: 1\n---\n# c\nb: 2\n", "head_comment", "\"\"\n\"\"\n"),
        (
            "a: 1\n---\n# c\nb: 2\n",
            ".. | head_comment",
            "\"\"\n\"\"\n\"\"\n\"\"\n",
        ),
        (
            "a: 1\n---\n# c\nb: 2\n",
            ".a | key | head_comment",
            "\"\"\n\"\"\n",
        ),
        (
            "# welcome!\n\na: cat # meow\n\n# have a great day\n",
            "head_comment",
            "\"welcome!\\n\"\n",
        ),
        (
            "# welcome!\n\na: cat # meow\n\n# have a great day\n",
            ".. | head_comment",
            "\"welcome!\\n\"\n\"\"\n",
        ),
        (
            "# welcome!\n\na: cat # meow\n\n# have a great day\n",
            ".a | key | head_comment",
            "\"\"\n",
        ),
        (
            "# welcome!\n---\n# bob\na: cat # meow\n\n# have a great day\n",
            "head_comment",
            "\"welcome!\\nbob\"\n",
        ),
        (
            "# welcome!\n---\n# bob\na: cat # meow\n\n# have a great day\n",
            ".. | head_comment",
            "\"welcome!\\nbob\"\n\"\"\n",
        ),
        (
            "# welcome!\n---\n# bob\na: cat # meow\n\n# have a great day\n",
            ".a | key | head_comment",
            "\"\"\n",
        ),
        ("# c\n---\n# d\n- 1\n", "head_comment", "\"c\\nd\"\n"),
        (
            "# c\n---\n# d\n- 1\n",
            ".. | head_comment",
            "\"c\\nd\"\n\"\"\n",
        ),
        ("\n# c\n\na: 1\n", "head_comment", "\"\\nc\\n\"\n"),
        (
            "\n# c\n\na: 1\n",
            ".. | head_comment",
            "\"\\nc\\n\"\n\"\"\n",
        ),
        ("\n# c\n\na: 1\n", ".a | key | head_comment", "\"\"\n"),
        ("# c\n\n\"s\"\n", "head_comment", "\"c\\n\"\n"),
        ("# c\n\n\"s\"\n", ".. | head_comment", "\"c\\n\"\n"),
        ("# c\n\n\"s\"\n", ".a | key | head_comment", ""),
    ] {
        let (stdout, code) = run_yq_stdin(filter, doc, &["-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{doc:?}: {filter}");
    }
    Ok(())
}
