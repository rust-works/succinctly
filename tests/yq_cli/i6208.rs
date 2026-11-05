//! #6208: an implicit flow mapping key is a simple key, which has no line break between its first
//! character and its `:`. yq stops with `did not find expected ',' or '}'` on `{x⏎ y: 1}`,
//! `{x⏎: 1}`, `{"x"⏎ : 1}`, a multi-line quoted key, a nested-collection key and a key with a
//! property; `succinctly yq` folded the break into the key (`x y`) or looked past it for the `:`.
//! The same text without a `:` is a multi-line plain scalar entry (`{x⏎ y}` is `x y: null`), and
//! the explicit `? key` form may span lines. Every row was captured from yq v4.53.3.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

#[test]
fn test_implicit_flow_key_across_a_line_break_is_refused_6208() -> Result<()> {
    for doc in [
        "{x\n y: 1}\n",
        "{x\n: 1}\n",
        "{x \n : 1}\n",
        "{x\n\n y: 1}\n",
        "{x\r\n y: 1}\n",
        "{\"x\"\n : 1 }\n",
        "{\"x\n y\": 1}\n",
        "{'x\n y': 1}\n",
        "{[a,\n b]: 1}\n",
        "{{a: 1,\n b: 2}: 1}\n",
        "{&a x\n y: 1}\n",
        "{a: 1,\n b\n : 2}\n",
        "{\"a\": 1,\n\"b\"\n:2}\n",
        "a: {x\n y: 1}\n",
        "- {x\n y: 1}\n",
        "[{\"x\"\n : 1}]\n",
        "[{x\n}: 1]\n",
        "[\"x\n y\": 1]\n",
        "['x\n y': 1]\n",
        "[[a,\n b]: 1]\n",
        "[{a: 1,\n b: 2}: 1]\n",
        "[&a x\n y: 1]\n",
        "[x\n y: 1]\n",
        "[x\n: 1]\n",
        "[a, \"b\n c\": 1]\n",
        "{x y\n z: 1}\n",
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, &["-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{doc:?}: {stderr}");
        // A refusal of the document's syntax (`expected ...`), not some other failure.
        assert!(
            stderr.contains("YAML parse error") && stderr.contains("expected"),
            "{doc:?}: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_flow_mapping_text_across_a_line_break_that_is_not_a_key_still_reads_6208() -> Result<()> {
    for (doc, expected) in [
        ("{x\n}\n", "{\"x\":null}\n"),
        ("{x\n, y: 1}\n", "{\"x\":null,\"y\":1}\n"),
        ("{x\n  y}\n", "{\"x y\":null}\n"),
        ("{? x\n y: 1}\n", "{\"x y\":1}\n"),
        ("{? x\n: 1}\n", "{\"x\":1}\n"),
        ("{a: 1,\n b: 2}\n", "{\"a\":1,\"b\":2}\n"),
        ("{a:\n 1}\n", "{\"a\":1}\n"),
        ("{a: x\n y}\n", "{\"a\":\"x y\"}\n"),
        ("{ x : 1 ,\n y : 2 }\n", "{\"x\":1,\"y\":2}\n"),
        ("{\"x\"\n}\n", "{\"x\":null}\n"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, &["-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{doc:?}");
    }
    Ok(())
}
