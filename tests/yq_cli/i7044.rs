//! #7044: a comment block between a bare `-` and its value on a later line, followed by a blank
//! line, is in yq the *foot* of the value's first key or scalar, printed below it, once a node has
//! opened earlier in the stream and the dash line has no comment of its own; a block after the
//! blank line is that node's head. Every answer was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

/// Each node's and each mapping key's `[head_comment, foot_comment]`.
const COMMENTS: &str = "[.. | [head_comment, foot_comment]] + [.. | select(tag==\"!!map\") | keys[] | [head_comment, foot_comment]]";

/// (document, yq's `.`, yq's `-o=json -I0` of [`COMMENTS`]).
const FOOT_ROWS: &[(&str, &str, &str)] = &[
    // one key
    ("- 1\n-\n  # c\n\n  n1: 1\n", "- 1\n- n1: 1\n  # c\n", "[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"c\"]]\n"),
    // two keys
    ("- 1\n-\n  # c\n\n  n1: 1\n  n2: 2\n", "- 1\n- n1: 1\n  # c\n\n  n2: 2\n", "[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"c\"],[\"\",\"\"]]\n"),
    // third item
    ("- 1\n- 2\n-\n  # c\n\n  n1: 1\n", "- 1\n- 2\n- n1: 1\n  # c\n", "[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"c\"]]\n"),
    // two lines
    ("- 1\n-\n  # c\n  # d\n\n  n1: 1\n", "- 1\n- n1: 1\n  # c\n  # d\n", "[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"c\\nd\"]]\n"),
    // a second block heads the key
    ("- 1\n-\n  # c\n\n  # d\n  n1: 1\n", "- 1\n- # d\n  n1: 1\n  # c\n", "[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"d\",\"c\"]]\n"),
    // scalar
    ("- 1\n-\n  # c\n\n  5\n", "- 1\n- 5\n# c\n", "[[\"\",\"\"],[\"\",\"\"],[\"\",\"c\"],[]]\n"),
    // nested sequence
    ("- 1\n-\n  # c\n\n  - a: 1\n", "- 1\n- - a: 1\n    # c\n", "[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"c\"]]\n"),
    // first item after a key
    ("a: 1\nb:\n  -\n    # c\n\n    n1: 1\n", "a: 1\nb:\n  - n1: 1\n    # c\n", "[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"c\"]]\n"),
    // after an earlier document
    ("a: 1\n---\n-\n  # c\n\n  n1: 1\n", "a: 1\n---\n- n1: 1\n  # c\n", "[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"]]\n[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"c\"]]\n"),
    // CRLF
    ("- 1\r\n-\r\n  # c\r\n\r\n  n1: 1\r\n", "- 1\n- n1: 1\n  # c\n", "[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"c\"]]\n"),
    // a block above the dash that a blank line settles, then two blocks below it
    ("- 1\n# e9\n\n-\n  # c\n\n  # d\n  k0: 1\n", "- 1\n# e9\n\n- # d\n  k0: 1\n  # c\n", "[[\"\",\"\"],[\"\",\"e9\"],[\"\",\"\"],[\"\",\"\"],[\"d\",\"c\"]]\n"),
    // after a null item
    ("-\n-\n  # c\n\n  n1: 1\n", "-\n- n1: 1\n  # c\n", "[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"c\"]]\n"),
];

/// Shapes where the block stays the head, in yq and here.
const HEAD_ROWS: &[(&str, &str, &str)] = &[
    // first node of the document
    (
        "-\n  # c\n\n  n1: 1\n",
        "- # c\n  n1: 1\n",
        "[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"c\\n\",\"\"]]\n",
    ),
    // no blank line
    (
        "- 1\n-\n  # c\n  n1: 1\n",
        "- 1\n- # c\n  n1: 1\n",
        "[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"c\",\"\"]]\n",
    ),
    // a comment on the dash line
    (
        "- 1\n- # d\n  # c\n\n  n1: 1\n",
        "- 1\n- # d\n  # c\n  n1: 1\n",
        "[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"d\\nc\\n\",\"\"]]\n",
    ),
    // a blank line above the block
    (
        "- 1\n-\n\n  # c\n\n  n1: 1\n",
        "- 1\n- # c\n  n1: 1\n",
        "[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"c\\n\",\"\"]]\n",
    ),
    // a block above the dash, then one adjacent to the value
    (
        "- 1\n# e9\n\n-\n  # d\n  k0: 1\n",
        "- 1\n# e9\n\n- # d\n  k0: 1\n",
        "[[\"\",\"\"],[\"\",\"e9\"],[\"\",\"\"],[\"\",\"\"],[\"d\",\"\"]]\n",
    ),
];

#[test]
fn a_block_and_a_blank_line_below_a_bare_dash_is_the_values_foot_7044() -> Result<()> {
    for (document, yaml, comments) in FOOT_ROWS.iter().chain(HEAD_ROWS) {
        let (stdout, status) = run_yq_stdin(".", document, &[])?;
        assert_eq!((stdout.as_str(), status), (*yaml, 0), "{document:?}");
        let (stdout, status) = run_yq_stdin(COMMENTS, document, &["-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), status), (*comments, 0), "{document:?}");
    }
    Ok(())
}

/// Before a flow collection yq drops the block, which is not reproduced: the block stays the first
/// node's head, as before #7044 (`docs/compliance/yq/limitations.md`). (document, succinctly's
/// `-o=json -I0` of [`COMMENTS`]; yq's answer is in each comment.)
const FLOW_RESIDUAL_ROWS: &[(&str, &str)] = &[
    // a flow sequence: yq drops the block ("[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[]]")
    (
        "- 1\n-\n  # c\n\n  [1]\n",
        "[[\"\",\"\"],[\"\",\"\"],[\"c\\n\",\"\"],[\"\",\"\"],[]]\n",
    ),
    // a flow mapping: yq drops the block ("[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"]]")
    (
        "- 1\n-\n  # c\n\n  {a: 1}\n",
        "[[\"\",\"\"],[\"\",\"\"],[\"c\\n\",\"\"],[\"\",\"\"],[\"\",\"\"]]\n",
    ),
    // a sequence whose first item is one: yq drops the block ("[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[]]")
    (
        "- 1\n-\n  # c\n\n  - [1]\n",
        "[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"c\\n\",\"\"],[\"\",\"\"],[]]\n",
    ),
    // an anchored one: yq drops the block ("[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[]]")
    (
        "- 1\n-\n  # c\n\n  &x [1]\n",
        "[[\"\",\"\"],[\"\",\"\"],[\"c\",\"\"],[\"\",\"\"],[]]\n",
    ),
    // a tagged one: yq drops the block ("[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"]]")
    (
        "- 1\n-\n  # c\n\n  !!map {a: 1}\n",
        "[[\"\",\"\"],[\"\",\"\"],[\"c\",\"\"],[\"\",\"\"],[\"\",\"\"]]\n",
    ),
    // a key written as one: yq drops the block ("[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"]]")
    (
        "- 1\n-\n  # c\n\n  [1]: 2\n",
        "[[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"\",\"\"],[\"c\\n\",\"\"]]\n",
    ),
];

#[test]
fn a_block_before_a_flow_collection_stays_its_head_7044() -> Result<()> {
    for (document, comments) in FLOW_RESIDUAL_ROWS {
        let (stdout, status) = run_yq_stdin(COMMENTS, document, &["-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), status), (*comments, 0), "{document:?}");
    }
    Ok(())
}
