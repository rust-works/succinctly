//! #5445: a sequence of mapping key nodes (`[.[] | key]`) keeps each key's quoting and its `~`
//! spelling, as yq prints them. Every output is captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

fn check(input: &str, rows: &[(&str, &str)]) -> Result<()> {
    for (filter, want) in rows {
        let (stdout, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!((stdout.as_str(), code), (*want, 0), "`{filter}`");
    }
    Ok(())
}

const K: &str = "\"qk\": 1\nk2: 2\n~: 3\n'sq': 4\n1: 5\ntrue: 6\n\"2\": 7\n";

const M: &str = "a: &anc # keyc\n  b: 1\nx:\n  - 1 # e0\n  - 2\n\"qk\": 1\nk2: |\n  block\n";

#[test]
fn test_key_nodes_in_a_sequence_k_5445() -> Result<()> {
    check(
        K,
        &[
            (
                "[.[] | key]",
                "- \"qk\"\n- k2\n- ~\n- 'sq'\n- 1\n- true\n- \"2\"\n",
            ),
            (
                "[.. | key]",
                "- \"qk\"\n- k2\n- ~\n- 'sq'\n- 1\n- true\n- \"2\"\n",
            ),
            (
                "[.[] | key | .]",
                "- \"qk\"\n- k2\n- ~\n- 'sq'\n- 1\n- true\n- \"2\"\n",
            ),
            ("[.[] | key] | .[0]", "qk\n"),
        ],
    )
}

#[test]
fn test_key_nodes_in_a_sequence_m_5445() -> Result<()> {
    check(
        M,
        &[
            ("[.[] | key]", "- a\n- x\n- \"qk\"\n- k2\n"),
            ("[.[] | key | .]", "- a\n- x\n- \"qk\"\n- k2\n"),
            ("[.[] | key | tostring]", "- a\n- x\n- qk\n- k2\n"),
        ],
    )
}
