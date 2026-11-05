//! #6321: an explicit key (`? a`) whose value is a bare `:` line, with nothing after the colon, is a
//! key with a null value. `succinctly yq` dropped the entry (`? a⏎:` was `{}`) or, with another
//! entry after it, paired the key with that entry's key (`? a⏎:⏎? b⏎: 1` was `{"a":"b"}`, and
//! `x: 1⏎? a⏎:⏎y: 2` was `{"x":1,"a":"y"}`). A property on the colon (`: &y`, `: !!str`) already
//! had its null node. Every expectation is what yq v4.53.3 printed for `-o=json -I0 .`.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// (document, yq's JSON).
const ROWS: &[(&str, &str)] = &[
    ("? a\n:\n", "{\"a\":null}\n"),
    ("? a\n:", "{\"a\":null}\n"),
    ("? a\n: \n", "{\"a\":null}\n"),
    ("? a\n: null\n", "{\"a\":null}\n"),
    ("? a\n: # c\n", "{\"a\":null}\n"),
    ("? a\n:\n\n", "{\"a\":null}\n"),
    ("? a\n:\n? b\n: 1\n", "{\"a\":null,\"b\":1}\n"),
    ("? a\n:\n? b\n:\n", "{\"a\":null,\"b\":null}\n"),
    ("x: 1\n? a\n:\n", "{\"x\":1,\"a\":null}\n"),
    ("x: 1\n? a\n:\ny: 2\n", "{\"x\":1,\"a\":null,\"y\":2}\n"),
    ("- ? a\n  :\n", "[{\"a\":null}]\n"),
    ("- ? a\n  :\n- 2\n", "[{\"a\":null},2]\n"),
    ("m:\n  ? a\n  :\n", "{\"m\":{\"a\":null}}\n"),
    ("m:\n  ? a\n  :\n  b: 1\n", "{\"m\":{\"a\":null,\"b\":1}}\n"),
    ("? - x\n:\n", "{\"\":null}\n"),
    ("? [x]\n:\n", "{\"\":null}\n"),
    ("? \"a\"\n:\n", "{\"a\":null}\n"),
    ("? |\n  a\n:\n", "{\"a\\n\":null}\n"),
    ("? a\n? b\n:\n", "{\"a\":null,\"b\":null}\n"),
    (" ? a\n :\n", "{\"a\":null}\n"),
    ("? a # c\n:\n", "{\"a\":null}\n"),
    ("? a\n:\n# c\n", "{\"a\":null}\n"),
    ("--- \n? a\n:\n", "{\"a\":null}\n"),
    ("? &x a\n:\n", "{\"a\":null}\n"),
    ("? a\n: &y\n", "{\"a\":null}\n"),
    ("? a\n: !!str\n", "{\"a\":\"\"}\n"),
    ("? a\n: !!str\n? b\n:\n", "{\"a\":\"\",\"b\":null}\n"),
    ("? a\n: b\n", "{\"a\":\"b\"}\n"),
    ("? a\n:\n  b\n", "{\"a\":\"b\"}\n"),
    ("? a\n:\n  - x\n", "{\"a\":[\"x\"]}\n"),
    ("? a\n:\n- x\n", "{\"a\":[\"x\"]}\n"),
    ("? a\n:\n  b: 1\n", "{\"a\":{\"b\":1}}\n"),
    ("? a\n:\n\n  b\n", "{\"a\":\"b\"}\n"),
    ("? a\n:\n  # c\n  b\n", "{\"a\":\"b\"}\n"),
    ("? a\n: &y\n  b\n", "{\"a\":\"b\"}\n"),
    ("? a\n:\n# c\n? b\n:\n", "{\"a\":null,\"b\":null}\n"),
];

#[test]
fn test_a_bare_colon_after_an_explicit_key_is_a_null_value_6321() -> Result<()> {
    for &(document, expected) in ROWS {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", document, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{document:?}: {stderr}");
        assert_eq!(stdout, expected, "{document:?}");
    }
    Ok(())
}

/// A second `:` with no key before it is still refused, as yq refuses it.
#[test]
fn test_a_second_bare_colon_is_still_refused_6321() -> Result<()> {
    let (stdout, _, code) = run_yq_stdin_with_stderr(".", "? a\n:\n:\n", &["-o=json", "-I=0"])?;
    assert_ne!(code, 0, "{stdout}");
    Ok(())
}
