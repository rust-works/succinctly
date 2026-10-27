//! #5258: a node's same-line comment stays beside its new value when a write changes the node's
//! kind and the value still fits on that line -- a scalar, or an empty `{}` / `[]`. A flow container
//! (`b: [1, 2] # t`) keeps `# t` when it is overwritten with a scalar, as a scalar always did.
//! A non-empty block container does not fit on the line: yq parks the comment on the next line
//! (or drops it past the last key), which this tool does not reproduce (`limitations.md`).
//!
//! Every output is captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, `yq` output).
const ROWS: &[(&str, &str, &str)] = &[
    ("b: [1, 2] # t\nc: 3\n", ".b = 5", "b: 5 # t\nc: 3\n"),
    ("b: [1, 2] # t\nc: 3\n", ".b = null", "b: null # t\nc: 3\n"),
    ("b: [1, 2] # t\nc: 3\n", ".b |= length", "b: 2 # t\nc: 3\n"),
    ("b: [1, 2] # t\nc: 3\n", ".b = \"s\"", "b: s # t\nc: 3\n"),
    ("b: [1, 2] # t\nc: 3\n", ".b = 1.5", "b: 1.5 # t\nc: 3\n"),
    ("b: [1, 2] # t\nc: 3\n", ".b = true", "b: true # t\nc: 3\n"),
    (
        "b: [1, 2] # t\nc: 3\n",
        ".b = \"a b\"",
        "b: a b # t\nc: 3\n",
    ),
    (
        "b: [1, 2] # t\nc: 3\n",
        ".b = \"x: y\"",
        "b: 'x: y' # t\nc: 3\n",
    ),
    (
        "b: [1, 2] # t\nc: 3\n",
        ".b = (.c|tostring)",
        "b: \"3\" # t\nc: 3\n",
    ),
    (
        "b: [1, 2] # t\nc: 3\n",
        ".b = \"multi\\nline\"",
        "b: |- # t\n  multi\n  line\nc: 3\n",
    ),
    ("b: {x: 1} # t\nc: 3\n", ".b = 5", "b: 5 # t\nc: 3\n"),
    ("b: {x: 1} # t\nc: 3\n", ".b = null", "b: null # t\nc: 3\n"),
    ("b: {x: 1} # t\nc: 3\n", ".b |= length", "b: 1 # t\nc: 3\n"),
    ("b: {x: 1} # t\nc: 3\n", ".b = \"s\"", "b: s # t\nc: 3\n"),
    ("b: {x: 1} # t\nc: 3\n", ".b = 1.5", "b: 1.5 # t\nc: 3\n"),
    ("b: {x: 1} # t\nc: 3\n", ".b = true", "b: true # t\nc: 3\n"),
    (
        "b: {x: 1} # t\nc: 3\n",
        ".b = \"a b\"",
        "b: a b # t\nc: 3\n",
    ),
    (
        "b: {x: 1} # t\nc: 3\n",
        ".b = \"x: y\"",
        "b: 'x: y' # t\nc: 3\n",
    ),
    (
        "b: {x: 1} # t\nc: 3\n",
        ".b = (.c|tostring)",
        "b: \"3\" # t\nc: 3\n",
    ),
    (
        "b: {x: 1} # t\nc: 3\n",
        ".b = \"multi\\nline\"",
        "b: |- # t\n  multi\n  line\nc: 3\n",
    ),
    ("b: [] # t\nc: 3\n", ".b = 5", "b: 5 # t\nc: 3\n"),
    ("b: [] # t\nc: 3\n", ".b = null", "b: null # t\nc: 3\n"),
    ("b: [] # t\nc: 3\n", ".b |= length", "b: 0 # t\nc: 3\n"),
    ("b: [] # t\nc: 3\n", ".b = \"s\"", "b: s # t\nc: 3\n"),
    ("b: [] # t\nc: 3\n", ".b = 1.5", "b: 1.5 # t\nc: 3\n"),
    ("b: [] # t\nc: 3\n", ".b = true", "b: true # t\nc: 3\n"),
    ("b: [] # t\nc: 3\n", ".b = \"a b\"", "b: a b # t\nc: 3\n"),
    (
        "b: [] # t\nc: 3\n",
        ".b = \"x: y\"",
        "b: 'x: y' # t\nc: 3\n",
    ),
    (
        "b: [] # t\nc: 3\n",
        ".b = (.c|tostring)",
        "b: \"3\" # t\nc: 3\n",
    ),
    (
        "b: [] # t\nc: 3\n",
        ".b = \"multi\\nline\"",
        "b: |- # t\n  multi\n  line\nc: 3\n",
    ),
    ("b: {} # t\nc: 3\n", ".b = 5", "b: 5 # t\nc: 3\n"),
    ("b: {} # t\nc: 3\n", ".b = null", "b: null # t\nc: 3\n"),
    ("b: {} # t\nc: 3\n", ".b |= length", "b: 0 # t\nc: 3\n"),
    ("b: {} # t\nc: 3\n", ".b = \"s\"", "b: s # t\nc: 3\n"),
    ("b: {} # t\nc: 3\n", ".b = 1.5", "b: 1.5 # t\nc: 3\n"),
    ("b: {} # t\nc: 3\n", ".b = true", "b: true # t\nc: 3\n"),
    ("b: {} # t\nc: 3\n", ".b = \"a b\"", "b: a b # t\nc: 3\n"),
    (
        "b: {} # t\nc: 3\n",
        ".b = \"x: y\"",
        "b: 'x: y' # t\nc: 3\n",
    ),
    (
        "b: {} # t\nc: 3\n",
        ".b = (.c|tostring)",
        "b: \"3\" # t\nc: 3\n",
    ),
    (
        "b: {} # t\nc: 3\n",
        ".b = \"multi\\nline\"",
        "b: |- # t\n  multi\n  line\nc: 3\n",
    ),
    ("b: 1 # t\nc: 3\n", ".b = {}", "b: {} # t\nc: 3\n"),
    ("b: 1 # t\nc: 3\n", ".b = []", "b: [] # t\nc: 3\n"),
    ("b: [1, 2] # t\nc: 3\n", ".b = {}", "b: {} # t\nc: 3\n"),
    ("b: [1, 2] # t\nc: 3\n", ".b = []", "b: [] # t\nc: 3\n"),
    ("b: {x: 1} # t\nc: 3\n", ".b = {}", "b: {} # t\nc: 3\n"),
    ("b: {x: 1} # t\nc: 3\n", ".b = []", "b: [] # t\nc: 3\n"),
    ("b: [] # t\nc: 3\n", ".b = {}", "b: {} # t\nc: 3\n"),
    ("b: [] # t\nc: 3\n", ".b = []", "b: [] # t\nc: 3\n"),
    ("b: {} # t\nc: 3\n", ".b = {}", "b: {} # t\nc: 3\n"),
    ("b: {} # t\nc: 3\n", ".b = []", "b: [] # t\nc: 3\n"),
    ("a: 1\nb: [1] # t\n", ".b = 5", "a: 1\nb: 5 # t\n"),
    ("- 1\n- [1] # t\n- 3\n", ".[1] = 5", "- 1\n- 5 # t\n- 3\n"),
    (
        "a:\n  b: {x: 1} # t\nc: 3\n",
        ".a.b = 5",
        "a:\n  b: 5 # t\nc: 3\n",
    ),
    (
        "b: [1, 2] # t\nc: 3 # u\n",
        ".b = .c",
        "b: 3 # u\nc: 3 # u\n",
    ),
    (
        "b: [1, 2] # t\nc: {} # u\n",
        ".b = .c",
        "b: {} # u\nc: {} # u\n",
    ),
];

#[test]
fn test_5258_a_a_line_comment_stays_beside_a_value_that_fits_on_the_line() -> Result<()> {
    for (input, filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(code, 0, "{filter} on {input:?}");
        assert_eq!(&out, want, "{filter} on {input:?}");
    }
    Ok(())
}

/// A non-empty container does not fit on the line. yq prints the comment after the next entry
/// (`c: 3 # t`) and drops it when the node is the last entry; this tool drops it either way rather
/// than attach it to a node it never described.
#[test]
fn test_5258_b_a_non_empty_container_result_drops_the_comment() -> Result<()> {
    for (input, filter, want) in [
        (
            "b: [1, 2] # t\nc: 3\n",
            ".b = {\"y\": 2}",
            "b:\n  y: 2\nc: 3\n",
        ),
        ("b: 1 # t\nc: 3\n", ".b = [9]", "b:\n  - 9\nc: 3\n"),
        ("b: {x: 1} # t\nc: 3\n", ".b = [9]", "b:\n  - 9\nc: 3\n"),
    ] {
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(code, 0, "{filter} on {input:?}");
        assert_eq!(out, want, "{filter} on {input:?}");
    }
    Ok(())
}

/// `-o=json` carries no comments, so the write is unchanged.
#[test]
fn test_5258_c_json_output_is_unchanged() -> Result<()> {
    let (out, code) = run_yq_stdin(".b = 5", "b: [1, 2] # t\n", &["-o=json", "-I=0"])?;
    assert_eq!((out.as_str(), code), ("{\"b\":5}\n", 0));
    Ok(())
}
