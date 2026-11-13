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
