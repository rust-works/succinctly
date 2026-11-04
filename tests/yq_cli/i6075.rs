//! #6075: a source single-quoted scalar with an empty line keeps its quotes across the break
//! (#6058), and yq indents the continuation by the scalar's depth: a flow collection inside another
//! one more step in for each level (`a: [['x<LF><LF>      y']]`), and a mapping key that holds a break
//! is written as `? 'k<LF><LF>  z'` with the `:` on a line of its own. `succinctly yq` spelled the
//! nested scalar with no indent, which a YAML reader folds back to a space, and the key as a
//! one-line scalar with one break. Every row is a live yq v4.53.3 capture.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, extra arguments, expected output).
const ROWS: &[(&str, &str, &[&str], &str)] = &[
    ("a: [['x\n\n  y']]\n", ".", &[], "a: [['x\n\n      y']]\n"),
    (
        "a: [[['x\n\n  y']]]\n",
        ".",
        &[],
        "a: [[['x\n\n        y']]]\n",
    ),
    (
        "a: {b: ['x\n\n  y']}\n",
        ".",
        &[],
        "a: {b: ['x\n\n      y']}\n",
    ),
    (
        "a: [{b: 'x\n\n  y'}]\n",
        ".",
        &[],
        "a: [{b: 'x\n\n      y'}]\n",
    ),
    (
        "a: [['x\n\n\n  y']]\n",
        ".",
        &[],
        "a: [['x\n\n\n      y']]\n",
    ),
    ("[[['x\n\n  y']]]\n", ".", &[], "[[['x\n\n        y']]]\n"),
    (
        "a: [{k: 'x\n\n  y', j: 1}]\n",
        ".",
        &[],
        "a: [{k: 'x\n\n      y', j: 1}]\n",
    ),
    (
        "a: [['x\n\n  y']]\n",
        ".",
        &["-I4"],
        "a: [['x\n\n            y']]\n",
    ),
    (
        "a: [['x\n\n  y']]\n",
        ".zz = 1",
        &[],
        "a: [['x\n\n      y']]\nzz: 1\n",
    ),
    (
        "a: [[['x\n\n  y'], 1], 2]\nb: {c: [{d: 'p\n\n q'}]}\n",
        ".",
        &[],
        "a: [[['x\n\n        y'], 1], 2]\nb: {c: [{d: 'p\n\n        q'}]}\n",
    ),
    (
        "k:\n  a: [['x\n\n  y']]\n",
        ".",
        &[],
        "k:\n  a: [['x\n\n        y']]\n",
    ),
    (
        "- a: [['x\n\n  y']]\n",
        ".",
        &[],
        "- a: [['x\n\n        y']]\n",
    ),
    (
        "a: [['x\n\n  y']] # c\n",
        ".",
        &[],
        "a: [['x\n\n      y']] # c\n",
    ),
    (
        "a: [['x\n\n  y']]\n",
        ".a |= .",
        &[],
        "a: [['x\n\n      y']]\n",
    ),
    ("? 'k\n\nz'\n: 1\n", ".", &[], "? 'k\n\n  z'\n: 1\n"),
    ("? 'k\n\n\nz'\n: 1\n", ".", &[], "? 'k\n\n\n  z'\n: 1\n"),
    (
        "? 'k\n\nz'\n: {b: 'q\n\n r'}\n",
        ".",
        &[],
        "? 'k\n\n  z'\n: {b: 'q\n\n    r'}\n",
    ),
    (
        "a:\n  ? 'k\n\nz'\n  : 1\n",
        ".",
        &[],
        "a:\n  ? 'k\n\n    z'\n  : 1\n",
    ),
    (
        "? 'k\n\nz'\n: 1\n",
        ".zz = 1",
        &[],
        "? 'k\n\n  z'\n: 1\nzz: 1\n",
    ),
    (
        "? 'k\n\nz'\n: 1\n",
        ".zz = 1",
        &["-I4"],
        "? 'k\n\n    z'\n: 1\nzz: 1\n",
    ),
    (
        "a:\n  ? 'k\n\nz'\n  : 1\n",
        ".zz = 1",
        &[],
        "a:\n  ? 'k\n\n    z'\n  : 1\nzz: 1\n",
    ),
    ("? 'k\n\nz'\n: 1\n", ".[\"k\\nz\"]", &[], "1\n"),
    (
        "? 'k\n\nz'\n: 1\nb: 2\n",
        ".",
        &[],
        "? 'k\n\n  z'\n: 1\nb: 2\n",
    ),
    (
        "? 'k\n\nz'\n: 1\n? 'p\n\nq'\n: 2\n",
        ".",
        &[],
        "? 'k\n\n  z'\n: 1\n? 'p\n\n  q'\n: 2\n",
    ),
];

#[test]
fn a_nested_flow_item_and_a_key_with_an_empty_line_are_written_as_yq_writes_them() -> Result<()> {
    for (doc, filter, args, want) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, doc, args)?;
        assert_eq!(code, 0, "{doc:?} {filter}");
        assert_eq!(&stdout, want, "{doc:?} {filter} {args:?}");
    }
    Ok(())
}

/// The output reads back as the same value: an unindented continuation folded to a space.
#[test]
fn the_output_of_such_a_scalar_reads_back_as_the_same_value_6075() -> Result<()> {
    for doc in [
        "a: [['x\n\n  y']]\n",
        "a: [[['x\n\n  y']]]\n",
        "a: [{b: 'x\n\n  y'}]\n",
        "? 'k\n\nz'\n: 1\n",
        "a:\n  ? 'k\n\nz'\n  : 1\n",
    ] {
        let (written, code) = run_yq_stdin(".", doc, &[])?;
        assert_eq!(code, 0, "{doc:?}");
        let (before, _) = run_yq_stdin(".", doc, &["-o=json", "-I0"])?;
        let (after, _) = run_yq_stdin(".", &written, &["-o=json", "-I0"])?;
        assert_eq!(before, after, "{doc:?} wrote {written:?}");
    }
    Ok(())
}
