//! #5483: yq's `explode(.)` expands every alias into a copy of its anchor's node, drops the
//! anchors and expands `<<` merges. It parses here as the write that clears every anchor, whose
//! aliases are then written out as the nodes they stood for. Rows were captured from yq v4.53.3.

use anyhow::Result;

use crate::{run_jq_stdin, run_yq_stdin};

#[test]
fn test_explode_dot_matches_yq_5483() -> Result<()> {
    for (input, filter, want) in [
        (
            "a: &x\n  p: 1\nb: *x\nc: &y 2\nd: *y\n",
            "explode(.)",
            "a:\n  p: 1\nb:\n  p: 1\nc: 2\nd: 2\n",
        ),
        (
            "a: &x\n  p: 1\nb: *x\nc: &y 2\nd: *y\n",
            "explode(.) | .b",
            "p: 1\n",
        ),
        (
            "a: &x\n  p: 1\nb: *x\n",
            "explode( . )",
            "a:\n  p: 1\nb:\n  p: 1\n",
        ),
        (
            "a: &x\n  p: 1\nb: *x\n",
            "explode((.))",
            "a:\n  p: 1\nb:\n  p: 1\n",
        ),
        (
            "base: &b {p: 1, q: 2}\nitem:\n  <<: *b\n  r: 3\n",
            "explode(.)",
            "base: {p: 1, q: 2}\nitem:\n  p: 1\n  q: 2\n  r: 3\n",
        ),
        (
            "base: &b {p: 1, q: 2}\nitem:\n  <<: *b\n  r: 3\n",
            "explode(.) | .item",
            "p: 1\nq: 2\nr: 3\n",
        ),
        (
            "a: &a {x: 1}\nb: &b {y: 2}\nc:\n  <<: [*a, *b]\n  z: 3\n",
            "explode(.)",
            "a: {x: 1}\nb: {y: 2}\nc:\n  y: 2\n  x: 1\n  z: 3\n",
        ),
        (
            "a: &x {p: 1}\nb:\n  <<: *x\n  p: 2\n",
            "explode(.)",
            "a: {p: 1}\nb:\n  p: 2\n",
        ),
        (
            "- &s 1\n- *s\n- {k: *s}\n",
            "explode(.)",
            "- 1\n- 1\n- {k: 1}\n",
        ),
        (
            "a: &s hello\nb: *s\nc: [*s, *s]\n",
            "explode(.)",
            "a: hello\nb: hello\nc: [hello, hello]\n",
        ),
        ("a: &s hello\nb: *s\n", "explode(.) | .b", "hello\n"),
        ("a: 1\nb: [2]\n", "explode(.)", "a: 1\nb: [2]\n"),
        (
            "a: &x 1\nb: *x\n---\nc: &y 2\nd: *y\n",
            "explode(.)",
            "a: 1\nb: 1\n---\nc: 2\nd: 2\n",
        ),
        (
            "a: &x\n  inner: &i\n    - 1\n    - 2\nb: *x\nc: *i\n",
            "explode(.)",
            "a:\n  inner:\n    - 1\n    - 2\nb:\n  inner:\n    - 1\n    - 2\nc:\n  - 1\n  - 2\n",
        ),
        (
            "a: &x\n  p: 1\nb: *x\n",
            "explode(.) | .. style = \"flow\"",
            "{a: {p: 1}, b: {p: 1}}\n",
        ),
    ] {
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter} over {input:?}");
    }
    Ok(())
}

#[test]
fn test_explode_of_a_path_other_than_dot_is_refused_5483() -> Result<()> {
    // yq's `explode(.a)` leaves the aliases outside `.a` alone, which this route cannot do yet.
    for filter in ["explode(.a)", "explode(.a, .b)", "explode(..)"] {
        let (out, code) = run_yq_stdin(filter, "a: &x 1\nb: *x\n", &[])?;
        assert_eq!((out.as_str(), code), ("", 1), "{filter}");
    }
    Ok(())
}

#[test]
fn test_explode_inside_a_navigation_is_refused_not_answered_wrongly_5483() -> Result<()> {
    // `.b | explode(.)` is the write behind a navigation, which the metadata write refuses.
    let (out, code) = run_yq_stdin(".b | explode(.)", "a: &x\n  p: 1\nb: *x\n", &[])?;
    assert_eq!((out.as_str(), code), ("", 1));
    Ok(())
}

#[test]
fn test_jq_explode_is_still_the_string_builtin_5483() -> Result<()> {
    let (out, code) = run_jq_stdin("explode", "\"ab\"", &["-c"])?;
    assert_eq!((out.as_str(), code), ("[97,98]\n", 0));
    let (_, code) = run_jq_stdin("explode(.)", "\"ab\"", &["-c"])?;
    assert_ne!(code, 0);
    Ok(())
}
