//! The repetition loop `jq_bench` and `yq_bench` share (#5409, #6417): both tools run in every
//! repetition, in an alternating order, and a tool that exits non-zero fails its row.
//!
//! Not in `bench_runner`, whose module is only built with the `bench-runner` feature: the two
//! runners are always built.

use anyhow::Result;

/// Which tool a repetition runs: the reference (jq or yq) or succinctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tool {
    Reference,
    Succinctly,
}

/// Which tool runs first in repetition `rep`: the reference on even reps, succinctly on odd ones,
/// so neither tool always runs second (#5409, #6417; benchmarking.md rule 1).
pub(crate) fn rep_order(rep: usize) -> [Tool; 2] {
    if rep % 2 == 0 {
        [Tool::Reference, Tool::Succinctly]
    } else {
        [Tool::Succinctly, Tool::Reference]
    }
}

/// Run `warmup` untimed and then `runs` timed repetitions, each running both tools in
/// [`rep_order`]'s order. One counter spans the warmups and the timed reps, so the order keeps
/// alternating across the boundary rather than running one tool twice in a row there. With an odd
/// total one tool leads one rep more often; no order of an odd count can balance that. Returns the
/// timed results per tool (reference first), in rep order. A warmup's error is ignored; a timed
/// rep's error aborts the row.
pub(crate) fn interleaved_reps<T>(
    warmup: usize,
    runs: usize,
    mut run_reference: impl FnMut() -> Result<T>,
    mut run_succinctly: impl FnMut() -> Result<T>,
) -> Result<(Vec<T>, Vec<T>)> {
    for rep in 0..warmup {
        for tool in rep_order(rep) {
            let _ = match tool {
                Tool::Reference => run_reference(),
                Tool::Succinctly => run_succinctly(),
            };
        }
    }
    let mut reference = Vec::with_capacity(runs);
    let mut succinctly = Vec::with_capacity(runs);
    for rep in warmup..warmup + runs {
        for tool in rep_order(rep) {
            match tool {
                Tool::Reference => reference.push(run_reference()?),
                Tool::Succinctly => succinctly.push(run_succinctly()?),
            }
        }
    }
    Ok((reference, succinctly))
}

/// Fail the row when a timed tool exits non-zero (`/usr/bin/time` exits with the child's status):
/// a tool that rejects the query would otherwise be timed on its near-instant error (#5409, #6417).
pub(crate) fn ensure_success(
    program: &str,
    status: std::process::ExitStatus,
    stderr: &str,
) -> Result<()> {
    if status.success() {
        return Ok(());
    }
    let first = stderr.lines().next().unwrap_or("").trim();
    anyhow::bail!("`{program}` exited with {status}: {first}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// #5409, #6417: each repetition runs both tools, the order flipping every rep, warmups
    /// included, and only the timed reps are returned (benchmarking.md rule 1).
    #[test]
    fn reps_interleave_the_tools_and_alternate_the_order() {
        use std::cell::RefCell;
        let log = RefCell::new(Vec::new());
        let (reference, succ) = interleaved_reps(
            2,
            3,
            || {
                let mut l = log.borrow_mut();
                l.push("ref");
                Ok(l.len())
            },
            || {
                let mut l = log.borrow_mut();
                l.push("succ");
                Ok(l.len())
            },
        )
        .unwrap();
        assert_eq!(
            log.into_inner(),
            [
                // warmup reps 0 and 1
                "ref", "succ", "succ", "ref", //
                // timed reps 2, 3 and 4: the order carries on across the boundary
                "ref", "succ", "succ", "ref", "ref", "succ",
            ]
        );
        // The values are the call positions: the warmups' four are discarded.
        assert_eq!(reference, [5, 8, 9]);
        assert_eq!(succ, [6, 7, 10]);
        // After one warmup (reference first) the first timed rep leads with succinctly.
        let log = RefCell::new(Vec::new());
        interleaved_reps(
            1,
            1,
            || -> Result<()> {
                log.borrow_mut().push("ref");
                Ok(())
            },
            || {
                log.borrow_mut().push("succ");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(log.into_inner(), ["ref", "succ", "succ", "ref"]);
    }

    #[test]
    fn a_failed_timed_rep_aborts_and_a_failed_warmup_does_not() {
        let mut calls = 0;
        let err = interleaved_reps(
            1,
            2,
            || -> Result<()> {
                calls += 1;
                if calls == 1 {
                    anyhow::bail!("warmup failure is ignored")
                }
                anyhow::bail!("timed failure")
            },
            || Ok(()),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "timed failure");
        assert_eq!(calls, 2, "the row stops at the first timed failure");
    }

    #[cfg(unix)]
    #[test]
    fn a_non_zero_exit_names_the_program_and_its_first_stderr_line() {
        let status = Command::new("sh").args(["-c", "exit 3"]).status().unwrap();
        let err = ensure_success("sh", status, "boom\nmore").unwrap_err();
        assert!(err.to_string().contains("`sh` exited with"), "{err}");
        assert!(err.to_string().ends_with(": boom"), "{err}");
        let ok = Command::new("true").status().unwrap();
        assert!(ensure_success("true", ok, "").is_ok());
    }
}
