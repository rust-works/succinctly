//! #5379: a call jq compiles to a jump (`TAIL_CALL_JQ`: a call with no arguments, last in its
//! body) runs in place, so a recursive `def` is no longer cut off at `MAX_EVAL_FRAMES` (about
//! 7,500 levels of the issue's program) or the native-stack floor. Every row's answer was
//! captured from jq 1.7.1 (`/usr/bin/jq`); jq runs these in constant space.

use anyhow::Result;

use crate::{run_jq_full, run_jq_stdin};

/// Levels deep enough to need the loop on every build: the nested form refuses at about 7,500.
const DEEP: u64 = 200_000;

fn run(filter: &str) -> Result<(String, String, i32)> {
    run_jq_full(&["-nc", filter], None)
}

#[test]
fn test_the_issue_program_answers_where_it_was_refused_5379() -> Result<()> {
    // The issue's own repro at 300,000, and past what a stack could hold: jq answers 3,000,000
    // in 13 MB.
    let levels = if cfg!(debug_assertions) {
        1_000_000
    } else {
        3_000_000
    };
    for n in [300_000, levels] {
        let filter = format!("def f: if . < {n} then . + 1 | f else . end; 0 | f");
        let (stdout, stderr, code) = run(&filter)?;
        assert_eq!(
            (stdout.as_str(), code),
            (format!("{n}\n").as_str(), 0),
            "{stderr:?}"
        );
    }
    Ok(())
}

/// One row per place jq jumps from, and the shapes the loop passes through on the way to it.
#[test]
fn test_every_tail_position_loops_in_place_5379() -> Result<()> {
    for (label, body, want) in [
        (
            "last pipe stage",
            "def f: if . < N then . + 1 | f else . end; 0 | f",
            "200000",
        ),
        (
            "elif",
            r#"def f: if . < N then . + 1 | f elif . == N then "done" else "no" end; 0 | f"#,
            r#""done""#,
        ),
        (
            "elif, the call in the last branch",
            r#"def f: if . >= N then "done" elif . < N then . + 1 | f else "no" end; 0 | f"#,
            r#""done""#,
        ),
        (
            "the condition after a stage",
            "def f: . + 1 | if . < N then f else . end; 0 | f",
            "200000",
        ),
        (
            "parenthesised",
            "def f: if . < N then (. + 1 | f) else . end; 0 | f",
            "200000",
        ),
        (
            "last comma branch, under limit",
            "[limit(N; def f: ., (.+1|f); 0|f)] | length",
            "200000",
        ),
        (
            "a wrapper def",
            "def f: if . < N then . + 1 | f else . end; def g: f; 0 | g",
            "200000",
        ),
        (
            "an outer def called from a nested one",
            "def f: def h: f; if . < N then . + 1 | h else . end; 0 | f",
            "200000",
        ),
        (
            "mutual recursion through a nested def",
            "def a: def b: . + 1 | a; if . < N then . + 1 | b else . end; 0 | a",
            "200000",
        ),
        (
            "mutual recursion, both halves nested",
            "def f: def a: def b: . + 1 | a; if . < N then . + 1 | b else . end; a; 0 | f",
            "200000",
        ),
        (
            "a builtin in the condition",
            "def f: if (. | tostring | length) < 6 then . * 10 + 1 | f else . end; 0 | f",
            "111111",
        ),
        (
            "a stage that selects",
            "def f: if . < N then . + 1 | select(. >= 0) | f else . end; 0 | f",
            "200000",
        ),
        (
            "an error at depth, caught outside",
            r#"def f: if . < N then . + 1 | f else error("boom") end; try (0 | f) catch ."#,
            r#""boom""#,
        ),
        (
            "an empty answer at depth",
            "def f: if . < N then . + 1 | f else empty end; [0 | f]",
            "[]",
        ),
        (
            "$__loc__ at the bottom",
            "def f: if . < N then . + 1 | f else $__loc__.line end; 0 | f",
            "1",
        ),
        (
            "two calls in one pipe, side by side",
            "def f: if . < N then . + 1 | f else . end; [0 | f, f]",
            "[200000,200000]",
        ),
        (
            "an object value",
            "def f: if . < N then . + 1 | f else . end; {a: (0 | f)}",
            r#"{"a":200000}"#,
        ),
        (
            "a stream of calls",
            "def f: if . < N then . + 1 | f else . end; 0 | (f, 5)",
            "200000\n5",
        ),
    ] {
        let filter = body.replace('N', &DEEP.to_string());
        let (stdout, stderr, code) = run(&filter)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (want, 0),
            "[{label}] `{filter}` -- {stderr:?}"
        );
    }
    Ok(())
}

/// What a loop in front of a consumer that stops, or a branch that is not a call, still does.
#[test]
fn test_the_loop_keeps_jqs_order_and_demand_5379() -> Result<()> {
    for (label, filter, want) in [
        (
            "a comma emits each branch before the call",
            "def f: if . < 5 then ., (. + 1 | f) else . end; [0|f]",
            "[0,1,2,3,4,5]",
        ),
        (
            "the last branch is a comma",
            r#"def f: if . < 5 then . + 1 | f else ., "end" end; [0|f]"#,
            r#"[5,"end"]"#,
        ),
        (
            "one loop per input",
            "def f: if . < 10 then . + 1 | f else . end; [range(3) | f]",
            "[10,10,10]",
        ),
        (
            "first stops a generator that tail-calls",
            "first(def f: ., (.+1|f); 0|f)",
            "0",
        ),
        (
            "limit stops it",
            "[limit(3; def f: ., (.+1|f); 0|f)]",
            "[0,1,2]",
        ),
        (
            "isempty stops it",
            "isempty(def f: ., (.+1|f); 0|f)",
            "false",
        ),
        (
            "a nested loop under reduce",
            "def f: if . < 100 then . + 1 | f else . end; reduce range(3) as $i (0; . + ($i | f))",
            "300",
        ),
        (
            "an error inside the first rounds",
            r#"def f: if . < 3 then . + 1 | f else error("boom") end; try (0 | f) catch ."#,
            r#""boom""#,
        ),
    ] {
        let (stdout, stderr, code) = run(filter)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (want, 0),
            "[{label}] `{filter}` -- {stderr:?}"
        );
    }
    Ok(())
}

/// `input` reads in order through the rounds: a stage that consumes the input stream ahead of the
/// call runs once per round, as jq runs it.
#[test]
fn test_input_is_read_once_per_round_5379() -> Result<()> {
    let filter = "def f: if . < 3 then input | f else . end; 0 | f";
    let (stdout, code) = run_jq_stdin(filter, "1 2 9 10", &["-nc"])?;
    assert_eq!((stdout.as_str(), code), ("9\n", 0));
    Ok(())
}

/// What is not a jump in jq is not one here: the call has to be last, with no arguments, outside
/// a `try`, `?` or `label`. These still nest, so they keep the guard that refuses them where the
/// native stack runs out -- an error, exit 5, not an abort.
#[test]
fn test_a_call_that_is_not_a_tail_call_still_hits_the_guard_5379() -> Result<()> {
    for (label, body) in [
        (
            "under try",
            "def f: if . < N then try (. + 1 | f) catch . else . end; 0 | f",
        ),
        (
            "under ?",
            "def f: if . < N then (. + 1 | f)? else . end; 0 | f",
        ),
        (
            "under label",
            "def f: label $out | if . < N then . + 1 | f else . end; 0 | f",
        ),
        (
            "with an argument",
            "def f($n): if $n > 0 then f($n - 1) else 0 end; f(N)",
        ),
        (
            "a generator condition",
            "def f: if (. < N | (., empty)) then . + 1 | f else . end; 0 | f",
        ),
        (
            "a stage that is not settled",
            "def f: if . < N then (. + 1 | ., empty) | f else . end; 0 | f",
        ),
        (
            "an operand of an operator",
            "def f: if . < N then 1 + (. | f) else 0 end; 0 | f",
        ),
        (
            "inside an array",
            "def f: if . < N then [. + 1 | f] | .[0] else . end; 0 | f",
        ),
    ] {
        let filter = body.replace('N', &DEEP.to_string());
        let (stdout, stderr, code) = run(&filter)?;
        assert_eq!(
            (stdout.as_str(), code),
            ("", 5),
            "[{label}] `{filter}` -- {stderr:?}"
        );
        assert!(
            stderr.contains("exceeded maximum recursion depth"),
            "[{label}] `{filter}` -- {stderr:?}"
        );
    }
    Ok(())
}
