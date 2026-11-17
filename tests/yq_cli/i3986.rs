//! #3986 (the slice of the 2026-11-18 triage): over a multi-document stream yq prints `---` before a
//! result whose node comes from a later document, and not before one it computed. `head_comment`,
//! `line_comment`, `foot_comment` and `alias` are computed (a string made from the node's own
//! comment, or the alias's target name, belongs to no document), so over `a: 1` / `---` / `a: 2` they
//! print their strings with no separator between; `succinctly yq` printed one. The same getters of a
//! navigated node (`.a | head_comment`) keep the separator in both tools, and so do `tag`, `anchor`
//! and the rest of the list they sit beside. Every row is yq v4.53.3's output, captured by script.

use anyhow::Result;

use crate::run_yq_stdin;

fn assert_rows(rows: &[(&str, &[&str], &str, &str)]) -> Result<()> {
    for &(input, flags, filter, expected) in rows {
        let (out, status) = run_yq_stdin(filter, input, flags)?;
        assert_eq!(
            (out.as_str(), status),
            (expected, 0),
            "{filter} {flags:?} on {input:?}"
        );
    }
    Ok(())
}

/// A comment getter or `alias` of a document root prints no `---` between documents.
#[test]
fn a_comment_getter_or_alias_of_a_root_prints_no_separator_3986() -> Result<()> {
    assert_rows(&[
        (
            "a: 1 # one\nb: x\n---\na: 2\nb: y\n",
            &[],
            "head_comment",
            "\n\n",
        ),
        (
            "a: 1\n---\na: 2\n---\na: 3\n",
            &[],
            "head_comment",
            "\n\n\n",
        ),
        (
            "a: 1 # one\nb: x\n---\na: 2\nb: y\n",
            &[],
            "line_comment",
            "\n\n",
        ),
        (
            "a: 1\n---\na: 2\n---\na: 3\n",
            &[],
            "line_comment",
            "\n\n\n",
        ),
        (
            "a: 1 # one\nb: x\n---\na: 2\nb: y\n",
            &[],
            "foot_comment",
            "\n\n",
        ),
        (
            "a: 1\n---\na: 2\n---\na: 3\n",
            &[],
            "foot_comment",
            "\n\n\n",
        ),
        ("a: 1 # one\nb: x\n---\na: 2\nb: y\n", &[], "alias", "\n\n"),
        ("a: 1\n---\na: 2\n---\na: 3\n", &[], "alias", "\n\n\n"),
        (
            "# head\na: 1 # line\n# foot\n---\n# h2\na: 2 # l2\n",
            &[],
            "head_comment",
            "head\n\n",
        ),
        (
            "# head\na: 1 # line\n# foot\n---\n# h2\na: 2 # l2\n",
            &[],
            "line_comment",
            "\n\n",
        ),
        (
            "# head\na: 1 # line\n# foot\n---\n# h2\na: 2 # l2\n",
            &[],
            "foot_comment",
            "foot\n\n",
        ),
        (
            "a: &x 1\nb: *x\n---\na: &y 2\nb: *y\n",
            &[],
            "alias",
            "\n\n",
        ),
        (
            "a: 1 # one\nb: x\n---\na: 2\nb: y\n",
            &["-P"],
            "head_comment",
            "\n\n",
        ),
        (
            "a: 1\n---\na: 2\n---\na: 3\n",
            &["-o=json", "-I0"],
            "alias",
            "\"\"\n\"\"\n\"\"\n",
        ),
        ("1\n---\n2\n", &[], "line_comment", "\n\n"),
        ("- 1\n---\n- 2\n", &[], "foot_comment", "\n\n"),
    ])
}

/// The same getters of a navigated node, and the neighbours in the list, keep the separator.
#[test]
fn a_getter_of_a_navigated_node_keeps_the_separator_3986() -> Result<()> {
    assert_rows(&[
        (
            "a: 1 # one\nb: x\n---\na: 2\nb: y\n",
            &[],
            ".a | head_comment",
            "\n---\n\n",
        ),
        (
            "a: 1 # one\nb: x\n---\na: 2\nb: y\n",
            &[],
            ".a | line_comment",
            "one\n---\n\n",
        ),
        (
            "a: 1 # one\nb: x\n---\na: 2\nb: y\n",
            &[],
            ".a | foot_comment",
            "\n---\n\n",
        ),
        (
            "a: 1 # one\nb: x\n---\na: 2\nb: y\n",
            &[],
            ".a | alias",
            "1\n---\n2\n",
        ),
        (
            "a: &x 1\nb: *x\n---\na: &y 2\nb: *y\n",
            &[],
            ".b | alias",
            "x\n---\ny\n",
        ),
        (
            "a: 1\n---\na: 2\n---\na: 3\n",
            &[],
            ".a | tag",
            "!!int\n---\n!!int\n---\n!!int\n",
        ),
        (
            "a: 1\n---\na: 2\n---\na: 3\n",
            &[],
            ".a | anchor",
            "\n---\n\n---\n\n",
        ),
        (
            "a: 1\n---\na: 2\n---\na: 3\n",
            &[],
            "tag",
            "!!map\n!!map\n!!map\n",
        ),
        ("a: 1\n---\na: 2\n---\na: 3\n", &[], "length", "1\n1\n1\n"),
        (
            "# head\na: 1 # line\n# foot\n---\n# h2\na: 2 # l2\n",
            &[],
            ".a | head_comment",
            "\n---\n\n",
        ),
    ])
}

/// A root that carries a comment of its own, a getter after `.` or a root rebound to a variable:
/// the string is still computed, so no separator.
#[test]
fn a_getter_of_a_commented_or_rebound_root_prints_no_separator_3986() -> Result<()> {
    assert_rows(&[
        ("1 # c1\n---\n2 # c2\n", &[], "line_comment", "c1\nc2\n"),
        ("1 # c1\n---\n2 # c2\n", &[], ". | line_comment", "c1\nc2\n"),
        ("# h1\n1\n---\n# h2\n2\n", &[], "head_comment", "h1\nh2\n"),
        (
            "# h1\n1\n---\n# h2\n2\n",
            &[],
            ". | head_comment",
            "h1\nh2\n",
        ),
        ("1\n# f1\n---\n2\n# f2\n", &[], "foot_comment", "f1\nf2\n"),
        (
            "1\n# f1\n---\n2\n# f2\n",
            &[],
            ". | foot_comment",
            "f1\nf2\n",
        ),
        (
            "a: &x 1\nb: *x\n---\na: &y 2\nb: *y\n",
            &[],
            ".b | alias",
            "x\n---\ny\n",
        ),
        (
            "a: &x 1\nb: *x\n---\na: &y 2\nb: *y\n",
            &[],
            ". as $r | $r | head_comment",
            "\n\n",
        ),
        (
            "a: 1 # l\nb: 2\n---\na: 3\nb: 4\n",
            &[],
            ". as $r | $r | line_comment",
            "\n\n",
        ),
    ])
}
