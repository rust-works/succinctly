//! #5679: a pipe of more than 128 stages hands each call a bounded piece of the stages after it (the
//! rest of the stage's block and one link to the blocks after it), where it copied the whole rest and
//! held the copy while the rest ran, so `n` stages held `n` copies of up to `n` stages: 3,000 stages of
//! `. + 1` took 1.4 GB. Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`); stderr wording is
//! not compared. What each call hands on is pinned by `test_a_long_pipe_hands_on_a_bounded_tail_5679`
//! in `expr.rs`.

use anyhow::Result;

use crate::run_jq_full;

fn pipe(stage: &str, stages: usize) -> String {
    vec![stage; stages].join(" | ")
}

fn check(cases: &[(String, &str, i32)]) -> Result<()> {
    for (program, expected, status) in cases {
        let (stdout, stderr, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, *status),
            "{program:.60}: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_long_pipes_of_computed_stages_answer_as_jq_does_5679() -> Result<()> {
    check(&[
        (format!("0 | {}", pipe(". + 1", 129)), "129\n", 0),
        (format!("0 | {}", pipe(". + 1", 200)), "200\n", 0),
        (format!("[1] | {}", pipe("map(.)", 200)), "[1]\n", 0),
        (
            format!("0 | {}", pipe(". + 1 | tostring | tonumber", 100)),
            "100\n",
            0,
        ),
        (
            format!("{{}} | {}", pipe(".a += 1", 150)),
            "{\"a\":150}\n",
            0,
        ),
        (
            format!("[[1]] | {} | [paths] | length", pipe("[.[0]]", 150)),
            "2\n",
            0,
        ),
    ])
}

#[test]
fn test_a_long_pipe_with_stages_that_fan_out_or_end_answers_as_jq_does_5679() -> Result<()> {
    let adds = pipe(". + 1", 150);
    check(&[
        (format!("0 | {adds} | select(. > 1000)"), "", 0),
        (format!("0 | {adds} | select(. > 100)"), "150\n", 0),
        (
            format!("0 | (., . + 1) | {}", pipe(". + 1", 200)),
            "200\n201\n",
            0,
        ),
        (
            format!("[limit(2; (1, 2, 3) | {})]", pipe(". + 1", 200)),
            "[201,202]\n",
            0,
        ),
        (
            format!("first((1, 2, 3) | {})", pipe(". + 1", 200)),
            "201\n",
            0,
        ),
        (
            format!(
                "[label $out | (1, 2, 3) | {adds} | if . == 152 then ., break $out else . end]"
            ),
            "[151,152]\n",
            0,
        ),
    ])
}

#[test]
fn test_an_error_in_a_long_pipe_is_raised_and_caught_as_jq_does_5679() -> Result<()> {
    let adds = pipe(". + 1", 150);
    let hundred = pipe(". + 1", 100);
    check(&[
        (
            format!("try (0 | {adds} | error(\"x\")) catch ."),
            "\"x\"\n",
            0,
        ),
        (
            format!("[0 | {hundred} | if . == 100 then error(\"mid\") else . end | {hundred}]"),
            "",
            5,
        ),
        (
            format!("[0 | {hundred} | if . == 100 then error(\"mid\") else . end | {hundred}]?"),
            "",
            0,
        ),
    ])
}

#[test]
fn test_a_long_pipe_with_a_stage_holding_a_def_still_answers_5679() -> Result<()> {
    // A call of a `def` carries evaluation state a kept chain would share between calls, so a pipe
    // with one is copied as before. (`(def f: . + 1; f)` as a stage holds none: it is inlined.)
    let adds = pipe(". + 1", 150);
    check(&[
        (format!("def f: . + 1; 0 | {}", pipe("f", 200)), "200\n", 0),
        (format!("def f: . + 1; 0 | {adds} | f | {adds}"), "301\n", 0),
        (
            format!("def f(x): x + 1; 0 | {}", pipe("f(.)", 200)),
            "200\n",
            0,
        ),
    ])
}

#[test]
fn test_a_chain_of_thousands_of_computed_stages_is_answered_5679() -> Result<()> {
    // Quadratic in memory before (750 stages took 99 MB, 3,000 took 1.4 GB); the length is the
    // most a revert costs, not what pins it.
    check(&[
        (format!("0 | {}", pipe(". + 1", 3_000)), "3000\n", 0),
        (format!("[1] | {}", pipe("map(.)", 3_000)), "[1]\n", 0),
    ])
}
