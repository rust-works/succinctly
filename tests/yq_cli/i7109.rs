//! #7109: the owned step for a fold's `.[1:] + LITERAL` UPDATE is jq mode's: yq's arithmetic has its own
//! rules for the right side (#6802), so `succinctly yq` keeps the evaluator's route for the same shape.
//! `reduce` and `foreach` are jq-only surface in yq, so these run under `--jq-extensions` and have no
//! reference to be checked against: the rows are what that route answered before the step.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_slice_plus_a_literal_fold_update_keeps_the_evaluators_route_in_yq_mode_7109() -> Result<()> {
    for (filter, expected) in [
        ("reduce (0,1,2) as $i (.; .[1:] + [$i])", "[4,0,1,2]"),
        (
            "[foreach (0,1) as $i (.; .[1:] + [$i])]",
            "[[2,3,4,0],[3,4,0,1]]",
        ),
    ] {
        let (out, code) = run_yq_stdin(
            filter,
            "[1,2,3,4]",
            &["--jq-extensions", "-p=json", "-o=json", "-I=0"],
        )?;
        assert_eq!((out.trim(), code), (expected, 0), "{filter}");
    }
    Ok(())
}
