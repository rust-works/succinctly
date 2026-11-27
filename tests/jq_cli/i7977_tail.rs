//! #7977 step 4: the call machine's zero-argument tail loop, and the fork points a loop's round
//! leaves behind, with `SUCCINCTLY_CALL_MACHINE=1`.
//!
//! A `while` or `until` whose update leaves a finished `try` or label behind kept one fork point
//! per round on the machine, and met its byte budget at 1-2 million rounds, where the shipped
//! evaluators answer in constant memory (and jq in memory that grows with the rounds: 3.2 GB at
//! four million for the label row). Each row is checked against the run with the switch off, and
//! against the decision log, so a row the machine declined cannot pass.

use std::path::Path;
use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

const SWITCH: &str = "SUCCINCTLY_CALL_MACHINE";
const LOG: &str = "SUCCINCTLY_DEBUG_CALL_MACHINE_LOG";

/// `(stdout, stderr, exit, decisions)` of `succinctly jq -nc <filter>`, with the switch on when
/// `on`. `decisions` holds the decision log's routing lines (`taken`, `declined`).
fn run(filter: &str, on: bool, dir: &Path) -> Result<(String, String, i32, String)> {
    let log = dir.join("decisions.log");
    let (output, exit) = spawn_with_signal_retry(
        || {
            let _ = std::fs::remove_file(&log);
            let mut command = Command::new(succinctly_bin());
            command.args(["jq", "-nc", filter]).env(LOG, &log);
            if on {
                command.env(SWITCH, "1");
            } else {
                command.env_remove(SWITCH);
            }
            command
        },
        None,
    )?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        exit,
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter(|line| *line != "compiled")
            .map(|line| format!("{line}\n"))
            .collect(),
    ))
}

/// `filter` is taken by the machine and answers `want`, as it does with the switch off.
fn assert_answers_as_shipped(filter: &str, want: &str, dir: &Path) -> Result<()> {
    let (stdout, stderr, exit, decisions) = run(filter, true, dir)?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), exit),
        (want, "", 0),
        "on: {filter}"
    );
    assert_eq!(decisions, "taken\n", "{filter}");
    let (stdout, stderr, exit, _) = run(filter, false, dir)?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), exit),
        (want, "", 0),
        "off: {filter}"
    );
    Ok(())
}

/// The rows of #7977's step-2 follow-up: four million rounds of an update that leaves a finished
/// `try`, a finished label, or `first`'s label with the `break` that ends it. Each was refused at
/// the byte budget (`exceeded maximum recursion depth`) before its dead fork points were dropped.
#[test]
fn a_loop_past_a_finished_try_or_label_runs_in_constant_state_7977() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for update in ["try (. + 1) catch 0", "label $x | . + 1", "first(. + 1, 0)"] {
        let filter = format!("def f: f; 0 | until(. >= 4000000; {update})");
        assert_answers_as_shipped(&filter, "4000000\n", dir.path())?;
    }
    Ok(())
}

/// A `try` or label a later round can still reach is kept, so its output, its `catch` and a
/// pending `break` answer as in jq 1.7.1 (captured with `/usr/bin/jq -nc`), and the zero-argument
/// tail loop answers as the shipped route does with and without a fork point over its frame.
#[test]
fn a_try_or_label_still_reachable_answers_as_jq_7977() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for (filter, want) in [
        (
            r#"def f: f; [0 | until(. >= 3; try (. + 1, error("x")) catch 100)]"#,
            "[3,100,100,100]\n",
        ),
        (
            "def f: f; [label $out | 0 | until(. >= 3; (. + 1, break $out))]",
            "[3]\n",
        ),
        (
            "def f: f; [0 | until(. >= 3; label $x | try (. + 1, break $x) catch 99)]",
            "[3,99,99,99]\n",
        ),
        (
            r#"def f: f; try (0 | until(. >= 3; try (. + 1) catch 0) | error("later")) catch "outer""#,
            "\"outer\"\n",
        ),
        (
            "def f: if . < 100000 then . + 1 | f else . end; 0 | f",
            "100000\n",
        ),
        (
            "[def f: if . < 3 then (. + 1, . + 10) | f else . end; 0 | f]",
            "[3,12,11,10]\n",
        ),
    ] {
        assert_answers_as_shipped(filter, want, dir.path())?;
    }
    Ok(())
}
