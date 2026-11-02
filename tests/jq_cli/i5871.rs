//! #5871: a body rebuilt for each value whose pipe ends in a call to a `def` hands its rest on as a
//! link of the stateless stages ahead of the call plus the call itself, cloned. The answers are
//! unchanged by it: every expected string was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

fn run(program: &str) -> Result<String> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "-c", program], None)?;
    assert_eq!(code, 0, "{program}: {stderr}");
    Ok(stdout.trim_end().to_string())
}

#[test]
fn a_def_call_after_stages_that_read_the_variable_answers_as_jq_5871() -> Result<()> {
    for (program, want) in [
        (
            "def f: . + 1; [range(0;50) | . as $x | (. + $x | . + $x | . + $x | f)] | add",
            "4950",
        ),
        // A recursive call is the stateful stage: it must start empty for each value.
        (
            "def fib: if . < 2 then . else (. - 1 | fib) + (. - 2 | fib) end; \
             [range(0;10) | . as $x | (. + 0 | . * 1 | . + $x - $x | fib)]",
            "[0,1,1,2,3,5,8,13,21,34]",
        ),
        // Two calls: the second sits past stages the first's tail still holds.
        (
            "def f: . + 1; def g: . * 2; \
             [range(0;5) | . as $x | (. + $x | . + $x | f | . + $x | g)]",
            "[2,10,18,26,34]",
        ),
        // The call yields more than one value.
        (
            "def f: ., . + 1; [range(0;3) | . as $x | (. + $x | . + $x | . + $x | f)]",
            "[0,1,4,5,8,9]",
        ),
        (
            "def h(g): g + 1; [range(0;5) | . as $x | (. + $x | . + $x | h(. * 2))]",
            "[1,7,13,19,25]",
        ),
        // An error raised in the prefix, and a generator in it that `limit` stops early.
        (
            "def f: . + 1; [range(0;4) | . as $x | \
             try (. + $x | . + $x | if . > 4 then error(\"big\") else . end | f) catch \"caught\"]",
            r#"[1,4,"caught","caught"]"#,
        ),
        (
            "def f: . + 1; [limit(7; range(0;100) | . as $x | (. + $x | . + $x | (., 10) | f))]",
            "[1,11,4,11,7,11,10]",
        ),
        (
            "def f: if . > 0 then . - 1 | f else . end; \
             [range(0;6) | . as $x | (. + $x | . + $x | . + $x | f)]",
            "[0,0,0,0,0,0]",
        ),
        (
            "def f: [.]; [range(0;3) | . as $x | (. + $x | . + $x | . + $x | f | .[0] + $x)]",
            "[0,5,10]",
        ),
    ] {
        assert_eq!(run(program)?, want, "{program}");
    }
    Ok(())
}

/// The issue's shape with a pipe of 80 stages. The cost of it is pinned by a count of cloned
/// stages in `eval.rs`, not by time.
#[test]
fn a_long_pipe_ending_in_a_def_call_answers_as_jq_5871() -> Result<()> {
    let body = vec![". + $x"; 79].join(" | ");
    let program = format!("def f: . + 1; [range(0;2000) | . as $x | ({body} | f)] | add");
    // 2000 values, each `i * 80 + 1`.
    assert_eq!(run(&program)?, "159922000");
    Ok(())
}
