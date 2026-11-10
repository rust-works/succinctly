//! #5483: yq's `explode(f)` for a path other than `.` expands the aliases in the nodes `f`
//! selects (and their descendants), drops the anchors there and expands `<<` merges there;
//! aliases and anchors elsewhere stay. It parses here as `(f | ..) anchor = ""`. Rows were
//! captured from yq v4.53.3; a row that differs from yq says how.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

/// `a` is anchored once and aliased three times; `y` is anchored inside `d` and aliased outside it.
const DOC: &str = "a: &x\n  p: 1\nb: *x\nc: *x\nd:\n  e: *x\n  f: &y 2\ng: *y\n";

#[test]
fn test_explode_of_a_path_matches_yq_5483() -> Result<()> {
    for (filter, want) in [
        // An alias node selected: it becomes its anchor's value, the anchor and the other aliases stay.
        (
            "explode(.b)",
            "a: &x\n  p: 1\nb:\n  p: 1\nc: *x\nd:\n  e: *x\n  f: &y 2\ng: *y\n",
        ),
        (
            "explode(.c)",
            "a: &x\n  p: 1\nb: *x\nc:\n  p: 1\nd:\n  e: *x\n  f: &y 2\ng: *y\n",
        ),
        // An alias nested under the selected node, whose anchor lies outside it.
        (
            "explode(.d.e)",
            "a: &x\n  p: 1\nb: *x\nc: *x\nd:\n  e:\n    p: 1\n  f: &y 2\ng: *y\n",
        ),
        (
            "explode(.d | .e)",
            "a: &x\n  p: 1\nb: *x\nc: *x\nd:\n  e:\n    p: 1\n  f: &y 2\ng: *y\n",
        ),
        // Several selected nodes.
        (
            "explode(.d.e, .g)",
            "a: &x\n  p: 1\nb: *x\nc: *x\nd:\n  e:\n    p: 1\n  f: &y 2\ng: 2\n",
        ),
        (
            "explode(.c, .d.e)",
            "a: &x\n  p: 1\nb: *x\nc:\n  p: 1\nd:\n  e:\n    p: 1\n  f: &y 2\ng: *y\n",
        ),
    ] {
        let (out, code) = run_yq_stdin(filter, DOC, &[])?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}

#[test]
fn test_explode_of_a_path_expands_a_merge_under_it_5483() -> Result<()> {
    // yq: `explode(.m)` and `explode(.[])` write `m:` / `  p: 1` / `  q: 2` / `  r: 3`.
    let doc = "base: &b {p: 1, q: 2}\nm:\n  <<: *b\n  r: 3\n";
    for filter in ["explode(.m)", "explode(.[])", "explode(..)", "explode(.)"] {
        let (out, code) = run_yq_stdin(filter, doc, &[])?;
        assert!(
            out.contains("m:\n  p: 1\n  q: 2\n  r: 3\n"),
            "{filter}: {out}"
        );
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

#[test]
fn test_explode_of_a_path_that_selects_nothing_leaves_the_document_5483() -> Result<()> {
    // yq writes the same `nope: null` for the absent key it traversed.
    let (out, code) = run_yq_stdin("explode(.nope)", DOC, &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, format!("{DOC}nope: null\n"));
    let (out, code) = run_yq_stdin("explode(select(false))", DOC, &[])?;
    assert_eq!((out.as_str(), code), (DOC, 0));
    Ok(())
}

#[test]
fn test_explode_of_a_path_writes_no_alias_it_cannot_read_back_5483() -> Result<()> {
    // The anchor `y` is inside `.d`, which `explode(.d)` clears: yq leaves `g: *y` with no `&y`
    // anywhere, a document it then refuses to read, and `succinctly yq` writes the value.
    let (out, code) = run_yq_stdin("explode(.d)", DOC, &[])?;
    assert_eq!(
        (out.as_str(), code),
        (
            "a: &x\n  p: 1\nb: *x\nc: *x\nd:\n  e:\n    p: 1\n  f: 2\ng: 2\n",
            0
        )
    );
    // The same for the anchor `x` of the selected node: yq keeps `b: *x`, `c: *x` and `e: *x`.
    let (out, code) = run_yq_stdin("explode(.a)", DOC, &[])?;
    assert_eq!(
        (out.as_str(), code),
        (
            "a:\n  p: 1\nb:\n  p: 1\nc:\n  p: 1\nd:\n  e:\n    p: 1\n  f: &y 2\ng: *y\n",
            0
        )
    );
    Ok(())
}

#[test]
fn test_explode_of_a_path_expands_merges_outside_it_too_5483() -> Result<()> {
    // yq's `explode(.n)` keeps `m`'s `!!merge <<: *x`; a write expands every merge in the document.
    let doc = "a: &x {p: 1}\nm:\n  <<: *x\n  q: 2\nn: *x\n";
    let (out, code) = run_yq_stdin("explode(.n)", doc, &[])?;
    assert_eq!(code, 0);
    assert!(out.contains("m:\n  p: 1\n  q: 2\n"), "{out}");
    assert!(out.contains("a: &x {p: 1}\n"), "{out}");
    Ok(())
}

#[test]
fn test_explode_of_a_value_that_is_no_path_is_an_error_5483() -> Result<()> {
    // yq explodes a computed node that is in no document and answers the input unchanged.
    for filter in ["explode(1)", "explode(\"a\")"] {
        let (out, err, code) = run_yq_stdin_with_stderr(filter, DOC, &[])?;
        assert_eq!((out.as_str(), code), ("", 1), "{filter}");
        assert!(err.contains("Invalid path expression"), "{filter}: {err}");
    }
    Ok(())
}
