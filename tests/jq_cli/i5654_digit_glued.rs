//! #5654: a number with bytes glued on that are not a jq token boundary (`0x1`, `12x3`, `0.0x1`)
//! read as the valid number in front of them, because the three JSON number scanners stopped at the
//! first byte outside `[0-9.eE+-]`. jq 1.7.1 reads the whole run up to the next boundary as one
//! token and refuses it (`Invalid numeric literal`). The scanners now follow jq's rule, so every
//! route that reads the element refuses it. Documents, columns and messages were captured from jq
//! 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(document, filters that read the element, stderr)`; every row exits 5 with nothing on stdout.
const REFUSED: &[(&str, &[&str], &str)] = &[
    (
        "[0x1,2]",
        &[".", ".[0]", "first(.[])", "map(.)", "add", "tostring"],
        "jq: parse error: Invalid numeric literal at line 1, column 5\n",
    ),
    (
        "[12x3,2]",
        &[".", ".[0]", "first(.[])", "map(.)", "add", "tostring"],
        "jq: parse error: Invalid numeric literal at line 1, column 6\n",
    ),
    (
        "[0.0x1]",
        &[".", ".[0]", "first(.[])", "map(.)", "add", "tostring"],
        "jq: parse error: Invalid numeric literal at line 1, column 7\n",
    ),
    (
        "[1e5x,2]",
        &[".", ".[0]", "first(.[])", "map(.)", "add", "tostring"],
        "jq: parse error: Invalid numeric literal at line 1, column 6\n",
    ),
    (
        "{\"a\":0x1,\"b\":2}",
        &[".", ".a", "first(.[])", "map(.)", "add", "tostring"],
        "jq: parse error: Invalid numeric literal at line 1, column 9\n",
    ),
];

#[test]
fn a_number_with_bytes_glued_on_is_refused_wherever_it_is_read_5654() -> Result<()> {
    for (doc, filters, stderr) in REFUSED {
        for filter in *filters {
            let (out, err, code) = run_jq_full(&["-c", filter], Some(doc))?;
            assert_eq!(code, 5, "{filter} on {doc}: {out:?} {err:?}");
            assert_eq!(out, "", "{filter} on {doc}");
            assert_eq!(err, *stderr, "{filter} on {doc}");
        }
    }
    Ok(())
}

/// Every spelling jq accepts keeps its output on the routes that read it.
#[test]
fn valid_numbers_keep_their_output_5654() -> Result<()> {
    for (doc, filter, expected) in [
        ("[-0.5e-3,2]", "map(.)", "[-0.0005,2]"),
        ("[.5]", ".", "[0.5]"),
        ("[007,2]", "first(.[])", "7"),
        ("[nan,2]", "map(.)", "[null,2]"),
        ("[-inf]", ".[0]", "-1.7976931348623157e+308"),
        ("{\"a\":12 ,\"b\":2}", ".a", "12"),
        ("[1\n,2]", "add", "3"),
        ("12\n", ".", "12"),
    ] {
        let (out, err, code) = run_jq_full(&["-c", filter], Some(doc))?;
        assert_eq!(code, 0, "{filter} on {doc}: {err:?}");
        assert_eq!(out.trim(), expected, "{filter} on {doc}");
    }
    Ok(())
}

/// A route that never reads the element still answers, as `[1.5.5,2] | length` does (#3035).
#[test]
fn routes_that_never_read_the_element_still_answer_5654() -> Result<()> {
    for (doc, filter, expected) in [("[0x1,2]", "length", "2"), ("[0x1,2]", ".[1]", "2")] {
        let (out, _, code) = run_jq_full(&["-c", filter], Some(doc))?;
        assert_eq!(code, 0, "{filter} on {doc}");
        assert_eq!(out.trim(), expected, "{filter} on {doc}");
    }
    Ok(())
}

/// A glued first element is itself the malformed token, so a consumer that stops early still
/// refuses it, as jq does (it used to answer `1`).
#[test]
fn an_early_stopping_consumer_refuses_a_glued_first_element_5654() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-c", "map(.) | first"], Some("[1@,2]"))?;
    assert_eq!(code, 5, "{out:?} {err:?}");
    assert_eq!(out, "");
    assert_eq!(
        err,
        "jq: parse error: Invalid numeric literal at line 1, column 4\n"
    );
    Ok(())
}
