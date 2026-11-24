//! #7893: the outputs of a recursive generator that emits after its recursive call (`def f: if
//! . > 0 then (. - 1 | f), . else . end; [N | f]`) each went back up through every level of the
//! recursion on their way to the one consumer, N²/2 calls for N outputs, where jq hands each to
//! the consumer from where it is produced. Two things stood between a level's call and the sink
//! its caller was given:
//!
//! - since #7856, a wrapper that checks the native stack before an output is handed on, around
//!   every call to a def that may recurse, even where the sink was already such a wrapper (the
//!   whole of the cost of `def f($n): if $n > 0 then f($n - 1), $n else $n end; [f(N)]`);
//! - since #7702, for a def that threads its count through `.`, the owned evaluator's re-index
//!   bridge, which ran the `,` and the pipe `. - 1 | f` on a document of its own and gave the
//!   call a sink of the bridge's (found by building the merges either side of it).
//!
//! A call handed the wrapper of the call that handed its sink on is no longer wrapped again, and
//! a `,` or a pipe that ends in the recursive call runs on the owned tree with the sink it was
//! given.
//!
//! The pins of the work are `def_output_checks_tests_7893` in `src/jq/eval.rs`, which counts the
//! checks, and `a_comma_that_hands_on_a_recursion_answers_as_the_bridge_7893` in
//! `src/jq/eval/owned_call.rs`. These rows pin the answers, captured from jq 1.7.1 (jq 1.8.2
//! gives the same), over the shapes the change reaches: the def emitting after its call and
//! before it, `$`-parameters, a closure parameter, a second def between the levels, a tree of two
//! calls, and each consumer (an array, `reduce`, `foreach`, `first`, `limit`, `select`).

use anyhow::Result;

use crate::run_jq_full;

/// The issue's def: it emits its own value after its recursive call's outputs.
const AFTER: &str = "def f: if . > 0 then (. - 1 | f), . else . end;";

/// (program, stdout) for `succinctly jq -nc <program>`, each exit 0 with nothing on stderr.
fn cases() -> Vec<(String, &'static str)> {
    vec![
        (format!("{AFTER} [2000 | f] | length"), "2001\n"),
        (
            "def f($n): if $n > 0 then f($n - 1), $n else $n end; [f(2000)] | length".to_owned(),
            "2001\n",
        ),
        (
            format!("{AFTER} reduce (2000 | f) as $x (0; . + $x)"),
            "2001000\n",
        ),
        (
            format!("{AFTER} [foreach (2000 | f) as $x (0; . + $x)] | .[-1]"),
            "2001000\n",
        ),
        (format!("{AFTER} first(2000 | f)"), "0\n"),
        (format!("{AFTER} [limit(3; 2000 | f)]"), "[0,1,2]\n"),
        (
            format!("{AFTER} [2000 | f | select(. % 500 == 0)]"),
            "[0,500,1000,1500,2000]\n",
        ),
        (
            "def f(g): if . > 0 then (. - 1 | f(g)), g else g end; [2000 | f(.)] | length"
                .to_owned(),
            "2001\n",
        ),
        (
            "def f: def g: if . > 0 then (. - 1 | f), . else . end; g; [2000 | f] | add".to_owned(),
            "2001000\n",
        ),
        (
            "def t: if . > 0 then (. - 1 | t), (. - 1 | t), . else . end; [9 | t] | length"
                .to_owned(),
            "1023\n",
        ),
        (
            "def f: if . > 0 then ., (. - 1 | f) else . end; [2000 | f] | length".to_owned(),
            "2001\n",
        ),
    ]
}

#[test]
fn a_recursive_generators_outputs_reach_their_consumer_as_in_jq_7893() -> Result<()> {
    for (program, want) in cases() {
        let (out, err, code) = run_jq_full(&["-nc", &program], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (want, "", 0),
            "{program}"
        );
    }
    Ok(())
}

/// The same over an input document, which the document route's evaluator reads.
#[test]
fn a_recursive_generator_over_an_input_answers_as_in_jq_7893() -> Result<()> {
    let program = format!("{AFTER} [f] | length");
    let (out, err, code) = run_jq_full(&["-c", &program], Some("2000\n"))?;
    assert_eq!((out.as_str(), err.as_str(), code), ("2001\n", "", 0));
    Ok(())
}
