//! #6371: an explicit key's own-line plain scalar that a comment ends takes no more lines: yq stops with
//! `did not find expected key` at the next line more indented than the `?`, where `succinctly yq` read it as
//! the entry's value (`? a⏎  b # c⏎  d⏎: v` was `{"a b":"d"}`) and lost the `: v` line. A comment after the
//! scalar with the `:` line next, or the next key, is fine. Every row was captured from yq v4.53.3
//! (`yq -o=json -I0 .`).

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const ARGS: &[&str] = &["-o=json", "-I=0"];

#[test]
fn test_a_line_after_a_comment_that_ended_an_explicit_key_scalar_is_refused_6371() -> Result<()> {
    for doc in [
        "? a\n  b # c\n  d\n: v\n",
        "? a # c\n  b\n: v\n",
        "? a\n  b # c\n  d\n  e\n: v\n",
        "? a b # c\n  d\n: v\n",
        "k:\n  ? a\n    b # c\n    d\n  : v\n",
        "? a\n  b\n  # c\n  d\n: v\n",
        "? a\n# c\n  b\n: v\n",
        "? a\n  b # c\n\n  d\n: v\n",
        "? a # c\n  - x\n: v\n",
        "? a # c\n  k: v\n: w\n",
        "- ? a # c\n    b\n  : v\n",
        "? a\n  b # c\n  d\n",
        "? a # c\n  b",
    ] {
        let (stdout, _, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
        assert_ne!(code, 0, "{doc:?} should be refused, got {stdout:?}");
    }
    Ok(())
}

#[test]
fn test_an_explicit_key_scalar_ended_by_a_comment_still_reads_when_nothing_strays_6371(
) -> Result<()> {
    for (doc, expected) in [
        ("? a\n  b # c\n: v\n", "{\"a b\":\"v\"}"),
        ("? a # c\n: v\n", "{\"a\":\"v\"}"),
        ("? a # c\n\n: v\n", "{\"a\":\"v\"}"),
        ("? a # c\n? b\n: v\n", "{\"a\":null,\"b\":\"v\"}"),
        ("? a # c\n  # d\n: v\n", "{\"a\":\"v\"}"),
        ("? a\n  b\n: v\n", "{\"a b\":\"v\"}"),
        ("? a # c\n", "{\"a\":null}"),
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
