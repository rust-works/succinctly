//! #6379: a `?` followed by a blank (or the end of the line) straight after an implicit key's colon cannot
//! open a mapping: that is `key:` / `  ? x`, or `: ? x` after an explicit key's own colon. yq stops with
//! `mapping keys are not allowed in this context` (`... while parsing a flow mapping` in flow context);
//! `succinctly yq` read the line as the plain scalar `? x` (`c: ? w` was `{"c": "? w"}`). A property
//! between the colon and the `?` changes nothing, and `?x` and `?? x` are plain scalars. Every row was
//! captured from yq v4.53.3 (`yq -o=json -I0 .`).

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

const ARGS: &[&str] = &["-o=json", "-I=0"];

#[test]
fn test_an_explicit_key_indicator_after_an_implicit_keys_colon_is_refused_6379() -> Result<()> {
    for doc in [
        "c: ? w\n",
        "c: ? w\nd: 1\n",
        "\"c\": ? w\n",
        "'c': ? w\n",
        "c:   ? w\n",
        "c:\t? w\n",
        "c: ?\n",
        "c: ? w # x\n",
        "c: ?\n  w\n",
        "- c: ? w\n",
        "- - c: ? w\n",
        "x:\n  c: ? w\n",
        "x:\n  - c: ? w\n",
        "c: &a ? w\n",
        "c: !!str ? w\n",
        "? a: ? w\n",
        "? a\n: b: ? w\n",
        "[a]: ? w\n",
        "{a: 1}: ? w\n",
        "{c: ? w}\n",
        "[c: ? w]\n",
        "{a: ? w}\n",
        "[? c : ? d]\n",
        "{? c : ? d}\n",
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
        assert_ne!(code, 0, "{doc:?} should be refused, got {stdout:?}");
        assert!(
            stderr.contains("mapping keys are not allowed in this context"),
            "{doc:?}: {stderr:?}"
        );
    }
    Ok(())
}

#[test]
fn test_what_is_not_an_explicit_key_after_a_colon_still_reads_6379() -> Result<()> {
    for (doc, expected) in [
        // `?` with no blank after it is a plain scalar.
        ("c: ?w\n", "{\"c\":\"?w\"}"),
        ("c: ?? w\n", "{\"c\":\"?? w\"}"),
        ("[a: ?b]\n", "[{\"a\":\"?b\"}]"),
        ("{c: ?b}\n", "{\"c\":\"?b\"}"),
        ("[? c : ?d]\n", "[{\"c\":\"?d\"}]"),
        ("{? c : ?d}\n", "{\"c\":\"?d\"}"),
        // An explicit key is fine where it opens an entry.
        ("c:\n  ? w\n", "{\"c\":{\"w\":null}}"),
        ("c:\n? w\n", "{\"c\":null,\"w\":null}"),
        ("? w\n", "{\"w\":null}"),
        ("- ? w\n", "[{\"w\":null}]"),
        ("[? w]\n", "[{\"w\":null}]"),
        ("{? c}\n", "{\"c\":null}"),
        ("[? a : b]\n", "[{\"a\":\"b\"}]"),
        ("{? a : b}\n", "{\"a\":\"b\"}"),
        // After an explicit key's own colon it is the value's explicit key (#6340).
        ("? a\n: ? x\n  : y\n", "{\"a\":{\"x\":\"y\"}}"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, ARGS)?;
        assert_eq!((stdout.trim_end(), code), (expected, 0), "{doc:?}");
    }
    Ok(())
}
