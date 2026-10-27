//! #5389 (the review of #5380, which lifted #5292's `until`/`while` step cap): the loops run past
//! 100,000 rounds on every route, stop for a consumer that stops, and are still bounded where a
//! fork or a trailing error multiplies the work. Rows marked jq are captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// An in-place loop past the old cap through the document route (`each_loop_generic`): a document
/// node seeds it. The literal-seeded rows in `src/jq/eval.rs` take the value evaluator; no CLI test
/// ran this route past 100,000 rounds, so a regression to the old cap here passed every suite.
#[test]
fn test_in_place_loops_past_the_old_cap_through_the_document_route() -> Result<()> {
    for (filter, want) in [
        (".n | until(. >= 300000; . + 1)", "300000"),
        ("[.n | while(. < 300000; . + 1)] | length", "300000"),
        (".n | first(while(true; . + 1))", "0"),
        ("[.n | limit(3; while(true; . + 1))]", "[0,1,2]"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(r#"{"n":0}"#))?;
        assert_eq!(
            (stdout.trim_end(), code),
            (want, 0),
            "`{filter}`: {stderr:?}"
        );
    }
    Ok(())
}

/// A consumer that stops ends the value evaluator's loop at the state it wanted (jq 1.7.1: all of
/// these answer at once). The eager arm collected every state first and ran the whole round cap.
#[test]
fn test_a_consumer_that_stops_ends_the_loop() -> Result<()> {
    for (filter, want) in [
        ("first(0 | while(true; .+1))", "0"),
        ("[limit(5; 1 | while(true; . * 2))]", "[1,2,4,8,16]"),
        ("isempty(0 | while(true; .+1))", "false"),
        ("any(0 | while(true; .+1); . > 5)", "true"),
        (
            r#"first({"a":1,"b":[1,2,3]} | while(true; .a += 1))"#,
            r#"{"a":1,"b":[1,2,3]}"#,
        ),
        (
            "label $out | first(0 | while(true; .+1)) | ., break $out",
            "0",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (want, 0),
            "`{filter}`: {stderr:?}"
        );
    }
    Ok(())
}

/// ...and runs no round past the state it took: the update's side effect is the witness (jq 1.7.1
/// writes no `DEBUG` line for `first`, since it stops before stepping the first state; the eager arm
/// wrote 16,777,216 of them).
#[test]
fn test_a_consumer_that_stops_runs_no_further_round() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-nc", "first(0 | while(true; (.+1) | debug))"], None)?;
    assert_eq!((stdout.trim_end(), code), ("0", 0));
    assert_eq!(stderr, "", "a round ran after the consumer stopped");
    Ok(())
}

/// A loop that forks once and then steps in place is as long as it is (jq 1.7.1:
/// `[300000,300000]`): a fork spends the fork budget once, not once per later round.
#[test]
fn test_a_loop_that_forks_once_is_not_metered_for_its_length() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-nc",
            "[0 | until(. >= 300000; if . == 0 then (1, 2) else .+1 end)]",
        ],
        None,
    )?;
    assert_eq!(
        (stdout.trim_end(), code),
        ("[300000,300000]", 0),
        "{stderr:?}"
    );
    Ok(())
}

/// Rows that run every program the same way: expect the uncatchable cap error, exit 5.
fn assert_capped(filters: &[&str]) -> Result<()> {
    for filter in filters {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(code, 5, "`{filter}`: stdout {stdout:?} stderr {stderr:?}");
        assert!(
            stderr.contains("maximum iterations exceeded"),
            "`{filter}`: stderr {stderr:?}"
        );
        assert!(
            !stdout.contains("caught"),
            "`{filter}`: the handler ran: {stdout:?}"
        );
    }
    Ok(())
}

/// What stays bounded on the value evaluator, and uncatchable.
#[test]
fn test_forks_stay_bounded() -> Result<()> {
    assert_capped(&[
        // A tree of forks: every round forks, 100,000 forks spend the budget.
        "0 | until(false; ., .)",
        "[0 | while(true; ., .)] | length",
        "try (0 | until(false; ., .)) catch \"caught\"",
    ])
}

/// ...and through the path interpreter, where a round that answers once and then raises is a fork.
/// A dynamic path target is refused by the `unshared-containers` holdout (#5246).
#[test]
#[cfg_attr(feature = "unshared-containers", ignore = "holdout refusal (#5246)")]
fn test_forks_stay_bounded_through_the_path_interpreter() -> Result<()> {
    assert_capped(&[
        "[path(until((false, error(\"x\")); .))] | length",
        "try path(until(false; ., .)) catch \"caught\"",
    ])
}
