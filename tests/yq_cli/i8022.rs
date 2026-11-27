//! #8022: yq's `reduceOperator` (`pkg/yqlib/operator_reduce.go`, v4.53.3) reads the whole source
//! of `SRC as $x ireduce (INIT; UPDATE)` before it evaluates INIT, so an error anywhere in SRC wins
//! over INIT's and UPDATE's, and SRC is read to its end even when INIT yields nothing or UPDATE
//! drops every accumulator. `succinctly yq` evaluated INIT first and stopped reading SRC once there
//! was nothing to fold. Every row is captured from yq v4.53.3, as `yq -o=json -I0 FILTER` on
//! `[1, 2]` unless it says otherwise.

use anyhow::Result;

use crate::{run_jq_stdin_with_stderr, run_yq_stdin_with_stderr};

const INPUT: &str = "[1, 2]\n";
const JSON: &[&str] = &["-o", "json", "-I0"];

/// Filters yq v4.53.3 fails with exactly this error and no output.
const ERROR_ROWS: &[(&str, &str)] = &[
    // An error in SRC wins over INIT's.
    ("error(\"s\") as $x ireduce (error(\"i\"); .)", "Error: s"),
    (
        "(1, error(\"s\")) as $x ireduce (error(\"i\"); .)",
        "Error: s",
    ),
    (
        "(error(\"s\"), 1) as $x ireduce (error(\"i\"); .)",
        "Error: s",
    ),
    // SRC is read when INIT yields nothing.
    ("error(\"s\") as $x ireduce (select(false); .)", "Error: s"),
    (
        "error(\"s\") as $x ireduce (select(false); error(\"u\"))",
        "Error: s",
    ),
    // ...and after UPDATE dropped every accumulator, or failed.
    (
        "(1, error(\"s\")) as $x ireduce (0; select(false))",
        "Error: s",
    ),
    (
        "(1, error(\"s\")) as $x ireduce (0; error(\"u\"))",
        "Error: s",
    ),
    ("error(\"s\") as $x ireduce (0; error(\"u\"))", "Error: s"),
    // #8001's block UPDATE drops the accumulators at the first node; the rest is still read.
    (
        "(.[] | error(\"s\")) as $x ireduce (select(false); . + $x; 1)",
        "Error: s",
    ),
    // Inside an alternative, a collect, a binding and a write.
    (
        "(error(\"s\") as $x ireduce (error(\"i\"); .)) // \"alt\"",
        "Error: s",
    ),
    ("[error(\"s\") as $x ireduce (error(\"i\"); .)]", "Error: s"),
    (
        "(error(\"s\") as $x ireduce (error(\"i\"); .)) as $y | 1",
        "Error: s",
    ),
    (
        ".[0] |= (error(\"s\") as $x ireduce (error(\"i\"); .))",
        "Error: s",
    ),
    (
        ".[0] |= ((1, error(\"s\")) as $x ireduce (0; select(false)))",
        "Error: s",
    ),
    // With no error in SRC, INIT's error comes next, then UPDATE's.
    (".[] as $x ireduce (error(\"i\"); error(\"u\"))", "Error: i"),
    (".[] as $x ireduce (error(\"i\"); .)", "Error: i"),
    ("[] as $x ireduce (error(\"i\"); .)", "Error: i"),
    ("select(false) as $x ireduce (error(\"i\"); .)", "Error: i"),
    (".[] as $x ireduce ((0, error(\"i\")); . + $x)", "Error: i"),
    (".[] as $x ireduce (0; error(\"u\"))", "Error: u"),
];

#[test]
fn the_source_is_read_before_init_8022() -> Result<()> {
    for (filter, error) in ERROR_ROWS {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, INPUT, JSON)?;
        assert_eq!(
            (stdout.as_str(), stderr.trim_end(), code),
            ("", *error, 1),
            "{filter}"
        );
    }
    Ok(())
}

/// The same order with no input document (`-n`) and over the whole document list (`--eval-all`),
/// where yq v4.53.3 answers as above; `.a` reads `1` and `2` from the two documents.
#[test]
fn the_order_holds_with_null_input_and_eval_all_8022() -> Result<()> {
    let docs = "a: 1\n---\na: 2\n";
    for (filter, input, args, error) in [
        (
            "error(\"s\") as $x ireduce (error(\"i\"); .)",
            "",
            &["-n", "-o", "json", "-I0"][..],
            "Error: s",
        ),
        (
            "(1, error(\"s\")) as $x ireduce (0; select(false))",
            "",
            &["-n", "-o", "json", "-I0"][..],
            "Error: s",
        ),
        (
            "(.a, error(\"s\")) as $x ireduce (select(false); .)",
            docs,
            &["-o", "json", "-I0"][..],
            "Error: s",
        ),
        (
            "(.a, error(\"s\")) as $x ireduce (error(\"i\"); .)",
            docs,
            &["--eval-all", "-o", "json", "-I0"][..],
            "Error: s",
        ),
        (
            "(.a, error(\"s\")) as $x ireduce (0; select(false))",
            docs,
            &["--eval-all", "-o", "json", "-I0"][..],
            "Error: s",
        ),
        (
            ". as $x ireduce (error(\"i\"); .)",
            docs,
            &["--eval-all", "-o", "json", "-I0"][..],
            "Error: i",
        ),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, input, args)?;
        assert_eq!(
            (stdout.as_str(), stderr.trim_end(), code),
            ("", error, 1),
            "{filter} {args:?}"
        );
    }
    Ok(())
}

/// A fold with no error still folds the whole source into every accumulator.
#[test]
fn a_fold_without_errors_is_unchanged_8022() -> Result<()> {
    for (filter, expected) in [
        (".[] as $x ireduce ((0, 10); . + $x)", "3\n13\n"),
        (".[] as $x ireduce (0; . + $x)", "3\n"),
        (".[] as $x ireduce (select(false); . + $x)", ""),
        (".[] as $x ireduce (0; select(false))", ""),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, INPUT, JSON)?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}: {stderr}");
    }
    Ok(())
}

/// jq's `reduce` evaluates INIT first and never reads the source of an empty INIT, and is
/// unchanged (jq 1.7.1: `i`, nothing, `s`). yq-mode `reduce` (behind `--jq-extensions`) takes
/// yq's order, as it takes yq's accumulator list (#6802): a recorded divergence from jq, where
/// `reduce error("s") as $x (error("i"); .)` is `i` (`docs/compliance/yq/limitations.md`).
#[test]
fn jq_reduce_keeps_init_first_8022() -> Result<()> {
    for (filter, expected_stderr, expected_code) in [
        ("reduce error(\"s\") as $x (error(\"i\"); .)", Some("i"), 5),
        ("reduce error(\"s\") as $x (empty; .)", None, 0),
        ("reduce (1, error(\"s\")) as $x (0; empty)", Some("s"), 5),
    ] {
        let (stdout, stderr, code) = run_jq_stdin_with_stderr(filter, "null\n", &["-c"])?;
        assert_eq!((stdout.as_str(), code), ("", expected_code), "{filter}");
        match expected_stderr {
            Some(message) => assert!(
                stderr.starts_with("jq: error")
                    && stderr.trim_end().ends_with(&format!(": {message}")),
                "{filter}: {stderr}"
            ),
            None => assert_eq!(stderr, "", "{filter}"),
        }
    }
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(
        "reduce error(\"s\") as $x (error(\"i\"); .)",
        INPUT,
        &["--jq-extensions", "-o", "json", "-I0"],
    )?;
    assert_eq!(
        (stdout.as_str(), stderr.trim_end(), code),
        ("", "Error: s", 1)
    );
    Ok(())
}

/// Only an ordinary error waits for the rest of the source. A `break` and a `halt` (behind
/// `--jq-extensions`; yq has neither) end the fold at once, as in jq's `reduce` (jq 1.7.1: nothing,
/// and `bye`), so a later error in the source never outranks them and an endless source stops at
/// the `break`.
#[test]
fn break_and_halt_end_the_fold_at_once_8022() -> Result<()> {
    let ext = &["--jq-extensions", "-n", "-o", "json", "-I0"];
    for filter in [
        "label $f | (1, 2, error(\"s\")) as $x ireduce (0; break $f)",
        "label $f | range(infinite) as $x ireduce (0; break $f)",
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, "", ext)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("", "", 0),
            "{filter}"
        );
    }
    let filter = "(1, error(\"s\")) as $x ireduce (0; \"bye\" | halt_error)";
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, "", ext)?;
    assert_eq!((stdout.as_str(), stderr.as_str(), code), ("", "bye", 1));
    Ok(())
}

/// The step budget's refusal (#2132) is uncatchable and ends the fold at once: a union in UPDATE
/// that doubles the list past 100,000 accumulators over 20 elements is refused before the
/// source's own error is read.
#[test]
fn the_step_budget_refusal_is_not_held_8022() -> Result<()> {
    let input = format!("{:?}\n", (0..20).collect::<Vec<i32>>());
    let filter = "(.[], error(\"s\")) as $x ireduce (0; (. + 1, . + 2))";
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, &input, JSON)?;
    assert_eq!(
        (stdout.as_str(), stderr.trim_end(), code),
        ("", "Error: reduce: maximum iterations exceeded", 1)
    );
    Ok(())
}
