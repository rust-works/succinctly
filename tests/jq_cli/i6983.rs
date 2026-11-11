//! #6983: `limit($n; f)` and `nth($n; f)` compare their count with `0` as jq does. jq's `$n > 0`,
//! `$n == 0` and `$n < 0` are comparisons between two number literals, which jq makes exactly, so a
//! literal too small for a double (`1e-1000`, `-1e-400`) is above or below `0` although its double is
//! `±0.0`. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// `(input, filter, stdout)`, with `None` for the input where the filter runs with `-n`.
const ROWS: &[(Option<&str>, &str, &str)] = &[
    // A program literal, and the fast paths that read one as a constant count.
    (None, "[limit(1e-1000; 1, 2)]", "[1]\n"),
    (None, "[limit((1e-1000); 1, 2)]", "[1]\n"),
    (None, "[limit(1e-1000 | .; 1, 2)]", "[1]\n"),
    (None, "[limit(1e-1000, 0, 2; 1, 2, 3)]", "[1,1,2]\n"),
    (None, "first(limit(1e-1000; 1, 2))", "1\n"),
    (None, "isempty(limit(1e-1000; 1))", "false\n"),
    (None, "reduce limit(1e-1000; 1, 2) as $x (0; . + $x)", "1\n"),
    (
        None,
        "[foreach limit(1e-1000; 1, 2) as $x (0; . + 1)]",
        "[1]\n",
    ),
    (None, "nth(1e-1000; 1, 2)", "1\n"),
    // A computed count is a double, compared as one: `1e-1000 * 1` and `-(1e-400)` are zero.
    (None, "[limit(1e-1000 * 1; 1, 2)]", "[]\n"),
    (None, "[limit(-(1e-400); 1, 2)]", "[]\n"),
    (None, "[limit(1e-1000; empty)]", "[]\n"),
    (None, "[limit(0e5; 1, 2)]", "[]\n"),
    // `tonumber` makes a literal.
    (None, "[limit(\"1e-1000\" | tonumber; 1, 2)]", "[1]\n"),
    (None, "[limit(\"-1e-400\" | tonumber; 1, 2)]", "[1,2]\n"),
    // A document literal, read directly, through a binding, a field and an index.
    (Some("1e-1000"), "[limit(.; 1, 2, 3)]", "[1]\n"),
    (Some("1e-400"), "[limit(.; 1, 2, 3)]", "[1]\n"),
    (Some("-1e-400"), "[limit(.; 1, 2, 3)]", "[1,2,3]\n"),
    (Some("-1e-400"), ". as $n | [limit($n; 1, 2)]", "[1,2]\n"),
    (Some("{\"n\":-1e-400}"), "[limit(.n; 1, 2)]", "[1,2]\n"),
    (Some("[1e-1000]"), "[limit(.[0]; 1, 2)]", "[1]\n"),
    (Some("1e-1000"), "[nth(.; 1, 2, 3)]", "[1]\n"),
    // Controls: a literal that is zero, and a subnormal a double holds.
    (Some("0.000e9"), "[limit(.; 1, 2, 3)]", "[]\n"),
    (Some("-0e-3"), "[limit(.; 1, 2, 3)]", "[]\n"),
    (Some("2.5e-324"), "[limit(.; 1, 2, 3)]", "[1]\n"),
    (Some("-2.5e-324"), "[limit(.; 1, 2, 3)]", "[1,2,3]\n"),
    (Some("0e0"), "[nth(.; 1, 2, 3)]", "[1]\n"),
];

#[test]
fn limit_and_nth_compare_a_count_literal_with_zero_exactly_6983() -> Result<()> {
    for &(input, filter, want) in ROWS {
        let args: Vec<&str> = match input {
            None => vec!["-nc", filter],
            Some(_) => vec!["-c", filter],
        };
        let (stdout, stderr, code) = run_jq_full(&args, input)?;
        assert_eq!(
            (stdout.as_str(), code),
            (want, 0),
            "{input:?} | {filter}: {stderr}"
        );
    }
    Ok(())
}

/// `nth` raises for a negative count, and a literal below `0` whose double is `-0.0` is one.
#[test]
fn nth_refuses_a_negative_count_literal_whose_double_is_zero_6983() -> Result<()> {
    for (input, filter, args) in [
        (Some("-1e-400"), "[nth(.; 1, 2, 3)]", vec!["-c"]),
        (None, "[nth(\"-1e-400\" | tonumber; 1, 2)]", vec!["-nc"]),
    ] {
        let mut args = args;
        args.push(filter);
        let (stdout, stderr, code) = run_jq_full(&args, input)?;
        assert_eq!(stdout, "", "{filter}");
        assert_eq!(code, 5, "{filter}");
        assert!(
            stderr.contains("nth doesn't support negative indices"),
            "{filter}: {stderr}"
        );
    }
    Ok(())
}
