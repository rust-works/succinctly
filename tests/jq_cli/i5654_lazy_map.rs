//! #5654: `arr | map(f)` and `obj | map(f)` pulled their elements through a cons list that ran no
//! gap check at all, so a junk byte, a doubled, missing or trailing `,` and a missing `:` around
//! them were read as whitespace and `map` answered. jq 1.7.1 refuses each document as a parse
//! error whatever the filter. Documents and messages were captured from jq 1.7.1 (`/usr/bin/jq`).
//! A consumer that stops early (`first(.[])`, `map(.) | first`) never pulls the bad gap, which is
//! the semi-index's lazy contract (ADR-0012), pinned below as the divergence it is.

use anyhow::Result;

use crate::run_jq_full;

/// `(document, stderr)`; every row exits 5 with nothing on stdout.
const REFUSED: &[(&str, &str)] = &[
    (
        "[1@,2]",
        "jq: parse error: Invalid numeric literal at line 1, column 4\n",
    ),
    (
        "[1,,2]",
        "jq: parse error: Expected value before ',' at line 1, column 4\n",
    ),
    (
        "[1,2,]",
        "jq: parse error: Expected another array element at line 1, column 6\n",
    ),
    (
        "[1 2]",
        "jq: parse error: Expected separator between values at line 1, column 5\n",
    ),
    (
        "[1,2@]",
        "jq: parse error: Invalid numeric literal at line 1, column 6\n",
    ),
    (
        "[[1],[2]@]",
        "jq: parse error: Invalid numeric literal at line 1, column 10\n",
    ),
    (
        "{\"a\":1@,\"b\":2}",
        "jq: parse error: Invalid numeric literal at line 1, column 8\n",
    ),
    (
        "{\"a\":1,,\"b\":2}",
        "jq: parse error: Expected value before ',' at line 1, column 8\n",
    ),
    (
        "{\"a\":1,\"b\":2,}",
        "jq: parse error: Expected another key-value pair at line 1, column 14\n",
    ),
    (
        "{\"a\":1,\"b\"}",
        "jq: parse error: Objects must consist of key:value pairs at line 1, column 11\n",
    ),
    (
        "{\"a\":1 \"b\":2}",
        "jq: parse error: Expected separator between values at line 1, column 10\n",
    ),
];

/// Whitespace in every gap is still whitespace, and an empty container has nothing to check.
const ACCEPTED: &[(&str, &str)] = &[
    ("[1 , 2 ,\n3 ]", "[1,2,3]\n"),
    ("[[1] , [2] ]", "[[1],[2]]\n"),
    ("{\"a\" : 1 , \"b\":2 \n}", "[1,2]\n"),
    ("[]", "[]\n"),
    ("{}", "[]\n"),
];

#[test]
fn test_map_refuses_a_bad_gap_it_pulls_past_5654() -> Result<()> {
    for (document, stderr) in REFUSED {
        for filter in ["map(.)", "map(.) | length", "map(tojson)"] {
            let (out, err, status) = run_jq_full(&["-c", filter], Some(document))?;
            assert_eq!(
                (out.as_str(), err.as_str(), status),
                ("", *stderr, 5),
                "{filter} over {document:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn test_map_still_answers_over_whitespace_gaps_5654() -> Result<()> {
    for (document, stdout) in ACCEPTED {
        let (out, err, status) = run_jq_full(&["-c", "map(.)"], Some(document))?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (*stdout, "", 0),
            "{document:?}"
        );
    }
    Ok(())
}

#[test]
fn test_a_consumer_that_stops_early_never_pulls_the_bad_gap_5654() -> Result<()> {
    for (document, filter, stdout) in [
        ("[1@,2]", "map(.) | first", "1\n"),
        ("[1,2@]", "map(.) | first", "1\n"),
        ("{\"a\":1@,\"b\":2}", "map(.) | first", "1\n"),
    ] {
        let (out, err, status) = run_jq_full(&["-c", filter], Some(document))?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (stdout, "", 0),
            "{filter} over {document:?}"
        );
    }
    Ok(())
}
