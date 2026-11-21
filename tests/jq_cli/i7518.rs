//! #7518: `last(f)` of an `f` that yields nothing yields nothing, as `first(f)` does. jq 1.7.1's
//! `def last(g): reduce g as $item (null; $item)` answered `null` for it (`[last(empty)]` was
//! `[null]`); jq 1.8 compiles `last(g)` itself and backtracks when `g` produced no value
//! (jqlang/jq#3179, listed in the 1.8 NEWS under changes to existing functions). `succinctly jq`
//! follows 1.8.2, and the difference from the pinned 1.7.1 is recorded in the limitations page and
//! as the `syntax:man-last-empty` ledger entry. Every row below was captured from jq 1.8.2
//! (`/opt/homebrew/bin/jq -c`), and every filter reads its input from stdin.

use anyhow::Result;

use crate::run_jq_full;

/// (input, filter, expected `-c` output, exit code).
const ROWS: &[(&str, &str, &str, i32)] = &[
    ("null", "[last(empty)]", "[]\n", 0),
    ("null", "[last(range(0))]", "[]\n", 0),
    ("null", "[last(1,empty)]", "[1]\n", 0),
    ("null", "[last(empty,1)]", "[1]\n", 0),
    ("null", "[last(empty,empty)]", "[]\n", 0),
    ("null", "[last(empty)] == [first(empty)]", "true\n", 0),
    ("null", "[last(select(false))]", "[]\n", 0),
    ("null", "[last(range(3) | select(. > 100))]", "[]\n", 0),
    ("null", "[last(limit(0; 1,2))]", "[]\n", 0),
    ("null", "[last(first(empty))]", "[]\n", 0),
    ("null", "[first(last(empty))]", "[]\n", 0),
    ("null", "[last(last(empty))]", "[]\n", 0),
    ("null", "def f: empty; [last(f)]", "[]\n", 0),
    ("null", "[last(empty)?]", "[]\n", 0),
    ("null", "[last(error(\"x\"))?]", "[]\n", 0),
    ("null", "last(error(\"x\"))", "", 5),
    ("null", "last(1, error(\"x\"))", "", 5),
    ("null", "[last(null)]", "[null]\n", 0),
    ("null", "[last(false)]", "[false]\n", 0),
    ("null", "[last(1, 2)]", "[2]\n", 0),
    ("null", "[last(empty) | 1]", "[]\n", 0),
    ("null", "[last(empty) // 7]", "[7]\n", 0),
    ("null", "[isempty(last(empty))]", "[true]\n", 0),
    ("null", "reduce last(empty) as $x (0; . + 1)", "0\n", 0),
    ("null", "[foreach last(empty) as $x (0; . + 1)]", "[]\n", 0),
    ("null", "[limit(3; last(empty), 1)]", "[1]\n", 0),
    ("null", "[range(3) | last(empty)]", "[]\n", 0),
    ("null", "[last(inputs)]", "[]\n", 0),
    ("null", "[[] | last]", "[null]\n", 0),
    ("null", "[[] | first]", "[null]\n", 0),
    ("[]", "last(.[])", "", 0),
    ("[]", "[last(.[])]", "[]\n", 0),
    ("[]", "[last(.[]?)]", "[]\n", 0),
    ("[1,2]", "[last(.[])]", "[2]\n", 0),
    ("{\"a\":[]}", "[last(.a[])]", "[]\n", 0),
    ("{\"a\":[]}", "last(.a[]) = 5", "{\"a\":[]}\n", 0),
    ("{\"a\":[]}", "last(.a[]) |= 5", "{\"a\":[]}\n", 0),
    ("{\"a\":[]}", "del(last(.a[]))", "{\"a\":[]}\n", 0),
    ("{\"a\":[]}", "[path(last(.a[]))]", "[]\n", 0),
    ("{\"a\":[]}", "[path(last(empty))]", "[]\n", 0),
    ("{\"a\":[1,2]}", "[path(last(.a[]))]", "", 5),
    ("[[],[1]]", "[.[] | last(.[])]", "[1]\n", 0),
    ("[[],[1]]", "map([last(.[])])", "[[],[1]]\n", 0),
    ("[]", "[last(.[])] | length", "0\n", 0),
    ("[3]", "[last(range(.[0]) | select(. > 5))]", "[]\n", 0),
    ("\"abc\"", "[last(splits(\"x\"))]", "[\"abc\"]\n", 0),
    ("\"abc\"", "[last(empty)]", "[]\n", 0),
];

#[test]
fn last_of_a_generator_that_yields_nothing_yields_nothing_7518() -> Result<()> {
    for (input, filter, expected, status) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(&stdout, expected, "{filter} on {input}: stderr: {stderr}");
        assert_eq!(code, *status, "{filter} on {input}: stderr: {stderr}");
    }
    Ok(())
}

/// Several documents go through the per-document loop, `-s` through one slurped array and `-n`
/// with `inputs` through the input queue.
#[test]
fn last_of_nothing_over_several_documents_7518() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", "[last(.[])]"], Some("[]\n[1,2]\n[]\n"))?;
    assert_eq!((stdout.as_str(), code), ("[]\n[2]\n[]\n", 0));

    let (stdout, _, code) = run_jq_full(&["-c", "-s", "last(.[])"], Some("1\n2\n"))?;
    assert_eq!((stdout.as_str(), code), ("2\n", 0));

    let (stdout, _, code) =
        run_jq_full(&["-c", "-s", "[last(.[] | select(. > 5))]"], Some("1\n2\n"))?;
    assert_eq!((stdout.as_str(), code), ("[]\n", 0));

    let (stdout, _, code) = run_jq_full(&["-n", "-c", "[last(inputs)]"], Some("1\n2\n3\n"))?;
    assert_eq!((stdout.as_str(), code), ("[3]\n", 0));

    let (stdout, _, code) = run_jq_full(
        &["-n", "-c", "[last(inputs | select(. > 5))]"],
        Some("1\n2\n"),
    )?;
    assert_eq!((stdout.as_str(), code), ("[]\n", 0));
    Ok(())
}
