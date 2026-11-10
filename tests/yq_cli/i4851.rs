//! #4851: the shapes next to #2835's own-line node properties.
//!
//! * A comment after properties that stand alone on their line (`!!str # d`) is deferred past
//!   them, as it is for a property-prefixed item on the dash's own line: the properties end
//!   the line and the node they name is below, so the comment lands on whatever that node is
//!   (`b: 1 # d`). It used to be read as the empty node's own comment, which also swallowed the
//!   value that followed (`a:` / `  !!str # d` / `  b: 1` dropped `b`).
//! * A tag, or an anchor, given to a node on one line cannot be given again on the next: yq stops
//!   with `did not find expected key` / `'-' indicator`.
//!
//! Every output is captured from yq v4.53.3.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

/// The printed document for each input.
#[test]
fn test_4851_a_comment_after_own_line_properties_is_deferred_past_them() -> Result<()> {
    let rows: &[(&str, &str)] = &[
        ("a:\n  !!str # d\nb: 1\n", "a: !!str\nb: 1 # d\n"),
        ("a:\n  !!str # d\n  x\nb: 1\n", "a: !!str x # d\nb: 1\n"),
        ("a:\n  !!str # d\n  b: 1\n", "a: !!str\n  b: 1 # d\n"),
        ("a:\n  &x # d\nb: 1\n", "a: &x\nb: 1 # d\n"),
        (
            "a:\n  &x !!map # d\n  b: 1\nc: 2\n",
            "a: &x !!map\n  b: 1 # d\nc: 2\n",
        ),
        ("a:\n  !!str # d\n\nb: 1\n", "a: !!str\nb: 1 # d\n"),
        (
            "a:\n  !!str # d\n  # e\nb: 1\n",
            "a: !!str\n# e\nb: 1 # d\n",
        ),
        ("a:\n  !!str # d\n", "a: !!str\n"),
        (
            "a:\n  b:\n    !!str # d\nc: 1\n",
            "a:\n  b: !!str\nc: 1 # d\n",
        ),
        ("- # c\n  !!str # d\n- 2\n", "- !!str\n# c\n- 2 # d\n"),
        ("-\n  !!str # d\n  x\n- 2\n", "- !!str x # d\n- 2\n"),
        ("- a\n-\n  &x # d\n- 2\n", "- a\n- &x\n- 2 # d\n"),
        (
            "k:\n  - # c\n    !!str # d\nz: 1\n",
            "k:\n  - !!str\n# c\nz: 1 # d\n",
        ),
        ("a:\n  !!str   # d   \nb: 1\n", "a: !!str\nb: 1 # d   \n"),
    ];
    for (input, expected) in rows {
        let (out, code) = run_yq_stdin(".", input, &[])?;
        assert_eq!(code, 0, "{input:?}");
        assert_eq!(&out, expected, "{input:?}");
    }
    Ok(())
}

/// The data under a comment-after-properties line: the value that follows it was swallowed.
#[test]
fn test_4851_b_a_value_after_a_commented_properties_line_is_kept() -> Result<()> {
    let rows: &[(&str, &str)] = &[
        ("a:\n  !!str # d\n  x\nb: 1\n", "{\"a\":\"x\",\"b\":1}\n"),
        ("a:\n  !!str # d\n  b: 1\n", "{\"a\":{\"b\":1}}\n"),
        ("- !!str # d\n  x\n- 2\n", "[\"x\",2]\n"),
        ("-\n  !!str # d\n  x\n- 2\n", "[\"x\",2]\n"),
    ];
    for (input, expected) in rows {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "{input:?}");
        assert_eq!(&out, expected, "{input:?}");
    }
    Ok(())
}

/// Controls: a tag and an anchor on separate lines, in either order, are one node's properties.
#[test]
fn test_4851_c_a_tag_and_an_anchor_on_separate_lines_are_accepted() -> Result<()> {
    let rows: &[(&str, &str)] = &[
        ("a:\n  !!str\n  &x\nb: 1\n", "a: &x !!str\nb: 1\n"),
        ("a:\n  &x\n  !!str\n  v\nb: 1\n", "a: &x !!str v\nb: 1\n"),
        ("a: !!map\n  &x\n  b: 1\n", "a: &x !!map\n  b: 1\n"),
        ("a: &x\n  !!map\n  b: 1\n", "a: &x !!map\n  b: 1\n"),
        // A key that merely contains a tag or an anchor inside its quotes owns no properties.
        ("\"a: !x\":\n  !!str\n  v\n", "\"a: !x\": !!str v\n"),
        ("'k: &a':\n  &b\n  v\n", "'k: &a': &b v\n"),
        ("\"a: &x\":\n  &y\n  v\n", "\"a: &x\": &y v\n"),
        ("- \"a: !x\":\n    !!str\n    v\n", "- \"a: !x\": !!str v\n"),
        (
            "k:\n  \"note: !important\":\n    !!str\n    v\n",
            "k:\n  \"note: !important\": !!str v\n",
        ),
        // Two different nodes, each with one tag.
        ("a:\n  !!str\nb:\n  !!str\n", "a: !!str\nb: !!str\n"),
        ("a: !!str\n  x\nb: !!str\n  y\n", "a: !!str x\nb: !!str y\n"),
    ];
    for (input, expected) in rows {
        let (out, code) = run_yq_stdin(".", input, &[])?;
        assert_eq!(code, 0, "{input:?}");
        assert_eq!(&out, expected, "{input:?}");
    }
    Ok(())
}

/// A repeated tag or anchor is refused, with or without a value below, whether the first sits on
/// the owner's line or on a line of its own, and below a key or a dash.
#[test]
fn test_4851_d_a_repeated_property_kind_on_a_later_line_is_refused() -> Result<()> {
    let rows: &[&str] = &[
        "a:\n  !!str\n  !!int\nb: 1\n",
        "a:\n  !!str\n  !!int\n  x\nb: 1\n",
        "a:\n  !!str\n  !!int\n",
        "a:\n  !!str\n    !!int\nb: 1\n",
        "a:\n  !!str\n  !!int\n  b: 1\n",
        "a:\n  &x\n  &y\nb: 1\n",
        "a:\n  &x\n  &y\n  v\n",
        "a: !!str\n  !!int\nb: 1\n",
        "a: !!str\n  !!int\n  v\nb: 1\n",
        "a: &x\n  &y\nb: 1\n",
        "- !!str\n  !!int\n- 2\n",
        "-\n  !!str\n  !!int\n- 2\n",
        "-\n  !!str\n  !!int\n  v\n- 2\n",
        "- &x\n  &y\n  b: 1\n",
        // The owner's line ends in several properties.
        "k: &x !!map\n  &y\n  b: 1\n",
        "- !x &y\n  !z\n  v\n",
        "- - &x\n    &y\n    b: 1\n",
        // A blank line or a comment between them does not make them two nodes.
        "a:\n  !!str\n\n  !!int\nb: 1\n",
        "a:\n  !!str\n  # c\n  !!int\nb: 1\n",
        "a:\n  !!str\r\n  !!int\r\nb: 1\r\n",
        // A trailing comment on the first line.
        "a:\n  !!str # c\n  !!int\nb: 1\n",
    ];
    for input in rows {
        let (out, err, code) = run_yq_stdin_with_stderr(".", input, &[])?;
        assert_ne!(code, 0, "{input:?}");
        assert_eq!(out, "", "{input:?}");
        assert!(err.contains("inconsistent indentation"), "{input:?}: {err}");
    }
    Ok(())
}

/// A comment after the properties is claimed by a block scalar header on the next line, not
/// lost. It prints on the header line, as yq does (`a: !!str | # d`, #5001).
#[test]
fn test_4851_e_a_commented_properties_line_before_a_block_scalar_header() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a:\n  !!str # d\n  |\n   text\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: !!str | # d\n  text\n");
    Ok(())
}

/// Comment lines between a bare `-` and properties that stand alone on the next line head
/// whatever opens after the empty node, as the dash line's own comment does; they used to be
/// placed by the column they were written at, which made them the sequence owner's foot and
/// printed a blank line after them that yq does not.
#[test]
fn test_4851_f_comment_lines_before_own_line_properties_head_the_next_node() -> Result<()> {
    let rows: &[(&str, &str)] = &[
        (
            "k:\n  - # c\n    # x\n    !!str\nz: 1\n",
            "k:\n  - !!str\n# c\n# x\nz: 1\n",
        ),
        (
            "k:\n  -\n    # x\n    !!str\nz: 1\n",
            "k:\n  - !!str\n# x\nz: 1\n",
        ),
        (
            "k:\n  - # c\n    # x\n    &y\nz: 1\n",
            "k:\n  - &y\n# c\n# x\nz: 1\n",
        ),
        (
            "k:\n  - # c\n    # x\n    # y\n    !!str\nz: 1\n",
            "k:\n  - !!str\n# c\n# x\n# y\nz: 1\n",
        ),
        (
            "k:\n  - # c\n    # x\n    !!str\nz: 1\ny: 2\n",
            "k:\n  - !!str\n# c\n# x\nz: 1\ny: 2\n",
        ),
        (
            "k:\n  -\n    # x\n    !!str\n  - 3\nz: 1\n",
            "k:\n  - !!str\n  # x\n  - 3\nz: 1\n",
        ),
        (
            "- k:\n    -\n      # x\n      !!str\n  z: 1\n- 2\n",
            "- k:\n    - !!str\n  # x\n  z: 1\n- 2\n",
        ),
        ("-\n  # x\n  !!str\n- 2\n", "- !!str\n# x\n- 2\n"),
        (
            "k:\n  -\n    # x\n\n    !!str\nz: 1\n",
            "k:\n  - !!str\n# x\n\nz: 1\n",
        ),
        (
            "k:\n  - 1\n  -\n    # x\n    !!str\nz: 1\n",
            "k:\n  - 1\n  - !!str\n# x\nz: 1\n",
        ),
    ];
    for (input, expected) in rows {
        let (out, code) = run_yq_stdin(".", input, &[])?;
        assert_eq!(code, 0, "{input:?}");
        assert_eq!(&out, expected, "{input:?}");
    }
    // The getters follow: the block heads `z`, and is not `k`'s foot.
    let getters: &[(&str, &str, &str)] = &[
        (
            "k:\n  - # c\n    # x\n    !!str\nz: 1\n",
            ".z | key | head_comment",
            "c\nx\n",
        ),
        (
            "k:\n  - # c\n    # x\n    !!str\nz: 1\n",
            ".k | key | foot_comment",
            "\n",
        ),
        (
            "k:\n  -\n    # x\n    !!str\nz: 1\n",
            ".z | key | head_comment",
            "x\n",
        ),
        (
            "k:\n  -\n    # x\n    !!str\nz: 1\n",
            ".k | key | foot_comment",
            "\n",
        ),
    ];
    for (input, filter, expected) in getters {
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(code, 0, "{input:?} {filter}");
        assert_eq!(&out, expected, "{input:?} {filter}");
    }
    Ok(())
}

/// At the end of the document yq drops the comments before an empty node's properties
/// (`k:` / `  - # c` / `    # x` / `    !!str` prints no `# c` / `# x`), and this keeps them as
/// the document's foot: matching would discard them (recorded in `limitations.md`).
#[test]
fn test_4851_g_comments_before_own_line_properties_at_the_end_of_the_document_are_kept(
) -> Result<()> {
    let (out, code) = run_yq_stdin(".", "k:\n  - # c\n    # x\n    !!str\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "k:\n  - !!str\n# c\n# x\n");
    Ok(())
}

/// Shapes next to the block that still differ from yq, pinned with this tool's answer so a change
/// is deliberate (all recorded in `limitations.md`): a `---` or `...` after the empty node (yq
/// drops the comments, this keeps them) and a comment after the properties line (yq orders it
/// before the block, with a blank line between them). A blank line between the properties and the next
/// key used to be a third (yq prints none after the comments); #5053 matched it.
#[test]
fn test_4851_h_residuals_next_to_the_block_are_pinned() -> Result<()> {
    let rows: &[(&str, &str)] = &[
        (
            "k:\n  -\n    # x\n    !!str\n---\nz: 1\n",
            "k:\n  - !!str\n# x\n---\nz: 1\n",
        ),
        ("k:\n  -\n    # x\n    !!str\n...\n", "k:\n  - !!str\n# x\n"),
        (
            "k:\n  - # c\n    # x\n    !!str\n\nz: 1\n",
            "k:\n  - !!str\n# c\n# x\nz: 1\n",
        ),
        (
            "k:\n  -\n    # x\n    !!str\n  # y\nz: 1\n",
            "k:\n  - !!str\n# x\n# y\nz: 1\n",
        ),
        (
            "k:\n  -\n    # x\n    !!str\n# y\nz: 1\n",
            "k:\n  - !!str\n# x\n\n# y\nz: 1\n",
        ),
    ];
    for (input, ours) in rows {
        let (out, code) = run_yq_stdin(".", input, &[])?;
        assert_eq!(code, 0, "{input:?}");
        assert_eq!(&out, ours, "{input:?}");
    }
    Ok(())
}
