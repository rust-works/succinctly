//! #5650: a junk byte between a sibling's last byte and its `,`, a form feed around the root or a
//! container's last child, and a bare token glued to a number (`1null`, `1e`) were read as
//! whitespace or dropped: `[1@,2]` printed `[1,2]`. jq 1.7.1 refuses each as `Invalid numeric
//! literal`. Every row and message was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(document, stdout, stderr)`; every row exits 5.
const REFUSED: &[(&str, &str, &str)] = &[
    (
        "[1@,2]",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 4\n",
    ),
    (
        "[\"a\"@,\"b\"]",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 6\n",
    ),
    (
        "[true@,1]",
        "",
        "jq: parse error: Invalid literal at line 1, column 7\n",
    ),
    (
        "[[1]@,2]",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 6\n",
    ),
    (
        "{\"a\":1@,\"b\":2}",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 8\n",
    ),
    (
        "{\"a\":\"x\"@,\"b\":2}",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 10\n",
    ),
    (
        "[1\x0c,2]",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 4\n",
    ),
    (
        "[\"a\"\x0c,\"b\"]",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 6\n",
    ),
    (
        "{\"a\":1\x0c,\"b\":2}",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 8\n",
    ),
    (
        "[1null,2]",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 7\n",
    ),
    (
        "[1true,2]",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 7\n",
    ),
    (
        "[1false,2]",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 8\n",
    ),
    (
        "{\"a\":1null,\"b\":2}",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 11\n",
    ),
    (
        "[1e,2]",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 4\n",
    ),
    (
        "[1e+,2]",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 5\n",
    ),
    (
        "[1.5x,2]",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 6\n",
    ),
    (
        "[1,2\x0c]",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 6\n",
    ),
    (
        "{\"a\":1,\"b\":2\x0c}",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 14\n",
    ),
    (
        "\x0c[1]",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 2\n",
    ),
    (
        "[1,2]\x0c[3]",
        "[1,2]\n",
        "jq: parse error: Invalid numeric literal at line 1, column 7\n",
    ),
    (
        "[1]\x0c",
        "[1]\n",
        "jq: parse error: Invalid numeric literal at EOF at line 1, column 4\n",
    ),
];

#[test]
fn test_a_junk_byte_or_glued_token_after_a_sibling_is_refused_5650() -> Result<()> {
    for (document, stdout, stderr) in REFUSED {
        let (out, err, code) = run_jq_full(&["-c", "."], Some(document))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, *stderr, 5),
            "{document:?}"
        );
    }
    Ok(())
}

/// `(document, stdout)`; spellings jq reads, which the stricter checks must keep reading.
const ACCEPTED: &[(&str, &str)] = &[
    ("[1,2]", "[1,2]\n"),
    ("[ 1 , 2 ]", "[1,2]\n"),
    ("[1\t,\n2\r]", "[1,2]\n"),
    ("[\"a\" ,\"b\"]", "[\"a\",\"b\"]\n"),
    ("[[1] ,2]", "[[1],2]\n"),
    ("{\"a\":1 ,\"b\":2}", "{\"a\":1,\"b\":2}\n"),
    ("[1e5,2]", "[1E+5,2]\n"),
    ("[1.,2]", "[1,2]\n"),
    ("[1.5e+3,2]", "[1.5E+3,2]\n"),
    ("[.5,2]", "[0.5,2]\n"),
    ("[+1,2]", "[1,2]\n"),
    ("[-0,2]", "[-0,2]\n"),
    ("[nan,1]", "[null,1]\n"),
    ("[-nan,1]", "[null,1]\n"),
    ("[NaN,1]", "[null,1]\n"),
    ("[Infinity,1]", "[1.7976931348623157e+308,1]\n"),
    ("[-Infinity,1]", "[-1.7976931348623157e+308,1]\n"),
    ("[Nan4000,1]", "[null,1]\n"),
    ("[true,false,null]", "[true,false,null]\n"),
    ("[1,2]\n[3]", "[1,2]\n[3]\n"),
];

#[test]
fn test_spellings_jq_reads_stay_readable_5650() -> Result<()> {
    for (document, stdout) in ACCEPTED {
        let (out, err, code) = run_jq_full(&["-c", "."], Some(document))?;
        assert_eq!((out.as_str(), code), (*stdout, 0), "{document:?}: {err:?}");
    }
    Ok(())
}
