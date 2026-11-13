//! #7171: yq's `filter(f)` runs once per node of its context (`operator_filter.go`), so over an empty
//! context it answers nothing, where `[.[] | select(f)]` (an array construction) answers `[]` once. Each row
//! was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn filter_over_an_empty_context_answers_nothing_7171() -> Result<()> {
    for (filter, input, expected) in [
        (".b[]? | filter(. == 1)", "-1", ""),
        (".b[]? | filter(. == 1)", "{\"a\":1}", ""),
        (".zz | filter(.)", "{\"a\":1}", "[]\n"),
        (".a | filter(. == 1)", "{\"a\":1}", "[]\n"),
        (".l | filter(. > 1)", "{\"l\":[1,2,3]}", "[2,3]\n"),
        (".l | filter(filter(.))", "{\"l\":[1,2,3]}", "[1,2,3]\n"),
        (
            "[.l, .l] | .[] | filter(. == 2)",
            "{\"l\":[1,2,3]}",
            "[2]\n[2]\n",
        ),
        (". *=? (.b[]? | filter(. == 1))", "{\"a\":1}", "{\"a\":1}\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, input, &["-o", "json", "-I0"])?;
        assert_eq!(
            (stdout.as_str(), code),
            (expected, 0),
            "{filter} over {input}"
        );
    }
    Ok(())
}

/// The array it stands for is still read off the cursor, so comments, quotes and anchors of the children
/// that pass print as before (a regression of binding the node first, found in review).
#[test]
fn filter_keeps_the_presentation_of_the_children_it_passes_7171() -> Result<()> {
    let input = "l:\n  - a: 1 # one\n  # mid\n  - b: &x 2\n  - *x\n  - \"q\" # tail\n";
    for (filter, expected) in [
        (".l | filter(. == \"q\")", "- \"q\" # tail\n"),
        (".l | filter(.a)", "- a: 1 # one\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}");
    }
    Ok(())
}
