//! #5292: the fold step budget meters the INIT forks past the first, not the first fork's length.
//!
//! In yq mode `path(reduce ...)`/`path(foreach ...)` (with `--jq-extensions`, which gates `path`) is
//! the one route that resolves a fold through `resolve_reduce`/`resolve_foreach`; jq mode sends
//! `path(...)` to the path interpreter, which has no fold budget. A single-fork fold over 200,000
//! steps used to stop with `maximum iterations exceeded`; real yq has no cap.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

const ARGS: &[&str] = &["--jq-extensions", "-n", "-o=json", "-I0"];

#[test]
fn test_a_single_fork_path_fold_runs_past_the_old_step_cap_5292() -> Result<()> {
    for (filter, want) in [
        (
            "path(reduce (range(100000), range(100000)) as $x (.; .))",
            "[]\n",
        ),
        (
            "[path(foreach (range(100000), range(100000)) as $x (.; .))] | length",
            "200000\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, "", ARGS)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}

#[test]
fn test_the_forks_past_the_first_are_still_metered_in_a_path_fold_5292() -> Result<()> {
    // Three INIT forks over 50,001 elements: the first is free, the second spends
    // 50,001 of the 100,000 steps and the third runs out.
    for (filter, want) in [
        (
            "path(reduce range(50001) as $x ((0,1,2); .))",
            "reduce: maximum iterations exceeded",
        ),
        (
            "[path(foreach range(50001) as $x ((0,1,2); .; .))] | length",
            "foreach: maximum iterations exceeded",
        ),
        // One fork, but 100,000 EXTRACT evaluations past each element's first: the
        // fan-out inside a step is what the budget bounds there.
        (
            "[path(foreach (1,2,3) as $x (.; range(50001); .))] | length",
            "foreach: maximum iterations exceeded",
        ),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, "", ARGS)?;
        assert_ne!(code, 0, "{filter}: stdout {stdout:?}");
        assert!(stderr.contains(want), "{filter}: stderr {stderr:?}");
    }
    Ok(())
}
