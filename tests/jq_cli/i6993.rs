//! #6993: `flatten($depth)` and `range($from; $upto; $by)` compare their argument with `0` as jq's
//! own definitions do. jq writes both in jq (`if $x < 0 then error(...) else _flatten($x) end`,
//! `$x != 0` per level; `if $by > 0 then $from | while(. < $upto; . + $by) elif $by < 0 ...`), and two
//! number literals compare exactly as decimals there, so a literal too small for a double
//! (`1e-1000`, `-1e-400`) is not zero, and `range`'s first `$from < $upto` can tell apart two
//! literals whose doubles tie. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// `(input, filter, stdout)`, with `None` for the input where the filter runs with `-n`.
const ROWS: &[(Option<&str>, &str, &str)] = &[
    (
        Some("{\"p\":1e-1000}"),
        ".p as $n | [[[1]]] | flatten($n)",
        "[1]\n",
    ),
    (
        Some("{\"p\":1e-1000}"),
        ".p as $n | [[[[1]]], 2] | flatten($n)",
        "[1,2]\n",
    ),
    (None, "[[[[1]]]] | flatten(1e-1000)", "[1]\n"),
    (None, "[[[[1]]]] | flatten(0e5)", "[[[[1]]]]\n"),
    (None, "[[[[1]]]] | flatten(-1e-400)", "[[[[1]]]]\n"),
    (None, "[[[[1]]]] | flatten(1.5)", "[1]\n"),
    (None, "[[[[1]]]] | flatten(2)", "[[1]]\n"),
    (None, "[[[[1]]]] | flatten(1e-1000 * 1)", "[[[[1]]]]\n"),
    (
        Some("{\"p\":1e-1000}"),
        ".p as $n | [limit(3; range(0; 1; $n))]",
        "[0,0,0]\n",
    ),
    (
        Some("{\"p\":-1e-400}"),
        ".p as $n | [limit(3; range(1; 0; $n))]",
        "[1,1,1]\n",
    ),
    (
        Some("{\"p\":-1e-400}"),
        ".p as $n | [limit(3; range(0; 1; $n))]",
        "[]\n",
    ),
    (None, "[limit(3; range(0; 1; 1e-1000))]", "[0,0,0]\n"),
    (None, "first(range(5; 6; 1e-1000))", "5\n"),
    (None, "[range(0; 1e-1000; 1)]", "[0]\n"),
    (None, "[range(1e-1000; 0; -1)]", "[1E-1000]\n"),
    (None, "[range(-1e-400; 0; 1)]", "[]\n"),
    (None, "[range(0; -1e-400; -1)]", "[]\n"),
    (None, "[range(0.1; 0.10000000000000000001; 1)]", "[0.1]\n"),
    (
        None,
        "[range(0.10000000000000000001; 0.1; -1)]",
        "[0.10000000000000000001]\n",
    ),
    (None, "[range(0.1; 0.1; 1)]", "[]\n"),
    (None, "[range(0; 1e-1000)]", "[]\n"),
    (None, "[range(0.1; 0.10000000000000000001)]", "[]\n"),
    (None, "[range(1e-1000)]", "[]\n"),
    (None, "[range(0; 3; 1)]", "[0,1,2]\n"),
    (None, "[range(0; 1; 0.25)]", "[0,0.25,0.5,0.75]\n"),
    (None, "[range(3; 0; -1)]", "[3,2,1]\n"),
    (None, "[range(0; 3; 0)]", "[]\n"),
    (None, "[range(0; 3; 1e-1000 * 1)]", "[]\n"),
    (None, "[range(0; 1e-1000 * 1; 1)]", "[]\n"),
    (
        None,
        "[limit(2; range(0; 1; \"1e-1000\" | tonumber))]",
        "[0,0]\n",
    ),
    (None, "[range(0; 1e-1000; 1)] | .[0] | . == 0", "true\n"),
    (None, "[range(1e-1000; 2; 1)]", "[1E-1000,1]\n"),
];

/// `(input, filter, message)`: jq raises, and so does `succinctly jq`, with jq's message.
const ERRORS: &[(Option<&str>, &str, &str)] = &[
    (
        Some("{\"n\":-1e-400}"),
        ".n as $n | [[[1]]] | flatten($n)",
        "flatten depth must not be negative",
    ),
    (
        None,
        "[[[[1]]]] | flatten(\"-1e-400\" | tonumber)",
        "flatten depth must not be negative",
    ),
    (
        None,
        "[1, 2] | flatten(\"-1e-400\" | tonumber)",
        "flatten depth must not be negative",
    ),
    (
        None,
        "[[[[1]]]] | flatten(-0.5)",
        "flatten depth must not be negative",
    ),
];

fn args_for(input: Option<&str>, filter: &str) -> Vec<String> {
    let flag = if input.is_some() { "-c" } else { "-nc" };
    vec![flag.to_string(), filter.to_string()]
}

#[test]
fn flatten_and_range_compare_a_literal_with_zero_exactly_6993() -> Result<()> {
    for &(input, filter, want) in ROWS {
        let args = args_for(input, filter);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (stdout, stderr, code) = run_jq_full(&args, input)?;
        assert_eq!(
            (stdout.as_str(), code),
            (want, 0),
            "{input:?} | {filter}: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn flatten_refuses_a_negative_depth_literal_whose_double_is_zero_6993() -> Result<()> {
    for &(input, filter, message) in ERRORS {
        let args = args_for(input, filter);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (stdout, stderr, code) = run_jq_full(&args, input)?;
        assert_eq!((stdout.as_str(), code), ("", 5), "{input:?} | {filter}");
        assert!(stderr.contains(message), "{filter}: {stderr}");
    }
    Ok(())
}
