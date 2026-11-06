//! #6340: the node after an explicit key's `: ` can be a compact mapping whose first entry is an
//! explicit key of its own (`? a` / `: ? x` / `  : y` is `{"a": {"x": "y"}}`), as the node after `? `
//! (#6322) and after `- ` (#339) can. `succinctly yq` read `? x` there as a plain scalar, and refused
//! the `: y` that followed; a key sequence whose first item opens one (`? - ? x`) was refused for
//! `inconsistent indentation`. Every row was captured from yq v4.53.3 (`yq -o=json -I0 .`).

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

const ARGS: &[&str] = &["-o=json", "-I=0"];

#[test]
fn test_an_explicit_key_after_an_explicit_values_colon_is_a_compact_mapping_6340() -> Result<()> {
    for (doc, expected) in [
        ("? a\n: ? x\n  : y\n", "{\"a\":{\"x\":\"y\"}}"),
        ("? a\n: ? x\n", "{\"a\":{\"x\":null}}"),
        ("? a\n: ?\n  : y\n", "{\"a\":{\"\":\"y\"}}"),
        ("? a\n: ?\n", "{\"a\":{\"\":null}}"),
        ("? a\n: ? x y\n", "{\"a\":{\"x y\":null}}"),
        ("? a\n: ? x # c\n  : y\n", "{\"a\":{\"x\":\"y\"}}"),
        ("? a\n: ? \"x\"\n  : y\n", "{\"a\":{\"x\":\"y\"}}"),
        ("? a\n: ? [x]\n  : y\n", "{\"a\":{\"\":\"y\"}}"),
        ("? a\n: ? |\n    k\n  : y\n", "{\"a\":{\"k\\n\":\"y\"}}"),
        (
            "? a\n: ? x\n  : y\n  ? p\n  : q\n",
            "{\"a\":{\"x\":\"y\",\"p\":\"q\"}}",
        ),
        (
            "? a\n: ? x\n  ? y\n  : z\n",
            "{\"a\":{\"x\":null,\"y\":\"z\"}}",
        ),
        ("? a\n: ? x\n  :\n", "{\"a\":{\"x\":null}}"),
        (
            "? a\n: ? x\n  : y\n? b\n: z\n",
            "{\"a\":{\"x\":\"y\"},\"b\":\"z\"}",
        ),
        (
            "? a\n: ? x\n  : ? p\n    : q\n",
            "{\"a\":{\"x\":{\"p\":\"q\"}}}",
        ),
        ("? a\n: ? x\n  : ? p\n", "{\"a\":{\"x\":{\"p\":null}}}"),
        ("? a\n: ? ? x\n    : y\n  : z\n", "{\"a\":{\"\":\"z\"}}"),
        ("- ? a\n  : ? x\n    : y\n", "[{\"a\":{\"x\":\"y\"}}]"),
        ("- ? a\n  : ? x\n- b\n", "[{\"a\":{\"x\":null}},\"b\"]"),
        (
            "m:\n  ? a\n  : ? x\n    : y\n",
            "{\"m\":{\"a\":{\"x\":\"y\"}}}",
        ),
        // The shapes that already read: the explicit key on its own line, or in a sequence.
        ("? a\n:\n  ? x\n  : y\n", "{\"a\":{\"x\":\"y\"}}"),
        ("? a\n: - ? x\n    : y\n", "{\"a\":[{\"x\":\"y\"}]}"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, ARGS)?;
        assert_eq!((stdout.trim_end(), code), (expected, 0), "{doc:?}");
    }
    Ok(())
}

#[test]
fn test_a_key_sequence_whose_first_item_is_an_explicit_key_6340() -> Result<()> {
    for (doc, expected) in [
        ("? - ? x\n    : y\n: v\n", "{\"\":\"v\"}"),
        ("? - ? x\n    : y\n", "{\"\":null}"),
        ("? - ? x\n: v\n", "{\"\":\"v\"}"),
        ("? - ? x\n  - b\n: v\n", "{\"\":\"v\"}"),
        ("? - a\n  - ? x\n    : y\n: v\n", "{\"\":\"v\"}"),
        ("? - ? x\n    : y\n  - ? p\n    : q\n: v\n", "{\"\":\"v\"}"),
        ("- ? - ? x\n      : y\n  : v\n", "[{\"\":\"v\"}]"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, ARGS)?;
        assert_eq!((stdout.trim_end(), code), (expected, 0), "{doc:?}");
    }
    Ok(())
}

#[test]
fn test_a_property_before_an_explicit_key_after_a_colon_is_refused_as_yq_refuses_it_6340(
) -> Result<()> {
    for doc in [
        "? a\n: &t ? x\n  : y\n",
        "? a\n: !!map ? x\n",
        "? a\n: ? x\n: b\n",
    ] {
        let (stdout, _stderr, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
        assert_ne!(code, 0, "{doc:?} should be refused, got {stdout:?}");
    }
    Ok(())
}
