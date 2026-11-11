//! #6986: yq's `map(f)` builds a sequence that takes the style of the node it maps, and nothing
//! else of it: over a flow-style sequence or mapping the result prints in flow, whatever `f` is,
//! without the node's comment or anchor. The streamers write it in flow; a node with a comment inside
//! it takes the DOM route, which lays that comment out and gives the result the style there. A later
//! block document is not a flow element of the document stream when the first document opens with
//! `[`.
//!
//! Every expected output is captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// `(filter, document, yq's output)` over a document with no comment: the streamers.
const STREAMED: &[(&str, &str, &str)] = &[
    ("map(.)", "[1, 2]\n", "[1, 2]\n"),
    ("map(. == 1)", "[1, 2]\n", "[true, false]\n"),
    ("map(tag)", "[1, 2]\n", "['!!int', '!!int']\n"),
    ("map(select(. > 1))", "[1, 2]\n", "[2]\n"),
    ("map([.])", "[1, 2]\n", "[[1], [2]]\n"),
    ("map(tostring)", "[1, 2]\n", "[\"1\", \"2\"]\n"),
    ("map({\"a\": .})", "[1, 2]\n", "[{a: 1}, {a: 2}]\n"),
    ("map(.a)", "[{a: 1}, {b: 2}]\n", "[1, null]\n"),
    (".m | map(.)", "m: {x: 1, y: [2]}\n", "[1, [2]]\n"),
    (".a | map(. == 1)", "a: {x: 1, y: 2}\n", "[true, false]\n"),
    (".a | map(.)", "a: [\"x, y\", \"a: b\", '#q']\n", "[\"x, y\", \"a: b\", '#q']\n"),
    (".a | map(.)", "a: [[1, 2]]\n", "[[1, 2]]\n"),
    (".a | map(select(. > 5))", "a: [1, 2]\n", "[]\n"),
    (".a | map(map(. == 1))", "a: [[1, 2], [3]]\n", "[[true, false], [false]]\n"),
    (".[\"a\"] | map(.)", "a: [1, 2]\n", "[1, 2]\n"),
    (".a[0].b | map(. == 1)", "a: [{b: [1]}]\n", "[true]\n"),
    (".a | map(.)", "a: &x [1, 2]\nb: *x\n", "[1, 2]\n"),
    (".b | map(. == 1)", "a: &x [1, 2]\nb: *x\n", "- true\n- false\n"),
];

/// `(filter, document, yq's output)` with a comment on the node or elsewhere in the document, which
/// keeps the streamers.
const COMMENT_OUTSIDE: &[(&str, &str, &str)] = &[
    (".a | map(.)", "# head\na: [1, 2]\n", "[1, 2]\n"),
    (".a | map(.)", "a: [1, \"b\", c] # lc\n", "[1, \"b\", c]\n"),
    (".a | map(.)", "# c\na: [1, 2] # lc\n", "[1, 2]\n"),
    (".a | map(. + 1)", "a: [1, 2] # lc\n", "[2, 3]\n"),
    ("map(. == 1)", "# head\n[1, 2] # lc\n", "[true, false]\n"),
];

/// `(filter, document, yq's output)` with a comment inside the node: the DOM route.
const COMMENT_INSIDE: &[(&str, &str, &str)] = &[
    ("map(.)", "[1, # one\n 2]\n", "[1, # one\n  2]\n"),
    (".a | map(. == 1)", "a: [1, # one\n  2]\n", "[true, false]\n"),
    (".a | map(.)", "a: [[1, # c\n  2], 3]\n", "[[1, # c\n    2], 3]\n"),
    (".a | map(.)", "a: {x: 1, # c\n  y: 2}\n", "[1, # c\n  2]\n"),
    (".b | map(.)", "# c\na: [1, 2]\nb: [3, # d\n  4]\n", "[3, # d\n  4]\n"),
    (".a | map(.)", "a: [\n  # head of 1\n  1,\n  2\n]\n", "[\n  # head of 1\n  1, 2]\n"),
];

/// `(filter, document, yq's output)`: a block-style node keeps a block result.
const BLOCK: &[(&str, &str, &str)] = &[
    ("map(.)", "- 1\n- 2\n", "- 1\n- 2\n"),
    ("map(. == 1)", "- 1\n- 2\n", "- true\n- false\n"),
    ("map(.a)", "- {a: 1}\n- {a: 2}\n", "- 1\n- 2\n"),
];

fn assert_rows(rows: &[(&str, &str, &str)], args: &[&str]) -> Result<()> {
    for &(filter, doc, expected) in rows {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, doc, args)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (expected, "", 0),
            "{filter} on {doc:?} with {args:?}"
        );
    }
    Ok(())
}

#[test]
fn map_over_a_flow_node_prints_in_flow_6986() -> Result<()> {
    assert_rows(STREAMED, &[])?;
    assert_rows(COMMENT_OUTSIDE, &[])?;
    assert_rows(COMMENT_INSIDE, &[])?;
    assert_rows(BLOCK, &[])
}

/// `--arg` sends every document to the DOM route, where the result is the same.
#[test]
fn map_over_a_flow_node_prints_in_flow_on_the_dom_route_6986() -> Result<()> {
    assert_rows(STREAMED, &["--arg", "z", "1"])?;
    assert_rows(COMMENT_OUTSIDE, &["--arg", "z", "1"])?;
    assert_rows(COMMENT_INSIDE, &["--arg", "z", "1"])?;
    assert_rows(BLOCK, &["--arg", "z", "1"])
}

/// Each document's own style decides (`-N`: yq writes no `---` between `map` results, a separate
/// rule this file does not pin).
#[test]
fn each_document_decides_its_own_style_6986() -> Result<()> {
    let rows: &[(&str, &str, &str)] = &[
        (".a | map(.)", "a: [1, 2]\n---\na:\n  - 3\n", "[1, 2]\n- 3\n"),
        ("map(.)", "[1, 2]\n---\n- 3\n", "[1, 2]\n- 3\n"),
        ("map(.)", "- 3\n---\n[1, 2]\n", "- 3\n[1, 2]\n"),
        ("style", "[1, 2]\n---\n- 3\n---\na: 1\n", "flow\n\n\n"),
    ];
    assert_rows(rows, &["-N"])?;
    assert_rows(rows, &["-N", "--arg", "z", "1"])
}
