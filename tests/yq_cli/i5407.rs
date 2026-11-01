//! #5407: a plain assignment whose target is a negative array index (`.a[-1] = .a[0]`) copies
//! the source's quoting, comment and flow style like a non-negative one: the index is counted
//! from the end of the array as the write found it. Every output captured from yq (v4.52.2 here;
//! the copy rules are the ones #5257 pinned from v4.53.3).

use anyhow::Result;

use crate::run_yq_stdin;

/// An array whose elements carry anchors, aliases and comments: the alias-mark pass counts a
/// negative target index the same way.
const ALIASES: &str = "a:\n  - &x {k: \"v\"} # c1\n  - *x\n  - &y \"s\" # c2\n  - *y\n";

/// (document, filter, output).
const ROWS: &[(&str, &str, &str)] = &[
    ("a: [\"x\", y]\n", ".a[-1] = .a[0]", "a: [\"x\", \"x\"]\n"),
    ("a: [\"x\", y]\n", ".a[-2] = .a[1]", "a: [\"y\", y]\n"),
    ("a: [\"x\", 'y']\n", ".a[0] = .a[-1]", "a: [\"y\", 'y']\n"),
    ("a: [\"x\", y]\n", ".a[-1] = .a[-2]", "a: [\"x\", \"x\"]\n"),
    ("a: [[1], [2]]\n", ".a[-1] = .a[0]", "a: [[1], [1]]\n"),
    (
        "a: [\"x\", y]\n",
        "(.a[-1], .a[0]) = .a[1]",
        "a: [\"y\", y]\n",
    ),
    (
        "a:\n  - \"x\" # c\n  - y\n",
        ".a[-1] = .a[0]",
        "a:\n  - \"x\" # c\n  - \"x\" # c\n",
    ),
    (
        "a: [1]\nb: [\"p\", q]\n",
        ".a[-1] = .b[-2]",
        "a: [\"p\"]\nb: [\"p\", q]\n",
    ),
    (
        ALIASES,
        ".a[-3] = .a[-4]",
        "a:\n  - &x {k: \"v\"} # c1\n  - {k: \"v\"} # c1\n  - &y \"s\" # c2\n  - *y\n",
    ),
    (
        ALIASES,
        ".a[-1] = .a[-2]",
        "a:\n  - &x {k: \"v\"} # c1\n  - *x\n  - &y \"s\" # c2\n  - \"s\" # c2\n",
    ),
    (
        ALIASES,
        ".a[-1] = .a[1]",
        "a:\n  - &x {k: \"v\"} # c1\n  - *x\n  - &y \"s\" # c2\n  - *x\n",
    ),
    (
        ALIASES,
        ".a[-4] = .a[-2]",
        "a:\n  - &x \"s\" # c2\n  - *x\n  - &y \"s\" # c2\n  - *y\n",
    ),
];

#[test]
fn test_negative_index_target_copies_the_sources_presentation_5407() -> Result<()> {
    for &(input, filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter} over {input:?}");
    }
    Ok(())
}
