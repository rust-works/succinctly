//! #5722: a pipe of more than 128 stages with a stage holding evaluation state (a call of a user
//! `def`) links the runs of stateless blocks around it and clones only the blocks that hold one, where
//! it copied every stage after each call. These pin the answers for pipes that mix the two, wherever
//! the def calls fall; what each call hands on is pinned by
//! `test_a_long_pipe_links_around_a_stateful_block_5722` in `expr.rs`. Expectations were captured from
//! jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

fn adds(n: usize) -> String {
    vec![". + 1"; n].join(" | ")
}

fn check(cases: &[(String, &str)]) -> Result<()> {
    for (program, expected) in cases {
        let (stdout, stderr, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{program:.80}: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn a_long_pipe_with_def_calls_in_some_blocks_answers_as_jq_does_5722() -> Result<()> {
    let (a, b) = (adds(100), adds(200));
    check(&[
        (format!("def f: . + 1; 0 | {a} | f | {b}"), "301\n"),
        (format!("def f: . + 1; 0 | f | {b}"), "201\n"),
        (format!("def f: . + 1; 0 | {b} | f"), "201\n"),
        (
            format!("def f: . + 1; 0 | {a} | f | {a} | f | {a} | f | {a}"),
            "403\n",
        ),
        (format!("def f(x): x + 1; 0 | {a} | f(.) | {b}"), "301\n"),
        (
            format!("def f: if . < 5 then (. + 1 | f) else . end; 0 | {a} | f | {b}"),
            "300\n",
        ),
        (
            format!("def f: . + 1; [0, 10] | map(. | {a} | f | {b})"),
            "[301,311]\n",
        ),
        (
            format!("def f: . + 1; [limit(2; (1, 2, 3) | {a} | f | {b})]"),
            "[302,303]\n",
        ),
        (
            format!("def f: (., . + 1); [0 | {a} | f | {a}]"),
            "[200,201]\n",
        ),
        (
            format!("def f: . + 1; 0 | {a} | f | {a} | f | . * 2"),
            "404\n",
        ),
    ])
}

#[test]
fn a_long_pipe_with_a_def_call_in_every_block_still_answers_5722() -> Result<()> {
    let stage = format!("{} | f", adds(15));
    let pipe = vec![stage.as_str(); 40].join(" | ");
    check(&[(format!("def f: . + 1; 0 | {pipe}"), "640\n")])
}
