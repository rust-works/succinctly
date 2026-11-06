//! #6382: a line more indented than the `?` after a quoted or flow explicit key is `did not find expected key`
//! in yq, with a comment or without: the key node ended on its own line, so the line has nothing to attach to.
//! `succinctly yq` read it as the entry's value and lost the `: v` line (`? "a"⏎  b⏎: v` was `{"a":"b"}`).
//! The `:` of the key's value, a comment, the next key and a node that continues past a line break (a quoted
//! scalar or a flow collection that is still open) are not that line. Every row was captured from yq v4.53.3
//! (`yq -o=json -I0 .`).

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const ARGS: &[&str] = &["-o=json", "-I=0"];

#[test]
fn test_a_line_after_a_quoted_or_flow_explicit_key_is_refused_6382() -> Result<()> {
    for doc in [
        "? \"a\"\n  b\n: v\n",
        "? \"a\" # c\n  b\n: v\n",
        "? 'a' # c\n  d\n: v\n",
        "? 'a'\n  d\n: v\n",
        "? [a] # c\n  b\n: v\n",
        "? [a]\n  b\n: v\n",
        "? {a: b}\n  c\n: v\n",
        "? *x\n  b\n: v\n",
        "? \"a\"\n  : v\n",
        "? \"a\"\n  :x\n: v\n",
        "? [a]\n  - x\n: v\n",
        "k:\n  ? \"a\"\n    b\n  : v\n",
        "- ? \"a\"\n    b\n  : v\n",
        "? \"a\"\n\n  b\n: v\n",
        "? [a, b]\n  k: v\n: w\n",
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
        assert_ne!(code, 0, "{doc:?} should be refused, got {stdout:?}");
        assert!(
            stderr.contains("a line that is neither a key, a sequence item nor a continuation")
                || stderr.contains("inconsistent indentation"),
            "{doc:?}: {stderr:?}"
        );
    }
    Ok(())
}

#[test]
fn test_a_quoted_or_flow_explicit_key_still_reads_when_nothing_strays_6382() -> Result<()> {
    for (doc, expected) in [
        ("? \"a\"\n: v\n", "{\"a\":\"v\"}"),
        ("? \"a\" # c\n: v\n", "{\"a\":\"v\"}"),
        ("? \"a\"\n  # c\n: v\n", "{\"a\":\"v\"}"),
        ("? [a,\n  b]\n: v\n", "{\"\":\"v\"}"),
        ("? \"a\n  b\"\n: v\n", "{\"a b\":\"v\"}"),
        ("? [a]\n? b\n: v\n", "{\"\":null,\"b\":\"v\"}"),
        ("? 'a'\n? b\n: v\n", "{\"a\":null,\"b\":\"v\"}"),
        ("? {a: b}\n: v\n", "{\"\":\"v\"}"),
        ("k:\n  ? \"a\"\n  : v\n", "{\"k\":{\"a\":\"v\"}}"),
        ("- ? [a]\n  : v\n", "[{\"\":\"v\"}]"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{doc:?}: {stderr:?}"
        );
    }
    Ok(())
}
