//! #5450: `map(key)` hands back the mapping's key nodes, so each key keeps the quoting, `~`
//! spelling and anchor it was written with, as `[.[] | key]` does since #5445. Every output is
//! captured from yq v4.53.3.

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

#[test]
fn test_map_key_keeps_each_key_spelling_5450() -> Result<()> {
    let keys = "- \"qk\"\n- k2\n- ~\n- 'sq'\n- 1\n- true\n- \"2\"\n";
    check(
        K,
        &[
            ("map(key)", keys),
            (". | map(key)", keys),
            // The same sequence `[.[] | key]` gives (#5445).
            ("[.[] | key]", keys),
        ],
    )
}

#[test]
fn test_map_key_keeps_a_key_anchor_5450() -> Result<()> {
    check(
        "&ka key: 1\n\"b\": 2\n",
        &[("map(key)", "- &ka key\n- \"b\"\n")],
    )?;
    check(
        "a: &anc # keyc\n  b: 1\nx:\n  - 1 # e0\n  - 2\n\"qk\": 1\nk2: |\n  block\n",
        &[("map(key)", "- a\n- x\n- \"qk\"\n- k2\n")],
    )
}

/// What `map(key)` did before and still does: a value read off the sequence, a sequence's
/// indexes, an empty mapping, and a scalar or `null` that `map` never iterates.
#[test]
fn test_map_key_other_inputs_5450() -> Result<()> {
    check(
        "\"a\": 1\n",
        &[("map(key) | .[0]", "a\n"), ("map(key) | length", "1\n")],
    )?;
    check("- a\n- b\n", &[("map(key)", "- 0\n- 1\n")])?;
    check("{}\n", &[("map(key)", "{}\n")])?;
    check("[]\n", &[("map(key)", "[]\n")])?;
    check("5\n", &[("map(key)", "5\n")])?;
    check("null\n", &[("map(key)", "[]\n")])
}

/// A repeated key is listed once by `map(key)`, in yq and here.
#[test]
fn test_map_key_repeated_key_5450() -> Result<()> {
    check("a: 1\na: 2\n", &[("map(key)", "- a\n")])
}
