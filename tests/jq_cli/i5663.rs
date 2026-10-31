//! #5663: a long chain of field or index reads over a computed value is stepped without a
//! re-entry per stage, which held one copy of the rest of the pipe per stage (a 5,000-step `.a.a.a…`
//! took 1.4 GB). Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`); stderr wording is
//! not compared. The memory itself is pinned by
//! `test_a_chain_of_reads_over_an_owned_value_builds_no_rest_per_stage_5663` in `eval.rs`.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_short_chains_over_a_computed_value_answer_as_jq_does_5663() -> Result<()> {
    for (program, expected, status) in [
        ("{\"a\":{\"a\":{\"a\":3}}} | .a.a.a", "3\n", 0),
        ("{\"a\":{\"a\":{\"a\":3}}} | .a.a.a.a.a", "", 5),
        ("{} | .a.a.a | length", "0\n", 0),
        ("{\"a\":[{\"b\":[10,20]}]} | .a[0].b[1]", "20\n", 0),
        ("{\"a\":[{\"b\":[10,20]}]} | .a[0].b[1] | . + 1", "21\n", 0),
        ("[[[7]]] | .[0][0][0]", "7\n", 0),
        ("[[[7]]] | .[0][0][0][0]", "", 5),
        ("[[[7]]] | .[0][0][5]", "null\n", 0),
        ("{\"a\":{\"a\":1}} | .a.a.a", "", 5),
        ("{\"a\":{\"a\":1}} | .a.a.a?", "", 0),
        ("{\"a\":[1]} | .a.b", "", 5),
        ("1 | .a.a", "", 5),
        ("{\"a\":1} | .a.b.c.d", "", 5),
    ] {
        let (stdout, _stderr, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((stdout.as_str(), code), (expected, status), "{program}");
    }
    Ok(())
}

#[test]
fn test_a_chain_of_thousands_of_reads_is_answered_5663() -> Result<()> {
    // Quadratic in memory before (2,000 steps took 198 MB, 5,000 took 1.4 GB); the length is the
    // most a revert costs, not what pins it.
    let fields = ".a".repeat(5_000);
    let indexes = ".[0]".repeat(5_000);
    for (program, expected) in [
        (format!("{{}} | {fields}"), "null\n"),
        (format!("[] | {indexes}"), "null\n"),
        (format!("{{}} | {fields} | length"), "0\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", &program], None)?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{stderr}");
    }
    Ok(())
}
