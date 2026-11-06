//! #6322: an explicit key whose own content is an explicit key (`? ? a⏎  : b⏎: c`) is a mapping used as a key,
//! which yq reads (`{"":"c"}` in JSON: a mapping has no JSON spelling as a key). `succinctly yq` stopped at the
//! inner `:` with `inconsistent indentation`, and read `? ? a⏎: c` as the plain-scalar key `? a`. The inner key
//! waits for its value in front of the outer one, and the outer one takes the next shallower `:`. Every row was
//! captured from yq v4.53.3 (`yq -o=json -I0 .`).

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const ARGS: &[&str] = &["-o=json", "-I=0"];

#[test]
fn test_an_explicit_key_inside_an_explicit_key_reads_as_yq_reads_it_6322() -> Result<()> {
    for (doc, expected) in [
        ("? ? a\n  : b\n: c\n", "{\"\":\"c\"}"),
        ("? ? a\n: c\n", "{\"\":\"c\"}"),
        ("? ?\n: c\n", "{\"\":\"c\"}"),
        ("? ? a\n  : b\n", "{\"\":null}"),
        ("? ? a\n  : b\n? x\n: c\n", "{\"\":null,\"x\":\"c\"}"),
        ("? ? a\n? x\n: c\n", "{\"\":null,\"x\":\"c\"}"),
        ("k:\n  ? ? a\n    : b\n  : c\n", "{\"k\":{\"\":\"c\"}}"),
        ("k:\n  ? ? a\n  : c\n", "{\"k\":{\"\":\"c\"}}"),
        ("- ? ? a\n    : b\n  : c\n", "[{\"\":\"c\"}]"),
        (
            "x: 1\n? ? a\n  : b\n: c\ny: 2\n",
            "{\"x\":1,\"\":\"c\",\"y\":2}",
        ),
        ("? ? ? a\n    : b\n  : c\n: d\n", "{\"\":\"d\"}"),
        ("? ? ? a\n: c\n", "{\"\":\"c\"}"),
        ("? ? ? a\n  ? y\n: c\n", "{\"\":\"c\"}"),
        ("? ? ? a\n  ? y\n  : z\n: c\n", "{\"\":\"c\"}"),
        ("? ? ? a\n    : b\n  : d\n: c\n", "{\"\":\"c\"}"),
        ("? ? a: 1\n  : b\n: c\n", "{\"\":\"c\"}"),
        ("? ? a: 1\n  ? b\n  : 2\n: c\n", "{\"\":\"c\"}"),
        ("? ? a\n  ? b\n: c\n", "{\"\":\"c\"}"),
        ("? ? a\n  : b\n  ? c\n: d\n", "{\"\":\"d\"}"),
        (
            "? ? a\n  : b\n? ? c\n  : d\n: e\n",
            "{\"\":null,\"\":\"e\"}",
        ),
        (
            "? ? a\n  : b\n---\n? ? z\n: y\n",
            "{\"\":null}\n{\"\":\"y\"}",
        ),
        ("? !!str\n  ? a\n: c\n", "{\"\":\"c\"}"),
        ("? \"?\"\n: c\n", "{\"?\":\"c\"}"),
        ("? ?a\n: c\n", "{\"?a\":\"c\"}"),
        ("? ? a\n  ? ? b\n  : c\n: d\n", "{\"\":\"d\"}"),
        ("? ? a\n  : x\n  ? ? b\n  : c\n: d\n", "{\"\":\"d\"}"),
        ("? ? a\n  ? ? b\n  : c\n  ? ? e\n: d\n", "{\"\":\"d\"}"),
        ("? ? ? a\n  ? ? b\n: c\n", "{\"\":\"c\"}"),
        ("? ? ? a\n    ? ? b\n  : c\n: d\n", "{\"\":\"d\"}"),
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

#[test]
fn test_a_misplaced_value_after_a_nested_explicit_key_is_refused_as_yq_refuses_it_6322(
) -> Result<()> {
    for doc in [
        "? ? a\n  : b\n  : d\n: c\n",
        "k:\n  ? ? a\n: c\n",
        "? !!str ? a\n: c\n",
        "? &x ? a\n: c\n",
        "? ? a\n : c\n",
    ] {
        let (stdout, _, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
        assert_ne!(code, 0, "{doc:?} should be refused, got {stdout:?}");
    }
    Ok(())
}
