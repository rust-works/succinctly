//! #6325: the compact mapping inside an explicit key, whose first entry has no key and no `?` before its
//! `:` (`? : x`, `- ? : x`, `? - : x`), is `did not find expected key` in yq as the keyless `:` of
//! #6288 is; `succinctly yq` read an empty key with a null value. A tag or an anchor before the `:`
//! (`? !!str : x`) is an empty key yq reads. Every row was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const ARGS: &[&str] = &["-o=json", "-I=0"];

#[test]
fn test_a_bare_colon_entry_inside_an_explicit_key_is_refused_as_yq_refuses_it_6325() -> Result<()> {
    for doc in [
        "? : x\n",
        "- ? : x\n",
        "? - : x\n",
        "? : \n",
        "a:\n  ? : x\n",
        "? : x\n: y\n",
        "? - a\n  - : b\n: c\n",
        "- ? : x\n- ? : y\n",
        "? :\n",
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
        assert_ne!(code, 0, "{doc:?} should be refused, got {stdout:?}");
        assert!(
            stderr.contains("did not find expected key"),
            "{doc:?}: {stderr:?}"
        );
    }
    Ok(())
}

#[test]
fn test_an_explicit_key_with_a_property_or_a_key_still_reads_6325() -> Result<()> {
    for (doc, expected) in [
        ("? !!str : x\n", "{\"\":null}"),
        ("- ? &a : x\n", "[{\"\":null}]"),
        ("? &a : x\n: y\n", "{\"\":\"y\"}"),
        ("? k: v\n", "{\"\":null}"),
        ("? - !!str : b\n: c\n", "{\"\":\"c\"}"),
        ("- ? a\n  : b\n", "[{\"a\":\"b\"}]"),
        ("? - a\n  - b\n: c\n", "{\"\":\"c\"}"),
        ("? a\n: b\n", "{\"a\":\"b\"}"),
        ("? - a: 1\n: v\n", "{\"\":\"v\"}"),
        ("? \n: b\n", "{\"\":\"b\"}"),
        ("- ? !!str : x\n  : y\n", "[{\"\":\"y\"}]"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
        assert_eq!(code, 0, "{doc:?}: {stderr:?}");
        assert_eq!(stdout.trim(), expected, "{doc:?}");
    }
    Ok(())
}
