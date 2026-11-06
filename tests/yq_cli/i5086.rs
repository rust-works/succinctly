//! #5086 (rows 1-2 and the anchored/tagged forms): a plain scalar on a line of its own under a `-`, a compact
//! mapping's key, or behind an anchor or tag is one folded scalar that ends at the `: ` of a later line, and yq stops
//! there with `mapping values are not allowed in this context`. The scalar was scanned from its own indent, so a later
//! line at that indent was not a continuation and the `key: value` line was dropped (`-` / `  a` / `  z: 1` was
//! `["a"]`); it is scanned from the owning block's indent now, as under a `key:` (#5083). Each row is a live yq
//! v4.53.3 capture.
//!
//! The rest of the issue's rows are further down: an explicit key whose own-line scalar ends at a later line's `: `
//! (`? a` / `  b: 1`), which yq refuses the same way, and a `-` on an indent between two open blocks, which yq
//! refuses with `did not find expected '-' indicator` and which #325/#485/#900 had read as the next item.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

/// Documents yq rejects with `mapping values are not allowed in this context`.
const REJECTED: &[&str] = &[
    "-\n  a\n  z: 1\n",
    "- k:\n    x\n    y: 1\n",
    "k:\n  &x a\n  z: 1\n",
    "k:\n  !!str a\n  z: 1\n",
    "- &x\n  a\n  z: 1\n",
    "k: &x\n  a\n  z: 1\n",
    "k: !!str\n  a\n  z: 1\n",
    "k: &x !!str\n  a\n  z: 1\n",
    "-\n  a\n  b\n  z: 1\n",
    "- k:\n    x\n    y\n    z: 1\n",
    "- -\n    a\n    z: 1\n",
    "-\n  - a\n    z: 1\n",
    "- - k:\n      x\n      y: 1\n",
    // An explicit key is a block node too: the scan ends at the later line's `: `.
    "? a\n  b: 1\n",
    "?\n  a\n  z: 1\n",
    "? a\n  b\n  c: 1\n: v\n",
    "k: v\n? a\n  b: 1\n",
    "- ? a\n    b: 1\n",
];

/// (document, `-o json -I0 .` output): continuations that are text, and siblings.
const ACCEPTED: &[(&str, &str)] = &[
    ("-\n  a\n  b\n", "[\"a b\"]\n"),
    ("-\n  a\n  b\n- c\n", "[\"a b\",\"c\"]\n"),
    (
        "- k:\n    x\n    y\n  m: 1\n",
        "[{\"k\":\"x y\",\"m\":1}]\n",
    ),
    ("k:\n  &x a\n  b\nm: 1\n", "{\"k\":\"a b\",\"m\":1}\n"),
    ("k: &x\n  a\n  b\nm: 1\n", "{\"k\":\"a b\",\"m\":1}\n"),
    (
        "- k:\n    !!str a\n    z\n   deep\n",
        "[{\"k\":\"a z deep\"}]\n",
    ),
    ("- k:\n    a b\n  m: 1\n", "[{\"k\":\"a b\",\"m\":1}]\n"),
    ("-\n  a\n- b\n", "[\"a\",\"b\"]\n"),
    ("- -\n    a\n    b\n- c\n", "[[\"a b\"],\"c\"]\n"),
    // An explicit key's continuation lines are text, and its value follows on the `:` line.
    ("? a\n  b\n: v\n", "{\"a b\":\"v\"}\n"),
    ("?\n  a\n  b\n: v\nk: w\n", "{\"a b\":\"v\",\"k\":\"w\"}\n"),
    ("? a\n: v\n", "{\"a\":\"v\"}\n"),
    // A sequence at its key's own indent, and nested ones at their own indents, are not gaps.
    ("- k:\n  - 1\n", "[{\"k\":[1]}]\n"),
    (
        "a:\n  - x\n  - y\nb: 1\n",
        "{\"a\":[\"x\",\"y\"],\"b\":1}\n",
    ),
    ("- - a\n  - b\n- c\n", "[[\"a\",\"b\"],\"c\"]\n"),
    ("a:\n - x\n - y\n", "{\"a\":[\"x\",\"y\"]}\n"),
    // A sequence-valued explicit key: its items are at the `-`'s own column, whichever line it is on.
    ("?\n - a\n - b\n", "{\"\":null}\n"),
    ("?\n - k: a\n: v\n", "{\"\":\"v\"}\n"),
    ("  ?\n   - k: a\n  : v\n", "{\"\":\"v\"}\n"),
    ("?   - a\n    - b\n: v\n", "{\"\":\"v\"}\n"),
    // A `-` on a nested item's virtual indent that is the item's content.
    ("-\n - a\n - b\n", "[[\"a\",\"b\"]]\n"),
];

/// Documents yq rejects with `did not find expected ...`: a `-` whose indent falls between two open blocks.
const DASH_GAPS: &[&str] = &[
    "- k: a\n - z\n",
    "- k: a\n  j: b\n - z\n",
    "a:\n  - x\n - y\n",
    "- - a\n - b\n",
    "b:\n    - x\n   - y\nc: 2\n",
    "-   a: hello\n  - b\n",
    "a:\n  b:\n    - x\n   - y\n  c: 2\n",
];

#[test]
fn test_own_line_scalar_under_a_dash_or_compact_key_is_scanned_from_the_owner_5086() -> Result<()> {
    for doc in REJECTED {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, &[])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{doc:?}");
        assert!(
            stderr.contains("mapping values are not allowed in this context"),
            "{doc:?}: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_own_line_scalar_continuations_and_siblings_stay_5086() -> Result<()> {
    for (doc, expected) in ACCEPTED {
        let (stdout, code) = run_yq_stdin(".", doc, &["-o", "json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (*expected, 0), "{doc:?}");
    }
    Ok(())
}

#[test]
fn test_a_dash_between_two_open_blocks_is_refused_as_yq_refuses_it_5086() -> Result<()> {
    for doc in DASH_GAPS {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, &[])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{doc:?}");
        assert!(
            stderr.contains("inconsistent indentation"),
            "{doc:?}: {stderr}"
        );
    }
    Ok(())
}
