//! #6980: `@uri` and `@base64` of a value the filter computes or writes as a literal refuse a
//! non-string as yq does, naming its tag, the way a document node already did (#5836). jq mode still
//! stringifies first.
//!
//! Every yq row is captured from yq v4.53.3 on `a: 1` / `s: x y`, with `-o=json -I=0` and with YAML
//! output (both agree).

use anyhow::Result;

use crate::{run_jq_stdin, run_yq_stdin_with_stderr};

const INPUT: &str = "a: 1\ns: x y\n";

const URI_HINT: &str = "can only operate on strings. Please first pipe through another encoding \
                        operator to convert the value to a string";

/// `(filter, tag yq names, format word)`: each refused.
const REFUSED: &[(&str, &str, &str)] = &[
    ("1 | @uri", "!!int", "URI"),
    (".a + 1 | @uri", "!!int", "URI"),
    ("null | @uri", "!!null", "URI"),
    ("true | @uri", "!!bool", "URI"),
    ("1.5 | @uri", "!!float", "URI"),
    ("[1] | @uri", "!!seq", "URI"),
    ("{\"k\": 1} | @uri", "!!map", "URI"),
    ("[1, \"a\"] | map(@uri)", "!!int", "URI"),
    ("1 | @base64", "!!int", "base64"),
    ("null | @base64", "!!null", "base64"),
    ("false | @base64", "!!bool", "base64"),
    ("[.a] | @base64", "!!seq", "base64"),
    ("{} | @base64", "!!map", "base64"),
    ("[1] | .[0] |= @base64", "!!int", "base64"),
    (".a = (2 | @base64)", "!!int", "base64"),
];

/// `(filter, expected -o=json -I=0 output)`: a string is still encoded.
const ENCODED: &[(&str, &str)] = &[
    ("\"x y\" | @uri", "\"x+y\"\n"),
    (".s | @uri", "\"x+y\"\n"),
    ("\"hi\" | @base64", "\"aGk=\"\n"),
    (".s + \"!\" | @base64", "\"eCB5IQ==\"\n"),
    ("(.a | tostring) | @uri", "\"1\"\n"),
];

#[test]
fn yq_uri_and_base64_refuse_a_computed_non_string_6980() -> Result<()> {
    for &(filter, tag, format) in REFUSED {
        let expected = if format == "URI" {
            format!("Error: cannot encode {tag} as URI, {URI_HINT}")
        } else {
            format!("Error: cannot encode {tag} as base64, can only operate on strings")
        };
        for extra in [&["-o=json", "-I=0"][..], &[][..]] {
            let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, INPUT, extra)?;
            assert_eq!(code, 1, "{filter} {extra:?}: exit code (stderr: {stderr})");
            assert_eq!(stdout, "", "{filter} {extra:?}: nothing printed");
            assert_eq!(stderr.trim_end(), expected, "{filter} {extra:?}");
        }
    }
    for &(filter, expected) in ENCODED {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, INPUT, &["-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}: {stderr}");
    }
    Ok(())
}

/// jq stringifies a scalar first and JSON-encodes a container (#1096), unchanged by #6980
/// (captured from jq 1.7.1).
#[test]
fn jq_uri_and_base64_still_stringify_a_non_string_6980() -> Result<()> {
    for (filter, expected) in [
        ("1 | @uri", "\"1\"\n"),
        ("null | @uri", "\"null\"\n"),
        ("[1,2] | @uri", "\"%5B1%2C2%5D\"\n"),
        ("1 | @base64", "\"MQ==\"\n"),
        ("true | @base64", "\"dHJ1ZQ==\"\n"),
    ] {
        let (stdout, code) = run_jq_stdin(filter, "null", &["-c"])?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}");
    }
    Ok(())
}
