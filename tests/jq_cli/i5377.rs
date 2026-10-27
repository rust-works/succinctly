//! #5377: `last(f)` runs as the `reduce f as $x (null; $x)` jq defines it by unless `f` is
//! navigation alone, so it holds one value at a time: `last(range(30000000) * 2)` was killed at
//! 3 GB where jq stays at 2.5 MB. The rows are what jq 1.7.1 prints, for the computed generators
//! that take the `reduce` route, the navigations that keep the collecting one, and `last(f)` in
//! path position.

use anyhow::Result;

use crate::run_jq_full;

/// (input, filter, expected `-c` output, exit code).
const ROWS: &[(&str, &str, &str, i32)] = &[
    ("null", "last(range(5))", "4\n", 0),
    ("null", "last(range(0))", "null\n", 0),
    ("null", "last(range(0;10;3))", "9\n", 0),
    ("null", "last(range(5;0;-2))", "1\n", 0),
    ("null", "last(range(3) * 2)", "4\n", 0),
    ("null", "last(range(3) | select(. > 100))", "null\n", 0),
    ("null", "[last(range(3), 7)]", "[7]\n", 0),
    ("null", "last(range(2), error(\"x\"))", "", 5),
    (
        "null",
        "try last(range(3) | if . == 1 then error(\"e\") else . end) catch .",
        "\"e\"\n",
        0,
    ),
    (
        "null",
        "last(range(3) | if . == 1 then empty else . end)",
        "2\n",
        0,
    ),
    ("null", "last(0 | while(. < 5; . + 1))", "4\n", 0),
    ("null", "last(0 | until(. > 5; . + 1))", "6\n", 0),
    (
        "null",
        "last(0 | recurse(if . < 5 then . + 1 else empty end))",
        "5\n",
        0,
    ),
    ("null", "last(limit(3; repeat(1)))", "1\n", 0),
    ("null", "last(range(3)) as $x | $x * 2", "4\n", 0),
    ("null", "[limit(2; last(range(5)), 9, 10)]", "[4,9]\n", 0),
    (
        "null",
        "label $out | last(range(5) | if . == 3 then break $out else . end)",
        "",
        0,
    ),
    ("null", "last(range(2; 5) | . * .)", "16\n", 0),
    ("null", "last(range(2) | range(.))", "0\n", 0),
    ("null", "def g: range(5); last(g)", "4\n", 0),
    ("null", "last(\"abab\" | splits(\"a\"))", "\"b\"\n", 0),
    ("null", "last(1, 2, 3)", "3\n", 0),
    ("null", "last(empty, empty)", "null\n", 0),
    ("null", "last((1,2) as $x | $x * 10)", "20\n", 0),
    ("[1,2,3]", "last(.[] | range(2))", "1\n", 0),
    ("[1,2,3]", "last(.[])", "3\n", 0),
    ("{\"a\":[1,2,{\"x\":1}]}", "last(.a[])", "{\"x\":1}\n", 0),
    ("[[1,2],[3,4]]", "last(.[] | .[])", "4\n", 0),
    ("[1,2,3]", "[last(range(3)), last(.[])]", "[2,3]\n", 0),
    ("[1,2,3]", "last(.[] | select(. > 1))", "3\n", 0),
    ("[1,2,3]", "last(.[] | tostring)", "\"3\"\n", 0),
    ("[1,2,3]", "last(paths)", "[2]\n", 0),
    ("{\"a\":[1,2]}", "last(.. | numbers)", "2\n", 0),
];

/// `last(f)` in path position: a target that does more than navigate, which the
/// holdout refuses before reading input.
const PATH_ROWS: &[(&str, &str, &str, i32)] = &[
    ("[1,2,3]", "path(last(.[]))", "", 5),
    ("[1,2,3]", "path(last(range(3)))?", "", 0),
    ("[1,2,3]", "last(.[]) |= 9", "", 5),
];

fn assert_rows(rows: &[(&str, &str, &str, i32)]) -> Result<()> {
    for (input, filter, want, want_code) in rows {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, *want_code),
            "{input} | `{filter}`: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_last_of_a_generator_answers_as_jq_5377() -> Result<()> {
    assert_rows(ROWS)
}

#[cfg_attr(feature = "unshared-containers", ignore = "holdout refusal (#5246)")]
#[test]
fn test_last_of_a_generator_in_path_position_answers_as_jq_5377() -> Result<()> {
    assert_rows(PATH_ROWS)
}
