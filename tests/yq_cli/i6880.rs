//! #6880: a write holds no presentation tree for the parts of a document that say nothing, and
//! gives the same output without it. Every row was captured from yq v4.53.3 (`yq`, YAML output).
//!
//! The rows cover what the saving must not change: `-P` placing a flow collection's trailing
//! comment on the next item (the held comment goes to that item's first member, which a
//! collapsed subtree has to grow back), a kind-changed item taking it too, writes over documents
//! with and without a comment, quoting or `- # note` beside the silent parts, and the writes
//! that need the tree by position (a plain `=` from a read, an alias, a metadata write, a tag
//! write), which keep the full tree.

use anyhow::Result;

use crate::run_yq_stdin;

fn assert_yq_rows(rows: &[(&str, &[&str], &str, &str)]) -> Result<()> {
    for (doc, flags, filter, expected) in rows {
        let (out, code) = run_yq_stdin(filter, doc, flags)?;
        assert_eq!(code, 0, "{flags:?} {filter} over {doc:?}");
        assert_eq!(&out, expected, "{flags:?} {filter} over {doc:?}");
    }
    Ok(())
}

#[test]
fn test_a_flow_comment_held_for_a_silent_block_item_is_kept_under_p_6880() -> Result<()> {
    assert_yq_rows(&[
        (
            "- [1, 2] # ec\n- a: 1\n  b: 2\n",
            &["-P"],
            ".[2] = 3",
            "- - 1\n  - 2\n- # ec\n  a: 1\n  b: 2\n- 3\n",
        ),
        (
            "- [1, 2] # ec\n- a: 1\n  b: 2\n",
            &["-P"],
            ".[1].zzz = 1",
            "- - 1\n  - 2\n- # ec\n  a: 1\n  b: 2\n  zzz: 1\n",
        ),
        (
            "- [1, 2] # ec\n- a: 1\n  b: 2\n",
            &["-P"],
            "del(.[2])",
            "- - 1\n  - 2\n- # ec\n  a: 1\n  b: 2\n",
        ),
        (
            "- [1, 2] # ec\n- a: 1\n  b: 2\n",
            &["-P"],
            ".[0] |= . + [3]",
            "- - 1\n  - 2\n  - 3\n- # ec\n  a: 1\n  b: 2\n",
        ),
        (
            "- [1, 2] # ec\n- a: 1\n  b: 2\n",
            &[],
            ".[2] = 3",
            "- [1, 2] # ec\n- a: 1\n  b: 2\n- 3\n",
        ),
        (
            "- {a: 1} # ec\n- b: 1\n  c: 2\n",
            &["-P"],
            ". += [1]",
            "- a: 1\n- # ec\n  b: 1\n  c: 2\n- 1\n",
        ),
        (
            "- [1] # ec\n- - 1\n  - 2\n- x\n",
            &["-P"],
            ".[3] = 4",
            "- - 1\n- # ec\n  - 1\n  - 2\n- x\n- 4\n",
        ),
        (
            "x:\n  - [1, 2] # ec\n  - a: 1\n    b: 2\nw: 1\n",
            &["-P"],
            ".zzz = 1",
            "x:\n  - - 1\n    - 2\n  - # ec\n    a: 1\n    b: 2\nw: 1\nzzz: 1\n",
        ),
        (
            "- [1, 2] # ec\n- a:\n    - 1\n    - 2\n  b: 2\n",
            &["-P"],
            ".[2] = 3",
            "- - 1\n  - 2\n- # ec\n  a:\n    - 1\n    - 2\n  b: 2\n- 3\n",
        ),
    ])
}

#[test]
fn test_a_kind_changed_item_takes_the_held_flow_comment_6880() -> Result<()> {
    assert_yq_rows(&[
        (
            "- [1, 2] # ec\n- 5\n",
            &["-P"],
            ".[1] = {\"a\": 1}",
            "- - 1\n  - 2\n- # ec\n  a: 1\n",
        ),
        (
            "- [1, 2] # ec\n- 5\n",
            &["-P"],
            ".[1] = [7]",
            "- - 1\n  - 2\n- # ec\n  - 7\n",
        ),
        (
            "- [1, 2] # ec\n- 5\n",
            &["-P"],
            ".[1] = {\"a\": {\"b\": 1}}",
            "- - 1\n  - 2\n- # ec\n  a:\n    b: 1\n",
        ),
    ])
}

#[test]
fn test_writes_over_documents_with_silent_parts_print_what_yq_prints_6880() -> Result<()> {
    assert_yq_rows(&[
        (
            "a:\n  b:\n    - 1\n    - 2\n  c: x\nd:\n  - e: 1\n  - e: 2\nf: 3\n",
            &[],
            ".zzz = 1",
            "a:\n  b:\n    - 1\n    - 2\n  c: x\nd:\n  - e: 1\n  - e: 2\nf: 3\nzzz: 1\n",
        ),
        (
            "a:\n  b:\n    - 1\n    - 2\n  c: x\nd:\n  - e: 1\n  - e: 2\nf: 3\n",
            &["-P"],
            ".a.c |= \"y\"",
            "a:\n  b:\n    - 1\n    - 2\n  c: y\nd:\n  - e: 1\n  - e: 2\nf: 3\n",
        ),
        (
            "a:\n  b:\n    - 1\n    - 2\n  c: x\nd:\n  - e: 1\n  - e: 2\nf: 3\n",
            &[],
            "del(.d[0])",
            "a:\n  b:\n    - 1\n    - 2\n  c: x\nd:\n  - e: 2\nf: 3\n",
        ),
        (
            "a:\n  b:\n    - 1\n    - 2\n  c: x\nd:\n  - e: 1\n  - e: 2\nf: 3\n",
            &[],
            ".a = {\"b\": 2}",
            "a:\n  b: 2\nd:\n  - e: 1\n  - e: 2\nf: 3\n",
        ),
        (
            "# head\na:\n  b:\n    - 1\n  c: x # kept\nd:\n  - e: 1\nf: 3\n",
            &[],
            ".zzz = 1",
            "# head\na:\n  b:\n    - 1\n  c: x # kept\nd:\n  - e: 1\nf: 3\nzzz: 1\n",
        ),
        (
            "# head\na:\n  b:\n    - 1\n  c: x # kept\nd:\n  - e: 1\nf: 3\n",
            &[],
            ".d += [{\"e\": 2}]",
            "# head\na:\n  b:\n    - 1\n  c: x # kept\nd:\n  - e: 1\n  - e: 2\nf: 3\n",
        ),
        (
            "a:\n  b: 'q'\n  c:\n    - 1\nd:\n  - e: 1\n",
            &[],
            ".zzz = 1",
            "a:\n  b: 'q'\n  c:\n    - 1\nd:\n  - e: 1\nzzz: 1\n",
        ),
    ])
}

#[test]
fn test_a_plain_assignment_from_a_read_copies_the_source_presentation_6880() -> Result<()> {
    assert_yq_rows(&[
        (
            "a:\n  x: 1 # one\n  y:\n    - 1\nc: 2\n",
            &[],
            ".c = .a",
            "a:\n  x: 1 # one\n  y:\n    - 1\nc:\n  x: 1 # one\n  y:\n    - 1\n",
        ),
        (
            "a:\n  x: 1 # one\n  y:\n    - 1\nc: 2\n",
            &[],
            ".c = .a.x",
            "a:\n  x: 1 # one\n  y:\n    - 1\nc: 1 # one\n",
        ),
        (
            "a:\n  x: 1 # one\n  y:\n    - 1\nc:\n  k: 2 # old\n",
            &[],
            ".c = .a",
            "a:\n  x: 1 # one\n  y:\n    - 1\nc:\n  x: 1 # one\n  y:\n    - 1\n",
        ),
        (
            "a:\n  x: 1\n  y:\n    - 1\nb:\n  - 7\nc: 2\n",
            &[],
            ".c = (.b // 0)",
            "a:\n  x: 1\n  y:\n    - 1\nb:\n  - 7\nc:\n  - 7\n",
        ),
    ])
}

#[test]
fn test_an_alias_a_metadata_write_and_a_tag_write_keep_the_tree_6880() -> Result<()> {
    assert_yq_rows(&[
        (
            "a: &x\n  b: 1\nd: *x\nf:\n  - g: 1\n",
            &[],
            ".zzz = 1",
            "a: &x\n  b: 1\nd: *x\nf:\n  - g: 1\nzzz: 1\n",
        ),
        (
            "a:\n  b: 1\nd:\n  - e: 1\n",
            &[],
            ".a style = \"flow\"",
            "a: {b: 1}\nd:\n  - e: 1\n",
        ),
        (
            "a:\n  b: 1\nd:\n  - e: 1\n",
            &[],
            ".d head_comment = \"hello\"",
            "a:\n  b: 1\nd:\n  # hello\n  - e: 1\n",
        ),
        (
            "a:\n  b: 1\nd:\n  - e: 1\n",
            &[],
            ".a tag = \"!foo\"",
            "a: !foo\n  b: 1\nd:\n  - e: 1\n",
        ),
    ])
}

#[test]
fn test_a_dash_comment_element_keeps_its_nodes_6880() -> Result<()> {
    assert_yq_rows(&[
        (
            "- # note\n  a: 1\n  b: 2\n- c: 3\n",
            &[],
            ".[1].zzz = 1",
            "- # note\n  a: 1\n  b: 2\n- c: 3\n  zzz: 1\n",
        ),
        (
            "- # note\n  a: 1\n  b: 2\n- c: 3\n",
            &["-P"],
            ".[2] = 3",
            "- # note\n  a: 1\n  b: 2\n- c: 3\n- 3\n",
        ),
    ])
}

/// A metadata write places its comment into the tree by position, so the document keeps the full
/// tree: with the silent parts collapsed it printed `a: 1` and lost the `# hello` it prints today
/// before the refusal (#2796: only the clearing form of `head_comment =` is supported).
#[test]
fn test_a_head_comment_write_over_a_stream_still_prints_what_it_printed_6880() -> Result<()> {
    let (out, code) = run_yq_stdin(
        ".a head_comment=\"hello\"",
        "a: 1\n---\nb:\n  c: 2\n---\n# c\nd: 3\n",
        &[],
    )?;
    assert_eq!(code, 1);
    assert_eq!(out, "a: 1\n# hello\n");
    Ok(())
}
