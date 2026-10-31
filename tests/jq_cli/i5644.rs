//! #5644: a byte that begins no JSON token (`@`, `#`, ...) right after `[`, `{` or a key's closing
//! quote was read as whitespace, so `[@1]` parsed as `[1]` and `{"a":@-3}` as `{"a":-3}`. jq 1.7.1
//! refuses each as `Invalid numeric literal` at the column of the first byte it cannot place. Rows
//! and messages were captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(document, jq's stderr)`; every row exits 5 with nothing on stdout.
const REFUSED: &[(&str, &str)] = &[
    (
        "[@1]",
        "jq: parse error: Invalid numeric literal at line 1, column 4\n",
    ),
    (
        "[@-3]",
        "jq: parse error: Invalid numeric literal at line 1, column 5\n",
    ),
    (
        "[@ -3]",
        "jq: parse error: Invalid numeric literal at line 1, column 3\n",
    ),
    (
        "[@3]",
        "jq: parse error: Invalid numeric literal at line 1, column 4\n",
    ),
    (
        "[#1]",
        "jq: parse error: Invalid numeric literal at line 1, column 4\n",
    ),
    (
        "[@\"a\"]",
        "jq: parse error: Invalid numeric literal at line 1, column 3\n",
    ),
    (
        "[@true]",
        "jq: parse error: Invalid numeric literal at line 1, column 7\n",
    ),
    (
        "[@[1]]",
        "jq: parse error: Invalid numeric literal at line 1, column 3\n",
    ),
    (
        "[@{}]",
        "jq: parse error: Invalid numeric literal at line 1, column 3\n",
    ),
    (
        "[1,2,@3]",
        "jq: parse error: Invalid numeric literal at line 1, column 8\n",
    ),
    (
        "[[@1]]",
        "jq: parse error: Invalid numeric literal at line 1, column 5\n",
    ),
    (
        "{\"a\":@1}",
        "jq: parse error: Invalid numeric literal at line 1, column 8\n",
    ),
    (
        "{\"a\":@-3}",
        "jq: parse error: Invalid numeric literal at line 1, column 9\n",
    ),
    (
        "{\"a\":@\"x\"}",
        "jq: parse error: Invalid numeric literal at line 1, column 7\n",
    ),
    (
        "{@\"a\":1}",
        "jq: parse error: Invalid numeric literal at line 1, column 3\n",
    ),
    (
        "{\"a\"@:1}",
        "jq: parse error: Invalid numeric literal at line 1, column 6\n",
    ),
    (
        "{\"a\":1,@\"b\":2}",
        "jq: parse error: Invalid numeric literal at line 1, column 9\n",
    ),
    (
        "{\"a\":[@1]}",
        "jq: parse error: Invalid numeric literal at line 1, column 9\n",
    ),
    (
        "[{\"a\":@1}]",
        "jq: parse error: Invalid numeric literal at line 1, column 9\n",
    ),
];

/// Whitespace in every gap stays accepted, and a form feed, which JSON does not count as
/// whitespace, is refused where the gap check sees it.
#[test]
fn test_a_junk_byte_after_an_opening_token_is_refused_5644() -> Result<()> {
    // The routes: the identity writer and a forced materialization read every byte.
    for filter in [vec!["-c", "."], vec!["-S", "-c", "."]] {
        for (document, stderr) in REFUSED {
            let (out, err, code) = run_jq_full(&filter, Some(document))?;
            assert_eq!(
                (out.as_str(), err.as_str(), code),
                ("", *stderr, 5),
                "{filter:?} {document:?}"
            );
        }
    }
    // `length` on an object reads each key and its `:` (the census scan, `following_gap_ok`).
    for (document, stderr) in [
        (
            "{\"a\":@1,\"b\":2}",
            "jq: parse error: Invalid numeric literal at line 1, column 8\n",
        ),
        (
            "{\"a\":1,\"b\":@\"x\"}",
            "jq: parse error: Invalid numeric literal at line 1, column 13\n",
        ),
    ] {
        let (out, err, code) = run_jq_full(&["-c", "length"], Some(document))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            ("", stderr, 5),
            "length {document:?}"
        );
    }
    // `length` reads only the top level, so it sees a fault among the top-level children (a
    // nested one is the semi-index's documented leniency, as for `[[x3]]`).
    for (document, stderr) in &REFUSED[..9] {
        let (out, err, code) = run_jq_full(&["-c", "length"], Some(document))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            ("", *stderr, 5),
            "length {document:?}"
        );
    }
    Ok(())
}

#[test]
fn test_whitespace_in_every_gap_still_parses_5644() -> Result<()> {
    for (document, want) in [
        ("[ 1 ]", "[1]\n"),
        ("[\n1,\t2\r\n]", "[1,2]\n"),
        ("{ \"a\" : [ 1 ] }", "{\"a\":[1]}\n"),
        (
            "{\"a\"\n:\t1 ,\n\"b\" : { \"c\" : -3 } }",
            "{\"a\":1,\"b\":{\"c\":-3}}\n",
        ),
        ("[[],{},[ ],{ }]", "[[],{},[],{}]\n"),
        ("{\"a\":.5}", "{\"a\":0.5}\n"),
    ] {
        let (out, err, code) = run_jq_full(&["-c", "."], Some(document))?;
        assert_eq!((out.as_str(), code), (want, 0), "{document:?}: {err:?}");
    }
    Ok(())
}

#[test]
fn test_a_form_feed_in_a_gap_is_refused_5644() -> Result<()> {
    for document in [
        "[\x0c1]",
        "{\"a\":\x0c1}",
        "{\"a\"\x0c:1}",
        "{\x0c\"a\":1}",
        // The gap after a `,` counts only JSON whitespace too.
        "[1,\x0c2]",
        "{\"a\":1,\x0c\"b\":2}",
    ] {
        let (out, _, code) = run_jq_full(&["-c", "."], Some(document))?;
        assert_eq!((out.as_str(), code), ("", 5), "{document:?}");
    }
    Ok(())
}
