//! #5249: a plain `=` from a static read of a node copies that node's trailing comment and
//! its flow style along with the value, as go-yaml's `Set` does (the comment and flow-style rows
//! #2505 left plain, and the same copy for a source with no alias in it).
//! Every expected output is what yq v4.53.3 prints for the same input and filter.
//!
//! What stays different (a scalar's own quoting, a constructed right side, a
//! compound assignment, the alias group a copy joins) is recorded in
//! `docs/compliance/yq/limitations.md`.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, expected output).
const ROWS: &[(&str, &str, &str)] = &[
    (
        "a: &x 1\nb: *x # bc\n",
        ".c = .b",
        "a: &x 1\nb: *x # bc\nc: *x # bc\n",
    ),
    // The target's parent is a scalar in the document the write started from (a null `c`), so
    // there is no node there to read the target's own style from.
    (
        "b: [1, 2] # t\nc:\n",
        ".c.d = .b",
        "b: [1, 2] # t\nc:\n  d: [1, 2] # t\n",
    ),
    (
        "a: &x 1\nb: *x # bc\nc: 2 # keep\n",
        ".c = .b",
        "a: &x 1\nb: *x # bc\nc: *x # bc\n",
    ),
    (
        "a: &x 1\nb: [*x]\n",
        ".c = .b",
        "a: &x 1\nb: [*x]\nc: [*x]\n",
    ),
    (
        "a: &x 1\nb: [*x] # t\n",
        ".c = .b",
        "a: &x 1\nb: [*x] # t\nc: [*x] # t\n",
    ),
    (
        "a: &x 1\nb: {k: *x}\n",
        ".c = .b",
        "a: &x 1\nb: {k: *x}\nc: {k: *x}\n",
    ),
    (
        "a: &x 1\nb: [*x] # t\nc: [9] # cc\n",
        ".c = .b",
        "a: &x 1\nb: [*x] # t\nc: [*x] # t\n",
    ),
    (
        "b: 2 # bc\nc: 3 # cc\n",
        ".c = .b",
        "b: 2 # bc\nc: 2 # bc\n",
    ),
    ("b: 2 # bc\nc: 3\n", ".c = .b", "b: 2 # bc\nc: 2 # bc\n"),
    ("b: 2\nc: 3 # cc\n", ".c = .b", "b: 2\nc: 2 # cc\n"),
    (
        "# head\nb: 2 # bc\n# foot\nc: 3\n",
        ".c = .b",
        "# head\nb: 2 # bc\n# foot\nc: 2 # bc\n",
    ),
    (
        "b:\n  # hd\n  k: 1 # kc\nc: 3\n",
        ".c = .b",
        "b:\n  # hd\n  k: 1 # kc\nc:\n  # hd\n  k: 1 # kc\n",
    ),
    (
        "b: [1, 2] # t\nc: 3 # cc\n",
        ".c = .b",
        "b: [1, 2] # t\nc: [1, 2] # t\n",
    ),
    (
        "b:\n  - 1 # one\n  - 2\n",
        ".c = .b",
        "b:\n  - 1 # one\n  - 2\nc:\n  - 1 # one\n  - 2\n",
    ),
    (
        "b: {x: 1, y: 2}\nc:\n  - 1\n",
        ".c = .b",
        "b: {x: 1, y: 2}\nc: {x: 1, y: 2}\n",
    ),
    (
        "b: [1] # t\nc: [9] # cc\n",
        ".c = .b",
        "b: [1] # t\nc: [1] # t\n",
    ),
    ("b: [1]\nc: [9] # cc\n", ".c = .b", "b: [1]\nc: [1] # cc\n"),
    ("b: 1 # t\nc: [9] # cc\n", ".c = .b", "b: 1 # t\nc: 1 # t\n"),
    (
        "b: [1] # t\nc: 9 # cc\n",
        ".c = .b",
        "b: [1] # t\nc: [1] # t\n",
    ),
    (
        "b:\n  - 1\nc: [9] # cc\n",
        ".c = .b",
        "b:\n  - 1\nc: [1] # cc\n",
    ),
    ("b: [] # t\nc: [9]\n", ".c = .b", "b: [] # t\nc: [] # t\n"),
    ("b: {} # t\nc: 1\n", ".c = .b", "b: {} # t\nc: {} # t\n"),
    (
        "b: [1, 2] # t\n",
        ".c.d = .b",
        "b: [1, 2] # t\nc:\n  d: [1, 2] # t\n",
    ),
    (
        "b: [1, 2] # t\nd: 1 # dd\n",
        "(.c, .d) = .b",
        "b: [1, 2] # t\nd: [1, 2] # t\nc: [1, 2] # t\n",
    ),
    (
        "b:\n  - 1 # one\n  - 2\n",
        ".c = .b[0]",
        "b:\n  - 1 # one\n  - 2\nc: 1 # one\n",
    ),
    (
        "b: {k: [1,2] # kk\n}\n",
        ".c = .b.k",
        "b: {k: [1, 2] # kk\n}\nc: [1, 2] # kk\n",
    ),
    (
        "b:\n  k: [1,2]\n",
        ".c = .b",
        "b:\n  k: [1, 2]\nc:\n  k: [1, 2]\n",
    ),
    (
        "b: [1, 2] # t\n",
        ".c = .b | .d = .c",
        "b: [1, 2] # t\nc: [1, 2] # t\nd: [1, 2] # t\n",
    ),
    (
        "b: [1, 2] # t\n",
        "(.c, .b) = .b",
        "b: [1, 2] # t\nc: [1, 2] # t\n",
    ),
    // A flow node wins over a block one: yq prints a flow source over a block target as flow, and
    // keeps a non-empty flow target flow under a block source, while an empty one takes the block.
    ("b: [1]\nc:\n  - 2\n", ".c = .b", "b: [1]\nc: [1]\n"),
    (
        "b: {x: 1}\nc:\n  y: 2\n",
        ".c = .b",
        "b: {x: 1}\nc: {x: 1}\n",
    ),
    ("b: [[1]]\nc:\n  - - 9\n", ".c = .b", "b: [[1]]\nc: [[1]]\n"),
    (
        "b: [1] # t\nc:\n  - 2 # q\n",
        ".c = .b",
        "b: [1] # t\nc: [1] # t\n",
    ),
    ("b:\n  - 1\nc: [2]\n", ".c = .b", "b:\n  - 1\nc: [1]\n"),
    ("b:\n  - 1\nc: []\n", ".c = .b", "b:\n  - 1\nc:\n  - 1\n"),
    ("b: []\nc: [9]\n", ".c = .b", "b: []\nc: []\n"),
    ("b: {x: 1}\nc: {}\n", ".c = .b", "b: {x: 1}\nc: {x: 1}\n"),
    (
        "a: &x [2]\nb: [1]\nc: *x\n",
        ".c = .b",
        "a: &x [2]\nb: [1]\nc: [1]\n",
    ),
    (
        "a: &x [2]\nb:\n  - 1\nc: *x\n",
        ".c = .b",
        "a: &x [2]\nb:\n  - 1\nc:\n  - 1\n",
    ),
    // A source the write created under a new parent has no node in the presentation tree yet.
    (
        "b: 1 # t\n",
        ".x.y = 1 | .c = .x.y",
        "b: 1 # t\nx:\n  y: 1\nc: 1\n",
    ),
];

#[test]
fn test_a_plain_assignment_copies_the_sources_comment_and_flow_style_5249() -> Result<()> {
    for &(input, filter, expected) in ROWS {
        let (output, exit_code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(exit_code, 0, "filter: {filter} on {input:?}");
        assert_eq!(output, expected, "filter: {filter} on {input:?}");
    }
    Ok(())
}

/// The value never changes, only how it is spelled.
#[test]
fn test_copying_presentation_leaves_the_values_alone_5249() -> Result<()> {
    for &(input, filter, _) in ROWS {
        let (plain, exit_code) = run_yq_stdin(filter, input, &["-o=json", "-I=0"])?;
        assert_eq!(exit_code, 0, "filter: {filter} on {input:?}");
        let (as_yaml, exit_code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(exit_code, 0, "filter: {filter} on {input:?}");
        let (roundtrip, exit_code) = run_yq_stdin(".", &as_yaml, &["-o=json", "-I=0"])?;
        assert_eq!(exit_code, 0, "filter: {filter} on {input:?}");
        assert_eq!(plain, roundtrip, "filter: {filter} on {input:?}");
    }
    Ok(())
}

/// A source a later stage rewrites has presentation that no longer describes the target, so the
/// copy is skipped: yq copies what the node held when it was read (`c: [1] # t`), this prints the
/// plain value (recorded in `limitations.md`). The source itself keeps its comment beside the
/// scalar that overwrote it (`b: 5 # t`, #5258), as yq prints it.
#[test]
fn test_a_source_rewritten_by_a_later_stage_copies_nothing_5249() -> Result<()> {
    let (output, exit_code) = run_yq_stdin(".c = .b | .b = 5", "b: [1] # t\n", &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "b: 5 # t\nc:\n  - 1\n");
    Ok(())
}

/// A target a later stage edits or replaces, and a target that is not a static path, take no copy:
/// yq prints `c: [{a: 2}] # t`, `c: 5 # t` and `c: [1] # t` for these (recorded in
/// `limitations.md`). The value is right in every one.
#[test]
fn test_targets_the_pass_does_not_reach_stay_plain_5249() -> Result<()> {
    for (input, filter, expected) in [
        (
            "b: [{a: 1}] # t\n",
            ".c = .b | .c[0].a = 2",
            "b: [{a: 1}] # t\nc:\n  - a: 2\n",
        ),
        ("b: [1] # t\n", ".c = .b | .c = 5", "b: [1] # t\nc: 5\n"),
        ("b: [1] # t\nc: 3\n", ".[] = .b", "b: [1] # t\nc:\n  - 1\n"),
    ] {
        let (output, exit_code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(exit_code, 0, "filter: {filter} on {input:?}");
        assert_eq!(output, expected, "filter: {filter} on {input:?}");
    }
    Ok(())
}

/// A target that already holds an alias, assigned a source that holds one and that a later stage
/// edits, takes no copy, so the alias pass alone clears the alias the old target held and writes
/// the source's: the same `b: [*x]` and `c: [7]` yq prints.
#[test]
fn test_an_alias_target_over_an_edited_alias_source_5249() -> Result<()> {
    let (output, exit_code) =
        run_yq_stdin(".b = .c | .c[0] = 7", "a: &x 1\nb: [*x]\nc: [*x]\n", &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: &x 1\nb: [*x]\nc: [7]\n");
    Ok(())
}

/// A value operand that is computed but still names one node (`.b // 0`, `select(true)`, `. `)
/// hands `=` that node, so the target takes its alias, explicit tag, quoting, comment and flow
/// style as it does for `.c = .b` (the computed-operand rows of #2505).
#[test]
fn test_a_computed_operand_naming_one_node_copies_it_5249() -> Result<()> {
    for (input, filter, expected) in [
        // A negative index is counted from the end of the array as the document had it.
        (
            "a: &x 1\nc: &y 2\nb: [*x, *y]\n",
            ".d = (.b[-1] // 0)",
            "a: &x 1\nc: &y 2\nb: [*x, *y]\nd: *y\n",
        ),
        // Two nodes with the same value: the one the operand reads is the one copied.
        (
            "x: &x 1\ny: 1\n",
            ".c = (.y // .x)",
            "x: &x 1\ny: 1\nc: 1\n",
        ),
        (
            "a: &x 1\nb: *x\n",
            ".c = (.b // 0)",
            "a: &x 1\nb: *x\nc: *x\n",
        ),
        (
            "a: &x 1\nb: *x\n",
            ".c = (.b | select(true))",
            "a: &x 1\nb: *x\nc: *x\n",
        ),
        (
            "a: &x 1\nb: *x\n",
            ".c = (.b | .)",
            "a: &x 1\nb: *x\nc: *x\n",
        ),
        (
            "a: !Foo x\nb: !!str 3\n",
            ".c = (.a // 1)",
            "a: !Foo x\nb: !!str 3\nc: !Foo x\n",
        ),
        (
            "a: !Foo x\nb: !!str 3\n",
            ".c = (.a | select(true))",
            "a: !Foo x\nb: !!str 3\nc: !Foo x\n",
        ),
        ("a: !Foo x\n", ".b = (.a | .)", "a: !Foo x\nb: !Foo x\n"),
        (
            "a: !Foo x\nb: !!str 3\n",
            ".b = (.a // 2)",
            "a: !Foo x\nb: !Foo x\n",
        ),
        (
            "a: &x \"q\" # ca\nb: *x # cb\nl: [1, 2] # cl\nm: {k: v}\n",
            ".c = (.nope // .b)",
            "a: &x \"q\" # ca\nb: *x # cb\nl: [1, 2] # cl\nm: {k: v}\nc: *x # cb\n",
        ),
        (
            "a: &x \"q\" # ca\nb: *x # cb\nl: [1, 2] # cl\nm: {k: v}\n",
            ".c = (.l // 0)",
            "a: &x \"q\" # ca\nb: *x # cb\nl: [1, 2] # cl\nm: {k: v}\nc: [1, 2] # cl\n",
        ),
        (
            "a: &x \"q\" # ca\nb: *x # cb\nl: [1, 2] # cl\nm: {k: v}\n",
            ".l = (.m // 0)",
            "a: &x \"q\" # ca\nb: *x # cb\nl: {k: v} # cl\nm: {k: v}\n",
        ),
        (
            "a: &x \"q\" # ca\nb: *x # cb\nl: [1, 2] # cl\nm: {k: v}\n",
            ".c.d = (.m // 0)",
            "a: &x \"q\" # ca\nb: *x # cb\nl: [1, 2] # cl\nm: {k: v}\nc:\n  d: {k: v}\n",
        ),
        (
            "a: &x \"q\" # ca\nb: *x # cb\nl: [1, 2] # cl\nm: {k: v}\n",
            "(.c, .d) = (.m // 0)",
            "a: &x \"q\" # ca\nb: *x # cb\nl: [1, 2] # cl\nm: {k: v}\nc: {k: v}\nd: {k: v}\n",
        ),
        (
            "a: &x \"q\" # ca\nb: *x # cb\nl: [1, 2] # cl\nm: {k: v}\n",
            ".c = (.a | select(true))",
            "a: &x \"q\" # ca\nb: *x # cb\nl: [1, 2] # cl\nm: {k: v}\nc: \"q\" # ca\n",
        ),
    ] {
        let (output, exit_code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(exit_code, 0, "filter: {filter} on {input:?}");
        assert_eq!(output, expected, "filter: {filter} on {input:?}");
    }
    Ok(())
}

/// A computed operand that is not a node read, or names several, keeps writing a plain value.
#[test]
fn test_a_computed_operand_that_is_not_one_node_stays_plain_5249() -> Result<()> {
    for (input, filter, expected) in [
        (
            "a: &x 1\nb: *x\n",
            ".c = (.b + 1)",
            "a: &x 1\nb: *x\nc: 2\n",
        ),
        ("a: !Foo x\n", ".b = \"lit\"", "a: !Foo x\nb: lit\n"),
        // Several results, a construction and a compound assignment are not one node read: yq
        // aliases `.c` in the first and third (the rows #5249 still tracks); the value is right.
        (
            "a: &x 1\nb: *x\n",
            ".c = (.a, .b)",
            "a: &x 1\nb: *x\nc: 1\n",
        ),
        ("a: !Foo x\n", ".c = [.a]", "a: !Foo x\nc:\n  - x\n"),
        ("a: &x 1\nb: *x\n", ".c += .b", "a: &x 1\nb: *x\nc: 1\n"),
        // An operand that falls back to a value has no node to copy.
        ("a: !Foo x\n", ".c = (.nope // 7)", "a: !Foo x\nc: 7\n"),
    ] {
        let (output, exit_code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(exit_code, 0, "filter: {filter} on {input:?}");
        assert_eq!(output, expected, "filter: {filter} on {input:?}");
    }
    Ok(())
}
