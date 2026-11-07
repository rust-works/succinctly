//! #6527: `add`, `min` and `max` of an array that an earlier stage computed are folded in place
//! instead of through the reindex bridge. The fold must answer as the bridge (and jq) did, so the
//! rows below, captured from jq 1.7.1 (`/usr/bin/jq`), pin the answers for ties between `1` and
//! `1.0`, `-0`, integers past 2^53, floats at both ends of the range, an infinity (which the fold
//! leaves to the bridge), an empty array and a mixed array (which raises). The unit tests
//! `owned_number_fold_agrees_with_the_reindex_bridge_6527` and
//! `owned_number_fold_declines_what_it_cannot_answer_identically_6527` pin the door itself.

use anyhow::Result;

use crate::run_jq_full;

/// `(stdin, filter, stdout, exit status)` under `jq -c`.
const ROWS: &[(&str, &str, &str, i32)] = &[
    ("[1,2,3]", "map(. * 2) | add", "12\n", 0),
    ("[1,2,3]", "map(. * 2) | max", "6\n", 0),
    ("[1.5,2.5,-0.0]", "map(. * 2) | min", "-0\n", 0),
    ("[0.1,0.2,0.3]", "map(. + 0.1) | add", "0.9\n", 0),
    ("[1,1.0,1.00]", "map(. * 1) | max", "1\n", 0),
    ("[1,1.0,1.00]", "map(. * 1) | min", "1\n", 0),
    ("[0,-0]", "map(. * 1) | max", "-0\n", 0),
    ("[-0,0]", "map(. * 1) | min", "-0\n", 0),
    (
        "[9007199254740993,1]",
        "map(. * 1) | add",
        "9007199254740992\n",
        0,
    ),
    (
        "[1e308,1e308]",
        "map(. * 2) | add",
        "1.7976931348623157e+308\n",
        0,
    ),
    ("[1e308,-1e308]", "map(. * 1) | add", "0\n", 0),
    (
        "[1e1000,1]",
        "map(. * 1) | max",
        "1.7976931348623157e+308\n",
        0,
    ),
    ("[1e1000,-1e1000]", "map(. * 1) | add", "null\n", 0),
    ("[]", "map(. * 2) | add", "null\n", 0),
    ("[]", "map(. * 2) | max", "null\n", 0),
    ("[3,1,2]", "[.[] | . * 2] | add", "12\n", 0),
    ("[3,1,2]", "[.[] | . * 2] | min", "2\n", 0),
    ("[1,\"a\"]", "map(. * 1) | add", "", 5),
    ("[null,1]", "map(. + 0) | max", "1\n", 0),
    (
        "{\"a\":[1,2.5],\"b\":[3]}",
        "map(map(. * 2) | add)",
        "[7,6]\n",
        0,
    ),
    ("[3,1,2]", "map(. * 2) | add | . + 1", "13\n", 0),
    ("[3,1,2]", "map(. * 2) | [add, min, max]", "[12,2,6]\n", 0),
];

/// `(args, stdin, stdout, exit status)`: other ways in (`-s`, `-n`), a bound node in scope (the
/// door stands down for the embed table), nesting, a `?` and a `try`, and floats around the
/// printer's exponent switch points.
const OTHER_ROWS: &[(&[&str], &str, &str, i32)] = &[
    (&["-c", "-s", "map(. * 2) | add"], "1 2 3.5", "13\n", 0),
    (&["-c", "-s", "map(. * 2) | max"], "1 2 3.5", "7\n", 0),
    (&["-c", "-n", "[1,2.5] | map(. * 2) | min"], "", "2\n", 0),
    (
        &[
            "-c",
            ". as $x | map(. * 2) | [add, max, min, ($x | length)]",
        ],
        "[3,1,2.5]",
        "[13,6,2,3]\n",
        0,
    ),
    (
        &[
            "-c",
            ". as $x | ([$x[0]] | map(. * 2) | add) as $s | [$s, $x]",
        ],
        "[3,1]",
        "[6,[3,1]]\n",
        0,
    ),
    (
        &["-c", "[.[] | map(. * 2) | add]"],
        "[[1,2],[3.5],[0.1,0.2]]",
        "[6,7,0.6000000000000001]\n",
        0,
    ),
    (
        &["-c", ". | (. | map(. * 2) | (. | add))"],
        "[1,2.5]",
        "7\n",
        0,
    ),
    (
        &["-c", "map(. * 1) | [add, min, max]"],
        "[1e-5,1e-4,1e16,1e17,1e19]",
        "[1.011e+19,1e-05,1e+19]\n",
        0,
    ),
    (&["-c", "map(. * 1) | add"], "[1e17,1e17]", "2e+17\n", 0),
    (&["-c", "map(. * 1) | max"], "[1e19,1e18]", "1e+19\n", 0),
    (
        &["-c", "map(. / 3) | [add, min, max]"],
        "[1e-5,1e16,1]",
        "[3333333333333334,3.3333333333333337e-06,3333333333333333.5]\n",
        0,
    ),
    (&["-c", "map(. * 2) | add?"], "[1,2]", "6\n", 0),
    (&["-c", "try (map(. * 2) | add) catch ."], "[1,2]", "6\n", 0),
];

#[test]
fn test_other_routes_into_the_fold_match_jq_6527() -> Result<()> {
    for (args, stdin, stdout, code) in OTHER_ROWS {
        let (out, _err, status) = run_jq_full(args, Some(stdin))?;
        assert_eq!(
            (out.as_str(), status),
            (*stdout, *code),
            "{args:?} on {stdin:?}"
        );
    }
    Ok(())
}

#[test]
fn test_add_min_max_of_a_computed_number_array_match_jq_6527() -> Result<()> {
    for (stdin, filter, stdout, code) in ROWS {
        let args = ["-c", filter];
        let (out, _err, status) = run_jq_full(&args, Some(stdin))?;
        assert_eq!(
            (out.as_str(), status),
            (*stdout, *code),
            "{filter:?} on {stdin:?}"
        );
    }
    Ok(())
}
