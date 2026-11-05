//! #6288: a block-mapping `:` line with no `?` before it (an entry with no key) is `did not find expected
//! key` in yq; `succinctly yq` read it as an empty key, and dropped its value after a mapping entry
//! (`a: 1⏎: b` answered `{"a":1}`). A `:` is only the value of a `?` key that is waiting for one,
//! and a tag or an anchor before the `:` (`!!str : 1`) is an empty key yq reads. Every row was
//! captured from yq v4.53.3 (`yq -o=json -I0 .`).

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const ARGS: &[&str] = &["-o=json", "-I=0"];

#[test]
fn test_a_colon_with_no_explicit_key_is_refused_as_yq_refuses_it_6288() -> Result<()> {
    for doc in [
        ": a\n",
        "a: 1\n: b\n",
        "- 1\n: b\n",
        "a:\n  : b\n",
        "---\n: a\n",
        "- : b\n",
        "- a: 1\n  : b\n",
        "a:\n  ? b\n  : c\n  : d\n",
        "? a\n: b\n: c\n",
        ": \n",
        "a: 1\n:\n",
        "a: 1\n:  # c\n",
        "a:\n- 1\n: b\n",
        "---\n? a\n: b\n---\n: c\n",
        "[a]\n: b\n",
        "k: v\n: w # c\n",
        ": : a\n",
        "- - : a\n",
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
        assert_ne!(code, 0, "{doc:?} should be refused, got {stdout:?}");
        assert!(
            stderr.contains("did not find expected"),
            "{doc:?}: {stderr:?}"
        );
    }
    Ok(())
}

#[test]
fn test_a_colon_after_an_explicit_key_or_a_property_still_reads_6288() -> Result<()> {
    for (doc, expected) in [
        ("? a\n: b\n", "{\"a\":\"b\"}"),
        ("? \n: b\n", "{\"\":\"b\"}"),
        ("?\n: b\n", "{\"\":\"b\"}"),
        ("? a: 1\n: b\n", "{\"\":\"b\"}"),
        ("? - a\n  - b\n: c\n", "{\"\":\"c\"}"),
        ("- ? a\n  : b\n", "[{\"a\":\"b\"}]"),
        ("- ? \n  : b\n", "[{\"\":\"b\"}]"),
        ("a:\n  ? b\n  : c\n", "{\"a\":{\"b\":\"c\"}}"),
        ("? a\n? b\n: c\n", "{\"a\":null,\"b\":\"c\"}"),
        ("? a\n: b\n? c\n: d\n", "{\"a\":\"b\",\"c\":\"d\"}"),
        ("? |\n  x\n: y\n", "{\"x\\n\":\"y\"}"),
        ("? &k a\n: b\n", "{\"a\":\"b\"}"),
        ("? !!str a\n: b\n", "{\"a\":\"b\"}"),
        ("? a\n\n: b\n", "{\"a\":\"b\"}"),
        ("? a\n# c\n: b\n", "{\"a\":\"b\"}"),
        ("a: b\n? c\n: d\n", "{\"a\":\"b\",\"c\":\"d\"}"),
        ("- !!str : b\n", "[{\"\":\"b\"}]"),
        ("- &a : b\n", "[{\"\":\"b\"}]"),
        ("!!str : 1\n", "{\"\":1}"),
        ("&a : 1\n", "{\"\":1}"),
        ("a: 1\nb: 2\n? c\n: d\n", "{\"a\":1,\"b\":2,\"c\":\"d\"}"),
        (
            "- ? a\n  : b\n- ? c\n  : d\n",
            "[{\"a\":\"b\"},{\"c\":\"d\"}]",
        ),
        ("? a\n: - 1\n  - 2\n", "{\"a\":[1,2]}"),
        ("? [a, b]\n: c\n", "{\"\":\"c\"}"),
        ("? a\n: b\n? c\n: d\n", "{\"a\":\"b\",\"c\":\"d\"}"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
        assert_eq!(code, 0, "{doc:?}: {stderr:?}");
        assert_eq!(stdout.trim(), expected, "{doc:?}");
    }
    Ok(())
}
