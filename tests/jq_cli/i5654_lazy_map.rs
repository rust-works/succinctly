//! #5654: `arr | map(f)` and `obj | map(f)` pulled their elements through a cons list that ran no
//! gap check at all, so a junk byte, a doubled, missing or trailing `,` and a missing `:` around
//! them were read as whitespace and `map` answered. jq 1.7.1 refuses each document as a parse
//! error whatever the filter. Documents and messages were captured from jq 1.7.1 (`/usr/bin/jq`).
//! An object is checked whole, by the probe that finds a repeated key. An array is checked as it is
//! pulled, so a consumer that stops early (`first(.[])`, `map(.) | first`) never sees a bad gap after
//! its answer, which is the semi-index's lazy contract, pinned below as the divergence it is.

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
    // A repeated key takes the collapsing branch, which reads the members another way.
    (
        "{\"a\":1,\"a\":2@}",
        "jq: parse error: Invalid numeric literal at line 1, column 14\n",
    ),
    (
        "{\"a\":1,\"a\":2 \"b\":3}",
        "jq: parse error: Expected separator between values at line 1, column 16\n",
    ),
    (
        "{\"a\":1,\"b\":2,\"a\":3,}",
        "jq: parse error: Expected another key-value pair at line 1, column 20\n",
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
fn test_an_object_is_checked_whole_even_for_a_consumer_that_stops_early_5654() -> Result<()> {
    for (document, stderr) in &REFUSED[6..] {
        let (out, err, status) = run_jq_full(&["-c", "map(.) | first"], Some(document))?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            ("", *stderr, 5),
            "{document:?}"
        );
    }
    Ok(())
}

#[test]
fn test_an_array_consumer_that_stops_early_never_pulls_the_bad_gap_5654() -> Result<()> {
    for (document, filter, stdout) in [
        ("[1,2@]", "map(.) | first", "1\n"),
        // A gap inside a child that `f` never reads is that child's own business.
        ("[1,[2,3,],4]", "map(.) | length", "3\n"),
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

#[test]
fn test_a_body_error_on_an_earlier_element_comes_before_the_parse_error_5654() -> Result<()> {
    let (out, err, status) = run_jq_full(&["-c", "map(.+1)"], Some("[[1],[2]@]"))?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        (
            "",
            "jq: error (at <stdin>:0): array ([1]) and number (1) cannot be added\n",
            5
        )
    );
    Ok(())
}
