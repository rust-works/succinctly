//! #5483: yq's `alias =` setter makes a node a `*name` alias of the anchor `name`. Rows marked
//! "yq" were captured from yq v4.53.3 and match it byte for byte; the others pin the recorded
//! divergence (`docs/compliance/yq/limitations.md`): succinctly writes the `*name` only where an
//! earlier `&name` holds an identical value, so it never prints an alias it cannot read back.

use anyhow::Result;

use crate::run_yq_stdin;

const SCALARS: &str = "a: &x 1\nb: 1\nc: 2\n";
const MAPPING: &str = "a: &x {p: 1}\nb: *x\ne: 3\n";

#[test]
fn test_alias_setter_writes_an_alias_of_an_equal_anchor_5483() -> Result<()> {
    // yq: the assigned node becomes `*x`, the anchor stays.
    let rows: &[(&str, &str, &str)] = &[
        (SCALARS, ".b alias = \"x\"", "a: &x 1\nb: *x\nc: 2\n"),
        // `|=` binds `.` to the node, and a constant right side is the same write.
        (SCALARS, ".b alias |= \"x\"", "a: &x 1\nb: *x\nc: 2\n"),
        // Several targets are all written.
        (
            "a: &x 1\nb: 1\nc: 1\n",
            "(.b, .c) alias = \"x\"",
            "a: &x 1\nb: *x\nc: *x\n",
        ),
        // A container is an alias of an equal container.
        (
            "x: &z [1]\ny: [1]\n",
            ".y alias = \"z\"",
            "x: &z [1]\ny: *z\n",
        ),
    ];
    for (input, filter, expected) in rows {
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(out, *expected, "{filter} over {input:?}");
    }
    Ok(())
}

#[test]
fn test_alias_setter_with_an_empty_or_container_name_leaves_the_node_5483() -> Result<()> {
    // yq: an empty text, and a sequence or mapping that has no text, write nothing, and they do
    // not clear an alias that is already there (`anchor = ""` clears; `alias = ""` does not).
    for filter in [
        ".b alias = \"\"",
        ".e alias = [\"a\"]",
        ".e alias = []",
        ".e alias = .a",
        ".e alias = .zz",
    ] {
        let (out, code) = run_yq_stdin(filter, MAPPING, &[])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(out, MAPPING, "{filter}");
    }
    Ok(())
}

#[test]
fn test_alias_setter_refuses_a_name_yq_refuses_5483() -> Result<()> {
    // yq: `yaml: yaml: alias value must contain valid characters only`, the rule of an anchor
    // name (whitespace, `,`, `:`, brackets, braces and non-ASCII are refused).
    for filter in [
        ".b alias = \"x y\"",
        ".b alias = \"a,b\"",
        ".b alias = \"a:b\"",
        ".b alias = \"é\"",
        ".b alias = {}",
    ] {
        let (out, code) = run_yq_stdin(filter, SCALARS, &[])?;
        assert_eq!(code, 1, "{filter}");
        assert_eq!(out, "", "{filter}");
    }
    Ok(())
}

#[test]
fn test_alias_setter_in_jq_mode_is_not_a_metadata_write_5483() -> Result<()> {
    // `alias` is yq's: `succinctly jq` does not read `.b alias = "x"` as a write.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["jq", ".b alias = \"x\""])
        .output()?;
    assert!(!output.status.success());
    Ok(())
}

#[test]
fn test_alias_setter_drops_an_alias_it_could_not_read_back_5483() -> Result<()> {
    // Divergences: yq writes `*name` in each of these and its output no longer reads back (no
    // `&z` at all, an anchor holding another value, a node the write created as `null`), so
    // the node keeps the value it has.
    let rows: &[(&str, &str, &str)] = &[
        // yq: `b: *z`.
        (SCALARS, ".b alias = \"z\"", "a: &x 1\nb: 1\nc: 2\n"),
        // yq: `c: *x`, which reads back as `1`.
        (SCALARS, ".c alias = \"x\"", "a: &x 1\nb: 1\nc: 2\n"),
        // yq: `b: *5`.
        (SCALARS, ".b alias = 5", "a: &x 1\nb: 1\nc: 2\n"),
        // yq: `d: *x`.
        (
            SCALARS,
            ".d alias = \"x\"",
            "a: &x 1\nb: 1\nc: 2\nd: null\n",
        ),
        // yq: `e: *x` over a mapping `x`.
        (MAPPING, ".e alias = \"x\"", MAPPING),
        // yq: `a: *x`, an alias of itself, and `b: *x`; without the declaration `b` has no
        // anchor to name, so both print the value.
        ("a: &x 1\nb: *x\n", ".a alias = \"x\"", "a: 1\nb: 1\n"),
    ];
    for (input, filter, expected) in rows {
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(out, *expected, "{filter} over {input:?}");
    }
    Ok(())
}

#[test]
fn test_a_field_named_alias_is_still_a_field_write_5483() -> Result<()> {
    // yq: `alias` after a `.` with no space is a field name, as `style` and `anchor` are.
    let input = "alias: 5\na: {alias: 1}\n";
    for (filter, expected) in [
        (".alias = 1", "alias: 1\na: {alias: 1}\n"),
        (".a.alias = 3", "alias: 5\na: {alias: 3}\n"),
        (".alias |= 2", "alias: 2\na: {alias: 1}\n"),
    ] {
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(out, expected, "{filter}");
    }
    Ok(())
}
