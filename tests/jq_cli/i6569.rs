//! #6569: a `reduce`/`foreach`/`until` step that is an `if` whose chosen branch is `.` or a closed
//! value (a running max: `if $x > . then $x else . end`) answers from the branch, where the whole
//! `if` used to run on the evaluator's route every step. Every expected string was captured from
//! jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// Numbers, a float, `null` and a string, so the comparisons cross types as jq orders them.
const INPUT: &str = r#"[3, 1, 4, 1, 5, 9, 2, 6, 1.5, null, "x"]"#;

#[test]
fn if_steps_with_a_dot_or_closed_branch_answer_as_jq_6569() -> Result<()> {
    for (filter, want, want_code) in [
        (
            r"reduce .[] as $x (null; if . == null or $x > . then $x else . end)",
            "\"x\"\n",
            0,
        ),
        (
            r"reduce .[] as $x (0; if $x < . then $x else . end)",
            "null\n",
            0,
        ),
        (
            r"reduce .[] as $x (0; if true then $x else . end)",
            "\"x\"\n",
            0,
        ),
        (
            r"reduce .[] as $x (0; if false then $x else . end)",
            "0\n",
            0,
        ),
        (
            r#"reduce .[] as $x (0; if $x == 1 then "one" elif $x == 2 then . else $x end)"#,
            "\"x\"\n",
            0,
        ),
        (
            r"reduce .[] as $x (0; if $x == 9 then {k: $x} else . end)",
            "{\"k\":9}\n",
            0,
        ),
        (
            r"reduce .[] as $x (0; if $x == 9 then empty else . end)",
            "null\n",
            0,
        ),
        (
            r#"try reduce .[] as $x (0; if $x == 9 then error("nine") else . end) catch ."#,
            "\"nine\"\n",
            0,
        ),
        (
            r"reduce .[] as $x (0; if $x == 9 then (1, 2) else . end)",
            "2\n",
            0,
        ),
        (
            r"[foreach .[] as $x (0; if $x > . then $x else . end)]",
            "[3,3,4,4,5,9,9,9,9,9,\"x\"]\n",
            0,
        ),
        (
            r"[foreach .[] as $x (0; if $x > . then $x else . end; [$x, .])]",
            "[[3,3],[1,3],[4,4],[1,4],[5,5],[9,9],[2,9],[6,9],[1.5,9],[null,9],[\"x\",\"x\"]]\n",
            0,
        ),
        (
            r"0 | until(. > 3; if . == 2 then 3.5 else . + 1 end)",
            "3.5\n",
            0,
        ),
        (r"reduce .[] as $x (0; if $x > . then $x end)", "\"x\"\n", 0),
        (
            r"reduce .[] as $x (1.50; if false then $x else . end)",
            "1.50\n",
            0,
        ),
        (
            r#"{"k": 5} as $o | reduce .[] as $x (0; if true then $o else . end)"#,
            "{\"k\":5}\n",
            0,
        ),
    ] {
        let (stdout, _, code) = run_jq_full(&["-c", filter], Some(INPUT))?;
        assert_eq!((stdout.as_str(), code), (want, want_code), "{filter}");
    }
    Ok(())
}

/// A running max over 300,000 steps; the answer is jq 1.7.1's.
#[test]
fn a_long_running_max_answers_6569() -> Result<()> {
    let filter = "reduce range(300000) as $i (null; if . == null or ($i % 1000) > . then ($i % 1000) else . end)";
    let (stdout, _, code) = run_jq_full(&["-nc", filter], None)?;
    assert_eq!((stdout.as_str(), code), ("999\n", 0));
    Ok(())
}

/// `while` hands its step the state it has just emitted, a second reference; the chosen branch
/// answers all the same. The answers are jq 1.7.1's.
#[test]
fn while_if_steps_answer_as_jq_6569() -> Result<()> {
    for (filter, want) in [
        (
            r"[0 | while(. < 6; if . == 1 then 3 elif . == 4 then 5 else . + 1 end)]",
            "[0,1,3,4,5]\n",
        ),
        (
            r"[1 | while(. < 100; if . > 10 then 100 else . * 3 end)]",
            "[1,3,9,27]\n",
        ),
    ] {
        let (stdout, _, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
