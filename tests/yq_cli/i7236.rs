//! #7236: an update that writes a closed literal through `.[]` (`.[] |= [1]`) replaces the nodes
//! it reaches, and each keeps nothing of the node it replaced, as in yq. The reconcile took the
//! iterated path for a path it could not name and paired the old sequence's elements with the new
//! one's by position, so a comment of the old element at that slot survived onto the literal's
//! (`x:` / `- 4 # c` written `.[] |= [1]` is `x:` / `- 1` in yq). A write through a named path
//! (`.x |= [1]`) was already right. Every row is yq v4.53.3's output, captured by script.

use anyhow::Result;

use crate::run_yq_stdin;

fn assert_rows(rows: &[(&str, &str, &str)]) -> Result<()> {
    for &(input, filter, expected) in rows {
        let (out, status) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(
            (out.as_str(), status),
            (expected, 0),
            "{filter} on {input:?}"
        );
    }
    Ok(())
}

/// A literal sequence or mapping written through `.[]` drops the comment of the node it replaces, at
/// any depth and over a mapping's values or a sequence's items.
#[test]
fn an_iterated_write_of_a_literal_keeps_no_old_comment_7236() -> Result<()> {
    assert_rows(&[
        (
            "x:\n  - 4 # c616\n  - 7 # c2\n",
            ".[] |= [1,2]",
            "x:\n  - 1\n  - 2\n",
        ),
        (
            "x:\n  - 4 # c616\n  - 7 # c2\n",
            ".[] = [1,2]",
            "x:\n  - 1\n  - 2\n",
        ),
        (
            "x:\n  - 4 # c616\n  - 7 # c2\n",
            ".[] |= ([1] | .)",
            "x:\n  - 1\n",
        ),
        (
            "x:\n  - 4 # c616\n  - [4, 9, 8]\n",
            ".[] |= [1]",
            "x:\n  - 1\n",
        ),
        ("z:\n  - [] # c707\n", ".[] |= [1]", "z:\n  - 1\n"),
        (
            "a:\n  b:\n    - 1 # p\n    - 2 # q\n",
            ".a[] |= [3]",
            "a:\n  b:\n    - 3\n",
        ),
        (
            "a:\n  b:\n    - 1 # p\n    - 2 # q\n",
            ".a.[] |= [3, 4]",
            "a:\n  b:\n    - 3\n    - 4\n",
        ),
        (
            "- - 4 # a\n  - 5 # b\n- - 6 # c\n",
            ".[] |= [1, 2]",
            "- - 1\n  - 2\n- - 1\n  - 2\n",
        ),
        (
            "- - 4 # a\n  - 5 # b\n- - 6 # c\n",
            ".[][] |= [9]",
            "- - - 9\n  - # a\n    - 9\n- # b\n  - - 9\n",
        ),
        (
            "x:\n  - 4 # a\ny:\n  - 5 # b\n",
            ".[] |= {\"k\": 1}",
            "x:\n  k: 1\ny:\n  k: 1\n",
        ),
    ])
}

/// The same writes through a named path, which the iterated one now agrees with.
#[test]
fn a_named_write_of_a_literal_agrees_with_the_iterated_one_7236() -> Result<()> {
    assert_rows(&[
        (
            "x:\n  - 4 # c616\n  - 7 # c2\n",
            ".x |= [1,2]",
            "x:\n  - 1\n  - 2\n",
        ),
        (
            "x:\n  - 4 # c616\n  - 7 # c2\ny: 1 # k\n",
            ".x |= ([1] | .)",
            "x:\n  - 1\ny: 1 # k\n",
        ),
    ])
}

/// What does not change: a scalar written through `.[]` keeps its own comment, and a body that
/// reads the node it updates keeps each element's.
#[test]
fn a_scalar_or_a_body_that_reads_the_node_keeps_its_comments_7236() -> Result<()> {
    assert_rows(&[
        ("- 1 # p\n- 2 # q\n", ".[] |= 3", "- 3 # p\n- 3 # q\n"),
        (
            "a: 1 # p\nb: 2 # q\n",
            ".[] |= \"s\"",
            "a: s # p\nb: s # q\n",
        ),
        (
            "x:\n  - 4 # c616\n  - 7 # c2\n",
            ".[] |= . + [1]",
            "x:\n  - 4 # c616\n  - 7 # c2\n  - 1\n",
        ),
        (
            "x:\n  - 4 # c616\n  - 7 # c2\n",
            ".[] |= map(.)",
            "x:\n  - 4 # c616\n  - 7 # c2\n",
        ),
        (
            "x:\n  - 4 # c616\n  - 7 # c2\n",
            ".[] |= [.[0]]",
            "x:\n  - 4 # c616\n",
        ),
        (
            "x:\n  - 4 # c616\n  - 7 # c2\n",
            ".[] |= (.[0] = 9)",
            "x:\n  - 9 # c616\n  - 7 # c2\n",
        ),
    ])
}

/// Another write in the pipe does not turn the iterated one back to the positional pairing.
#[test]
fn an_iterated_write_after_another_write_drops_the_comments_too_7236() -> Result<()> {
    assert_rows(&[(
        "x:\n  - 4 # c616\n  - 7 # c2\ny: 1\n",
        ".y = 2 | .[] |= [1]",
        "x:\n  - 1\ny:\n  - 1\n",
    )])
}
