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
    // in constant space.
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
            "an assignment before the call",
            "def f: if .n < N then .n += 1 | f else .n end; {\"n\":0} | f",
            "200000",
        ),
        (
            "an update before the call",
            "def f: if .a < N then .a |= . + 1 | f else .a end; {\"a\":0} | f",
            "200000",
        ),
        (
            "an array built before the call",
            "def f: if . < N then [., 1] | add | f else . end; 0 | f",
            "200000",
        ),
        (
            "a map before the call",
            "def f: if (.[0] // 0) < N then map(. + 1) | f else .[0] end; [0] | f",
            "200000",
        ),
        (
            "an as binding in front of the call",
            "def f: if . < N then . as $x | $x + 1 | f else . end; 0 | f",
            "200000",
        ),
        (
            "an as binding around the condition",
            "def f: . as $x | if $x < N then $x + 1 | f else $x end; 0 | f",
            "200000",
        ),
        (
            "two as bindings",
            "def f: if . < N then . as $x | ($x + 1) as $y | $y | f else . end; 0 | f",
            "200000",
        ),
        (
            "an as binding of a constant",
            "def f: if . < N then 1 as $one | . + $one | f else . end; 0 | f",
            "200000",
        ),
        (
            "an as binding that yields nothing",
            "def f: if . < N then empty as $x | . + 1 | f else . end; [0 | f]",
            "[]",
        ),
        (
            "a comma after an as binding",
            "def f: if . < 5 then . as $x | ., ($x + 1 | f) else . end; [0 | f]",
            "[0,1,2,3,4,5]",
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

/// The same loop over a document: a call whose input is a node of the input goes through the
/// document evaluator, not the one above, and carries the node, then a built value, between rounds.
/// Answers captured from jq 1.7.1.
#[test]
fn test_a_call_over_the_input_document_loops_too_5379() -> Result<()> {
    for (label, doc, body, want) in [
        (
            "the input itself",
            "0",
            "def f: if . < N then . + 1 | f else . end; f",
            "200000",
        ),
        (
            "a member",
            r#"{"a":{"b":0}}"#,
            "def f: if . < N then . + 1 | f else . end; .a.b | f",
            "200000",
        ),
        (
            "one loop per element",
            "[0,1,2]",
            "def f: if . < 100000 then . + 1 | f else . end; [.[] | f]",
            "[100000,100000,100000]",
        ),
        (
            "a member read each round",
            r#"{"a":[1,2,3]}"#,
            "def f: if (.a | length) < 3 then . else .a |= .[1:] | f end; f | .a",
            "[2,3]",
        ),
        (
            "a member consumed each round",
            r#"{"n":0}"#,
            "def f: if .n < N then .n += 1 | f else .n end; f",
            "200000",
        ),
        (
            "a wrapper def",
            "0",
            "def f: if . < N then . + 1 | f else . end; def g: f; g",
            "200000",
        ),
        (
            "an as binding of the input node",
            "0",
            "def f: . as $x | if $x < N then $x + 1 | f else $x end; f",
            "200000",
        ),
        (
            "an as binding of a member",
            r#"{"a":{"n":0}}"#,
            "def f: .n as $n | if $n < N then .n = $n + 1 | f else $n end; .a | f",
            "200000",
        ),
        (
            "mutual recursion through a nested def",
            "0",
            "def a: def b: . + 1 | a; if . < N then . + 1 | b else . end; a",
            "200000",
        ),
        (
            "limit over a generator",
            "0",
            "[limit(100000; def f: ., (.+1|f); f)] | length",
            "100000",
        ),
        (
            "an error at depth, caught",
            "0",
            r#"def f: if . < N then . + 1 | f else error("boom") end; try f catch ."#,
            r#""boom""#,
        ),
    ] {
        let filter = body.replace('N', &DEEP.to_string());
        let (stdout, stderr, code) = run_jq_full(&["-c", &filter], Some(doc))?;
        assert_eq!(
            (stdout.trim_end(), code),
            (want, 0),
            "[{label}] `{filter}` over {doc} -- {stderr:?}"
        );
    }
    Ok(())
}

/// A tail loop that never ends is stopped by the round budget (`MAX_LOOP_ROUNDS`, the bound
/// `until` and `while` keep for a loop that steps in place) with the refusal the nested form
/// raised. jq spins on `def f: f; f`, in 2 MB, until it is killed. Uncatchable, like that refusal.
#[test]
fn test_a_tail_loop_that_never_ends_hits_the_round_budget_5379() -> Result<()> {
    let (stdout, stderr, code) = run(r#"[try (def f: f; f) catch "caught"]"#)?;
    assert_eq!((stdout.as_str(), code), ("", 5), "{stderr:?}");
    assert!(
        stderr.contains("f/0 exceeded maximum recursion depth"),
        "{stderr:?}"
    );
    Ok(())
}

/// A program with a dynamic path target runs on the path-mode interpreter, which keeps jq's path
/// register; the loop carries it from stage to stage. Answers captured from jq 1.7.1.
#[cfg_attr(feature = "unshared-containers", ignore = "holdout refusal (#5246)")]
#[test]
fn test_a_program_that_reads_a_path_loops_too_5379() -> Result<()> {
    for (label, doc, body, want) in [
        (
            "a path read after the loop",
            r#"{"a":0}"#,
            "def f: if . < N then . + 1 | f else . end; [(.a | f), path(.a)]",
            r#"[200000,["a"]]"#,
        ),
        (
            "a path through the loop",
            r#"{"a":{"a":{"a":0}}}"#,
            r#"def f: if type == "object" then .a | f else . end; [path(f)]"#,
            r#"[["a","a","a"]]"#,
        ),
        (
            "an update before the call",
            "[0]",
            "def f: if (.[0] // 0) < N then .[0] += 1 | f else .[0] end; [f, path(.[0])]",
            "[200000,[0]]",
        ),
    ] {
        let filter = body.replace('N', &DEEP.to_string());
        let (stdout, stderr, code) = run_jq_full(&["-c", &filter], Some(doc))?;
        assert_eq!(
            (stdout.trim_end(), code),
            (want, 0),
            "[{label}] `{filter}` over {doc} -- {stderr:?}"
        );
    }
    Ok(())
}

/// A generator that tail-calls itself holds every output it emitted while a consumer that
/// collects (`[...]`) is waiting for its end, and jq's loop runs until its heap is gone. The
/// outputs a loop emits ahead of its calls are bounded (2^22), apart from its rounds, so the
/// collecting consumer is refused in a few hundred MB, not gigabytes.
#[test]
fn test_a_tail_generator_under_a_collector_is_bounded_5379() -> Result<()> {
    let (stdout, stderr, code) = run("[def f: ., (. + 1 | f); 0 | f] | length")?;
    assert_eq!((stdout.as_str(), code), ("", 5), "{stderr:?}");
    assert!(
        stderr.contains("f/0: maximum outputs exceeded"),
        "{stderr:?}"
    );
    // A consumer that stops is not charged for what it never asks for.
    let (stdout, stderr, code) = run("first(def f: ., (. + 1 | f); 0 | f)")?;
    assert_eq!((stdout.trim_end(), code), ("0", 0), "{stderr:?}");
    Ok(())
}

/// An `as` whose body never names the variable binds nothing, and over a document the node is
/// not read (#3856): the answer does not depend on whether the call is in tail position. The
/// document is one this reader accepts and jq's parser does not (a lone escape), so the row has
/// no jq capture; the non-tail spelling is the reference.
#[test]
fn test_an_unread_as_binding_does_not_decode_the_node_5379() -> Result<()> {
    let doc = r#"{"a":"\q","b":1}"#;
    for filter in [
        "def g: 5; def f: .a as $u | g; f",
        "def g: 5; def f: .a as $u | [g] | .[0]; f",
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(doc))?;
        assert_eq!(
            (stdout.trim_end(), code),
            ("5", 0),
            "`{filter}` -- {stderr:?}"
        );
    }
    Ok(())
}
