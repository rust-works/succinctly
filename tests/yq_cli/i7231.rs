//! #7231: a write chain that writes one node twice, the first write changing its kind or emptying it
//! (`.a = 5 | .a = [9]` over `a: [x, 1] # c1`), wrote the node in its original flow style with the
//! comment beside it. yq writes each stage onto the node the stage before left, and its
//! `CandidateNode.UpdateFrom` gives a node that is now a scalar, a collection of another kind or an
//! empty one the style of what the later write brings, so the node is block and its comment is held
//! for the next line (#7230). The chain is now reconciled a stage at a time. Every row is yq
//! v4.53.3's output, captured by script.

use anyhow::Result;

use crate::run_yq_stdin;

fn assert_rows(rows: &[(&str, &str, &[&str], &str)]) -> Result<()> {
    for &(input, filter, flags, expected) in rows {
        let (out, status) = run_yq_stdin(filter, input, flags)?;
        assert_eq!(
            (out.as_str(), status),
            (expected, 0),
            "{filter} {flags:?} on {input:?}"
        );
    }
    Ok(())
}

/// An earlier stage changed the node's kind or emptied it: the later write's block style wins.
#[test]
fn a_node_an_earlier_write_changed_takes_the_later_writes_style_7231() -> Result<()> {
    assert_rows(&[
        (
            "a: [x, 1] # c1\nz: 1\n",
            ".a = 5 | .a = [9]",
            &[],
            "a:\n  - 9\nz: 1 # c1\n",
        ),
        (
            "a: [x, 1] # c1\nz: 1\n",
            ".a = {\"z\": 1} | .a = [9]",
            &[],
            "a:\n  - 9\nz: 1 # c1\n",
        ),
        (
            "a: [x, 1] # c1\nz: 1\n",
            ".a = [] | .a = [9]",
            &[],
            "a:\n  - 9\nz: 1 # c1\n",
        ),
        (
            "a: [x, 1] # c1\nz: 1\n",
            ".a |= 5 | .a |= [9]",
            &[],
            "a:\n  - 9\nz: 1 # c1\n",
        ),
        (
            "a: [x, 1] # c1\nz: 1\n",
            ".a = 5 | .z = 2 | .a = [9]",
            &[],
            "a:\n  - 9\nz: 2 # c1\n",
        ),
        (
            "a: [x, 1] # c1\nz: 1\n",
            ".a = 5 | .a = [9]",
            &["-P"],
            "a:\n  - 9\nz: 1 # c1\n",
        ),
        (
            "a: {p: 1} # c1\nz: 1\n",
            ".a = [1] | .a = {\"q\": 2}",
            &[],
            "a:\n  q: 2\nz: 1 # c1\n",
        ),
        (
            "a: 5 # c1\nz: 1\n",
            ".a = [1] | .a = 7 | .a = [3]",
            &[],
            "a:\n  - 3\nz: 1 # c1\n",
        ),
        (
            "- [x, 1] # c1\n- 3\n",
            ".[0] = 5 | .[0] = [9]",
            &[],
            "- - 9\n- 3 # c1\n",
        ),
    ])
}

/// A chain whose earlier stage kept the node a non-empty collection of the same kind, or a scalar,
/// changes nothing: the node keeps its style and comment, as it did before.
#[test]
fn a_chain_that_keeps_the_nodes_kind_keeps_its_style_7231() -> Result<()> {
    assert_rows(&[
        (
            "a: [x, 1] # c1\nz: 1\n",
            ".a = [7] | .a = [9]",
            &[],
            "a: [9] # c1\nz: 1\n",
        ),
        (
            "a: [x, 1] # c1\nz: 1\n",
            ".a = 5 | .a = 6",
            &[],
            "a: 6 # c1\nz: 1\n",
        ),
        (
            "a: [x, 1] # c1\nz: 1\n",
            ".a = [9] | .a[0] = 3",
            &[],
            "a: [3] # c1\nz: 1\n",
        ),
        (
            "a: {p: 1} # c1\nz: 1\n",
            ".a.p = 5 | .a.p = [1]",
            &[],
            "a: {p: [1]} # c1\nz: 1\n",
        ),
    ])
}

/// A copy over a node an earlier stage turned into a scalar takes the copied node's style, as a copy
/// over a scalar does (the target is judged by what the stage before left, not by the document).
#[test]
fn a_copy_over_a_node_an_earlier_write_changed_takes_the_sources_style_7231() -> Result<()> {
    assert_rows(&[
        (
            "- {k0: [1]}\n- {k1: [2, x]}\n",
            ".[0] = \"s\" | .[0] = .[1]",
            &[],
            "- {k1: [2, x]}\n- {k1: [2, x]}\n",
        ),
        (
            "a: [1] # c1\nb: {k: 2}\nz: 1\n",
            ".a = 5 | .a = .b",
            &[],
            "a: {k: 2} # c1\nb: {k: 2}\nz: 1\n",
        ),
    ])
}

/// A `.` stage between the writes changes nothing: the stages either side are still replayed in turn.
#[test]
fn an_identity_stage_in_the_chain_is_passed_over_7231() -> Result<()> {
    assert_rows(&[
        (
            "a: [x, 1] # c1\nz: 1\n",
            ".a = 5 | . | .a = [9]",
            &[],
            "a:\n  - 9\nz: 1 # c1\n",
        ),
        (
            "- {k0: [1]}\n- {k1: [2, x]}\n",
            ".[0] = \"s\" | . | .[0] = .[1]",
            &[],
            "- {k1: [2, x]}\n- {k1: [2, x]}\n",
        ),
    ])
}

/// A copy keeps the presentation it brought when a later stage writes into the copy, replaces it or
/// rewrites its source (#5407's rows): the copy is placed at its own stage, and the later stage
/// writes onto it as yq does. Before, the copy was skipped, or a later write's kind change moved the
/// target's comment onto an unrelated key.
#[test]
fn a_copy_keeps_its_presentation_through_a_later_stage_7231() -> Result<()> {
    assert_rows(&[
        (
            "a: 5 # c1\nb: [1]\nz: 1\n",
            ".a = .b | .a = [9]",
            &[],
            "a: [9] # c1\nb: [1]\nz: 1\n",
        ),
        (
            "a: 5 # c1\nb: {k: 1}\nz: 1\n",
            ".a = .b | .a.k = 9",
            &[],
            "a: {k: 9} # c1\nb: {k: 1}\nz: 1\n",
        ),
        (
            "a: 5 # c1\nb: [1]\nz: 1\n",
            ".a = .b | .a += [2]",
            &[],
            "a: [1, 2] # c1\nb: [1]\nz: 1\n",
        ),
        (
            "b: [1] # t\nc: 3\nz: 1\n",
            ".c = .b | .b = 5",
            &[],
            "b: 5 # t\nc: [1] # t\nz: 1\n",
        ),
        (
            "b: [{a: 1}] # t\nc: 3\nz: 1\n",
            ".c = .b | .c[0].a = 2",
            &[],
            "b: [{a: 1}] # t\nc: [{a: 2}] # t\nz: 1\n",
        ),
        (
            "b: [1] # t\nc: 3\nz: 1\n",
            ".c = .b | .c = 5",
            &[],
            "b: [1] # t\nc: 5 # t\nz: 1\n",
        ),
    ])
}
