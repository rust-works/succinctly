//! #7230: a scalar's own line comment moves to the next line that has none when a write replaces
//! the scalar with a non-empty block collection. yq's encoder holds it (the mechanism #4377
//! describes for `-P`): the next item's line for a scalar, its dash line (the head comment of its
//! first member) for a block collection; it is overwritten by a commented item of a sequence,
//! waits behind a commented entry or a flow collection of a mapping, and is lost when nothing
//! follows.
//!
//! Every output is captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, `yq` output).
const ROWS: &[(&str, &str, &str)] = &[
    // The next item is a scalar: it takes the comment on its line.
    (
        "- 5 # own\n- 6\n",
        ".[0] = {\"a\": 1}",
        "- a: 1\n- 6 # own\n",
    ),
    ("- 5 # own\n- 6\n", ".[0] = [7]", "- - 7\n- 6 # own\n"),
    (
        "- 5 # own\n- 6\n",
        ".[0] = {\"a\": 1, \"b\": 2}",
        "- a: 1\n  b: 2\n- 6 # own\n",
    ),
    (
        "- 4\n- 5 # own\n- 6\n",
        ".[1] = {\"a\": 1}",
        "- 4\n- a: 1\n- 6 # own\n",
    ),
    // The next item is a block collection: the comment heads its first member.
    (
        "- 5 # own\n- a: 1\n",
        ".[0] = {\"b\": 1}",
        "- b: 1\n- # own\n  a: 1\n",
    ),
    (
        "- 5 # own\n- - 1\n  - 2\n",
        ".[0] = {\"a\": 1}",
        "- a: 1\n- # own\n  - 1\n  - 2\n",
    ),
    (
        "- 5 # own\n- a: 1\n  b: 2\n",
        ".[0] = [7]",
        "- - 7\n- # own\n  a: 1\n  b: 2\n",
    ),
    // Nothing follows: the comment is lost.
    ("- 4\n- 5 # own\n", ".[1] = {\"a\": 1}", "- 4\n- a: 1\n"),
    (
        "x:\n  - 5 # own\ny: 1\n",
        ".x[0] = {\"a\": 1}",
        "x:\n  - a: 1\ny: 1 # own\n",
    ),
    // A commented item of a sequence overwrites it.
    (
        "- 5 # own\n- 6 # six\n- 7\n",
        ".[0] = {\"a\": 1}",
        "- a: 1\n- 6 # six\n- 7\n",
    ),
    // A mapping's entry: the comment waits behind a commented entry and a flow collection.
    (
        "a: 5 # own\nb: 6\n",
        ".a = {\"x\": 1}",
        "a:\n  x: 1\nb: 6 # own\n",
    ),
    (
        "a: 5 # own\nb: 6 # six\nc: 7\n",
        ".a = {\"x\": 1}",
        "a:\n  x: 1\nb: 6 # six\nc: 7 # own\n",
    ),
    (
        "a: 5 # own\nb: {c: 1}\nd: 7\n",
        ".a = {\"x\": 1}",
        "a:\n  x: 1\nb: {c: 1}\nd: 7 # own\n",
    ),
    (
        "a: 5 # own\nb: []\nc: 7\n",
        ".a = {\"x\": 1}",
        "a:\n  x: 1\nb: []\nc: 7 # own\n",
    ),
    (
        "a: 5 # own\nb:\n  c: 1\n",
        ".a = {\"x\": 1}",
        "a:\n  x: 1\nb: # own\n  c: 1\n",
    ),
    // A value that fits on the line keeps it (#5258), as does a scalar written over a scalar.
    ("- 5 # own\n- 6\n", ".[0] = {}", "- {} # own\n- 6\n"),
    ("- 5 # own\n- 6\n", ".[0] = 7", "- 7 # own\n- 6\n"),
];

#[test]
fn test_7230_a_the_replaced_scalars_line_comment_moves_to_the_next_line() -> Result<()> {
    for (input, filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(code, 0, "{filter} on {input:?}");
        assert_eq!(&out, want, "{filter} on {input:?}");
    }
    Ok(())
}

/// `-o=json` carries no comments.
#[test]
fn test_7230_b_json_output_is_unchanged() -> Result<()> {
    let (out, code) = run_yq_stdin(
        ".[0] = {\"a\": 1}",
        "- 5 # own\n- 6\n",
        &["-o=json", "-I=0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(out, "[{\"a\":1},6]\n");
    Ok(())
}
