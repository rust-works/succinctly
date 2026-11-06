//! #6214 (the flow-collection half): go-yaml ends a line at U+2028, U+2029 and U+0085 as at `\n`, so a
//! tab at the start of the line after one is refused below the enclosing block's indent, as after a
//! `\n` (#6059). The break itself stays scalar content that folds as before (#4913). Every row was
//! captured from yq v4.53.3. The first two tests cover the flow scanners; the rest of the file covers the
//! block-context scanners. The empty-key host (`: [x\n\ty]`) is not changed (#6288).

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_tab_after_a_unicode_break_in_a_flow_plain_scalar_is_refused_6214() -> Result<()> {
    for input in [
        "a: [x\u{2028}\ty]\n",
        "a: [x\u{2028}\t y]\n",
        "a: [x\u{2029}\ty]\n",
        "a: [x\u{2029}\t y]\n",
        "a: [x\u{85}\ty]\n",
        "a: [x\u{85}\t y]\n",
        "a:\n  - [x\u{2028}\ty]\n",
        "a:\n  - [x\u{2028}\t y]\n",
        "a:\n  - [x\u{2028}  \ty]\n",
        "a:\n  - [x\u{2028} \ty]\n",
        "a:\n  - [x\u{2029}\ty]\n",
        "a:\n  - [x\u{2029}\t y]\n",
        "a:\n  - [x\u{2029}  \ty]\n",
        "a:\n  - [x\u{2029} \ty]\n",
        "a:\n  - [x\u{85}\ty]\n",
        "a:\n  - [x\u{85}\t y]\n",
        "a:\n  - [x\u{85}  \ty]\n",
        "a:\n  - [x\u{85} \ty]\n",
        "a: {k: x\u{2028}\ty}\n",
        "a: {k: x\u{2028}\t y}\n",
        "a: {k: x\u{2029}\ty}\n",
        "a: {k: x\u{2029}\t y}\n",
        "a: {k: x\u{85}\ty}\n",
        "a: {k: x\u{85}\t y}\n",
        "a: [? x\u{2028}\ty : 1]\n",
        "a: [? x\u{2028}\t y : 1]\n",
        "a: [? x\u{2029}\ty : 1]\n",
        "a: [? x\u{2029}\t y : 1]\n",
        "a: [? x\u{85}\ty : 1]\n",
        "a: [? x\u{85}\t y : 1]\n",
        "? [x\u{2028}\ty]\n: 1\n",
        "? [x\u{2028}\t y]\n: 1\n",
        "? [x\u{2029}\ty]\n: 1\n",
        "? [x\u{2029}\t y]\n: 1\n",
        "? [x\u{85}\ty]\n: 1\n",
        "? [x\u{85}\t y]\n: 1\n",
        "a: [x\u{2028}\u{2028}\ty]\n",
        "a: [x\u{2028}\u{2028}\t y]\n",
        "a: [x\u{2029}\u{2029}\ty]\n",
        "a: [x\u{2029}\u{2029}\t y]\n",
        "a: [x\u{85}\u{85}\ty]\n",
        "a: [x\u{85}\u{85}\t y]\n",
        "- [x\u{2028}\ty]\n",
        "- [x\u{2028}\t y]\n",
        "- [x\u{2029}\ty]\n",
        "- [x\u{2029}\t y]\n",
        "- [x\u{85}\ty]\n",
        "- [x\u{85}\t y]\n",
        "- a: [x\u{2028}\ty]\n",
        "- a: [x\u{2028}\t y]\n",
        "- a: [x\u{2028}  \ty]\n",
        "- a: [x\u{2028} \ty]\n",
        "- a: [x\u{2029}\ty]\n",
        "- a: [x\u{2029}\t y]\n",
        "- a: [x\u{2029}  \ty]\n",
        "- a: [x\u{2029} \ty]\n",
        "- a: [x\u{85}\ty]\n",
        "- a: [x\u{85}\t y]\n",
        "- a: [x\u{85}  \ty]\n",
        "- a: [x\u{85} \ty]\n",
        "a:\n  b: [x\u{2028}\ty]\n",
        "a:\n  b: [x\u{2028}\t y]\n",
        "a:\n  b: [x\u{2028}  \ty]\n",
        "a:\n  b: [x\u{2028} \ty]\n",
        "a:\n  b: [x\u{2029}\ty]\n",
        "a:\n  b: [x\u{2029}\t y]\n",
        "a:\n  b: [x\u{2029}  \ty]\n",
        "a:\n  b: [x\u{2029} \ty]\n",
        "a:\n  b: [x\u{85}\ty]\n",
        "a:\n  b: [x\u{85}\t y]\n",
        "a:\n  b: [x\u{85}  \ty]\n",
        "a:\n  b: [x\u{85} \ty]\n",
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(out.trim_end(), "", "{input:?}");
        assert_ne!(code, 0, "{input:?}");
    }
    Ok(())
}

#[test]
fn a_unicode_break_in_a_flow_plain_scalar_without_that_tab_is_read_6214() -> Result<()> {
    for (input, expected) in [
        ("a: [x\u{2028}y]\n", "{\"a\":[\"x\\u2028y\"]}"),
        ("a: [x\u{2028} y]\n", "{\"a\":[\"x\\u2028y\"]}"),
        ("a: [x\u{2028}  y]\n", "{\"a\":[\"x\\u2028y\"]}"),
        ("a: [x\u{2028}  \ty]\n", "{\"a\":[\"x\\u2028y\"]}"),
        ("a: [x\u{2028} \ty]\n", "{\"a\":[\"x\\u2028y\"]}"),
        ("a: [x\u{2029}y]\n", "{\"a\":[\"x\\u2029y\"]}"),
        ("a: [x\u{2029} y]\n", "{\"a\":[\"x\\u2029y\"]}"),
        ("a: [x\u{2029}  y]\n", "{\"a\":[\"x\\u2029y\"]}"),
        ("a: [x\u{2029}  \ty]\n", "{\"a\":[\"x\\u2029y\"]}"),
        ("a: [x\u{2029} \ty]\n", "{\"a\":[\"x\\u2029y\"]}"),
        ("a: [x\u{85}y]\n", "{\"a\":[\"x y\"]}"),
        ("a: [x\u{85} y]\n", "{\"a\":[\"x y\"]}"),
        ("a: [x\u{85}  y]\n", "{\"a\":[\"x y\"]}"),
        ("a: [x\u{85}  \ty]\n", "{\"a\":[\"x y\"]}"),
        ("a: [x\u{85} \ty]\n", "{\"a\":[\"x y\"]}"),
        ("[x\u{2028}y]\n", "[\"x\\u2028y\"]"),
        ("[x\u{2028}\ty]\n", "[\"x\\u2028y\"]"),
        ("[x\u{2028} y]\n", "[\"x\\u2028y\"]"),
        ("[x\u{2028}  y]\n", "[\"x\\u2028y\"]"),
        ("[x\u{2028}\t y]\n", "[\"x\\u2028y\"]"),
        ("[x\u{2028}  \ty]\n", "[\"x\\u2028y\"]"),
        ("[x\u{2028} \ty]\n", "[\"x\\u2028y\"]"),
        ("[x\u{2029}y]\n", "[\"x\\u2029y\"]"),
        ("[x\u{2029}\ty]\n", "[\"x\\u2029y\"]"),
        ("[x\u{2029} y]\n", "[\"x\\u2029y\"]"),
        ("[x\u{2029}  y]\n", "[\"x\\u2029y\"]"),
        ("[x\u{2029}\t y]\n", "[\"x\\u2029y\"]"),
        ("[x\u{2029}  \ty]\n", "[\"x\\u2029y\"]"),
        ("[x\u{2029} \ty]\n", "[\"x\\u2029y\"]"),
        ("[x\u{85}y]\n", "[\"x y\"]"),
        ("[x\u{85}\ty]\n", "[\"x y\"]"),
        ("[x\u{85} y]\n", "[\"x y\"]"),
        ("[x\u{85}  y]\n", "[\"x y\"]"),
        ("[x\u{85}\t y]\n", "[\"x y\"]"),
        ("[x\u{85}  \ty]\n", "[\"x y\"]"),
        ("[x\u{85} \ty]\n", "[\"x y\"]"),
        ("a:\n  - [x\u{2028}y]\n", "{\"a\":[[\"x\\u2028y\"]]}"),
        ("a:\n  - [x\u{2028} y]\n", "{\"a\":[[\"x\\u2028y\"]]}"),
        ("a:\n  - [x\u{2028}  y]\n", "{\"a\":[[\"x\\u2028y\"]]}"),
        ("a:\n  - [x\u{2029}y]\n", "{\"a\":[[\"x\\u2029y\"]]}"),
        ("a:\n  - [x\u{2029} y]\n", "{\"a\":[[\"x\\u2029y\"]]}"),
        ("a:\n  - [x\u{2029}  y]\n", "{\"a\":[[\"x\\u2029y\"]]}"),
        ("a:\n  - [x\u{85}y]\n", "{\"a\":[[\"x y\"]]}"),
        ("a:\n  - [x\u{85} y]\n", "{\"a\":[[\"x y\"]]}"),
        ("a:\n  - [x\u{85}  y]\n", "{\"a\":[[\"x y\"]]}"),
        ("a: {k: x\u{2028}y}\n", "{\"a\":{\"k\":\"x\\u2028y\"}}"),
        ("a: {k: x\u{2028} y}\n", "{\"a\":{\"k\":\"x\\u2028y\"}}"),
        ("a: {k: x\u{2028}  y}\n", "{\"a\":{\"k\":\"x\\u2028y\"}}"),
        ("a: {k: x\u{2028}  \ty}\n", "{\"a\":{\"k\":\"x\\u2028y\"}}"),
        ("a: {k: x\u{2028} \ty}\n", "{\"a\":{\"k\":\"x\\u2028y\"}}"),
        ("a: {k: x\u{2029}y}\n", "{\"a\":{\"k\":\"x\\u2029y\"}}"),
        ("a: {k: x\u{2029} y}\n", "{\"a\":{\"k\":\"x\\u2029y\"}}"),
        ("a: {k: x\u{2029}  y}\n", "{\"a\":{\"k\":\"x\\u2029y\"}}"),
        ("a: {k: x\u{2029}  \ty}\n", "{\"a\":{\"k\":\"x\\u2029y\"}}"),
        ("a: {k: x\u{2029} \ty}\n", "{\"a\":{\"k\":\"x\\u2029y\"}}"),
        ("a: {k: x\u{85}y}\n", "{\"a\":{\"k\":\"x y\"}}"),
        ("a: {k: x\u{85} y}\n", "{\"a\":{\"k\":\"x y\"}}"),
        ("a: {k: x\u{85}  y}\n", "{\"a\":{\"k\":\"x y\"}}"),
        ("a: {k: x\u{85}  \ty}\n", "{\"a\":{\"k\":\"x y\"}}"),
        ("a: {k: x\u{85} \ty}\n", "{\"a\":{\"k\":\"x y\"}}"),
        ("a: [? x\u{2028}y : 1]\n", "{\"a\":[{\"x\\u2028y\":1}]}"),
        ("a: [? x\u{2028} y : 1]\n", "{\"a\":[{\"x\\u2028y\":1}]}"),
        ("a: [? x\u{2028}  y : 1]\n", "{\"a\":[{\"x\\u2028y\":1}]}"),
        ("a: [? x\u{2028}  \ty : 1]\n", "{\"a\":[{\"x\\u2028y\":1}]}"),
        ("a: [? x\u{2028} \ty : 1]\n", "{\"a\":[{\"x\\u2028y\":1}]}"),
        ("a: [? x\u{2029}y : 1]\n", "{\"a\":[{\"x\\u2029y\":1}]}"),
        ("a: [? x\u{2029} y : 1]\n", "{\"a\":[{\"x\\u2029y\":1}]}"),
        ("a: [? x\u{2029}  y : 1]\n", "{\"a\":[{\"x\\u2029y\":1}]}"),
        ("a: [? x\u{2029}  \ty : 1]\n", "{\"a\":[{\"x\\u2029y\":1}]}"),
        ("a: [? x\u{2029} \ty : 1]\n", "{\"a\":[{\"x\\u2029y\":1}]}"),
        ("a: [? x\u{85}y : 1]\n", "{\"a\":[{\"x y\":1}]}"),
        ("a: [? x\u{85} y : 1]\n", "{\"a\":[{\"x y\":1}]}"),
        ("a: [? x\u{85}  y : 1]\n", "{\"a\":[{\"x y\":1}]}"),
        ("a: [? x\u{85}  \ty : 1]\n", "{\"a\":[{\"x y\":1}]}"),
        ("a: [? x\u{85} \ty : 1]\n", "{\"a\":[{\"x y\":1}]}"),
        ("? [x\u{2028}y]\n: 1\n", "{\"\":1}"),
        ("? [x\u{2028} y]\n: 1\n", "{\"\":1}"),
        ("? [x\u{2028}  y]\n: 1\n", "{\"\":1}"),
        ("? [x\u{2028}  \ty]\n: 1\n", "{\"\":1}"),
        ("? [x\u{2028} \ty]\n: 1\n", "{\"\":1}"),
        ("? [x\u{2029}y]\n: 1\n", "{\"\":1}"),
        ("? [x\u{2029} y]\n: 1\n", "{\"\":1}"),
        ("? [x\u{2029}  y]\n: 1\n", "{\"\":1}"),
        ("? [x\u{2029}  \ty]\n: 1\n", "{\"\":1}"),
        ("? [x\u{2029} \ty]\n: 1\n", "{\"\":1}"),
        ("? [x\u{85}y]\n: 1\n", "{\"\":1}"),
        ("? [x\u{85} y]\n: 1\n", "{\"\":1}"),
        ("? [x\u{85}  y]\n: 1\n", "{\"\":1}"),
        ("? [x\u{85}  \ty]\n: 1\n", "{\"\":1}"),
        ("? [x\u{85} \ty]\n: 1\n", "{\"\":1}"),
        (
            "a: [x\u{2028}\u{2028}y]\n",
            "{\"a\":[\"x\\u2028\\u2028y\"]}",
        ),
        (
            "a: [x\u{2028}\u{2028} y]\n",
            "{\"a\":[\"x\\u2028\\u2028y\"]}",
        ),
        (
            "a: [x\u{2028}\u{2028}  y]\n",
            "{\"a\":[\"x\\u2028\\u2028y\"]}",
        ),
        (
            "a: [x\u{2028}\u{2028}  \ty]\n",
            "{\"a\":[\"x\\u2028\\u2028y\"]}",
        ),
        (
            "a: [x\u{2028}\u{2028} \ty]\n",
            "{\"a\":[\"x\\u2028\\u2028y\"]}",
        ),
        (
            "a: [x\u{2029}\u{2029}y]\n",
            "{\"a\":[\"x\\u2029\\u2029y\"]}",
        ),
        (
            "a: [x\u{2029}\u{2029} y]\n",
            "{\"a\":[\"x\\u2029\\u2029y\"]}",
        ),
        (
            "a: [x\u{2029}\u{2029}  y]\n",
            "{\"a\":[\"x\\u2029\\u2029y\"]}",
        ),
        (
            "a: [x\u{2029}\u{2029}  \ty]\n",
            "{\"a\":[\"x\\u2029\\u2029y\"]}",
        ),
        (
            "a: [x\u{2029}\u{2029} \ty]\n",
            "{\"a\":[\"x\\u2029\\u2029y\"]}",
        ),
        ("a: [x\u{85}\u{85}y]\n", "{\"a\":[\"x\\ny\"]}"),
        ("a: [x\u{85}\u{85} y]\n", "{\"a\":[\"x\\ny\"]}"),
        ("a: [x\u{85}\u{85}  y]\n", "{\"a\":[\"x\\ny\"]}"),
        ("a: [x\u{85}\u{85}  \ty]\n", "{\"a\":[\"x\\ny\"]}"),
        ("a: [x\u{85}\u{85} \ty]\n", "{\"a\":[\"x\\ny\"]}"),
        ("- [x\u{2028}y]\n", "[[\"x\\u2028y\"]]"),
        ("- [x\u{2028} y]\n", "[[\"x\\u2028y\"]]"),
        ("- [x\u{2028}  y]\n", "[[\"x\\u2028y\"]]"),
        ("- [x\u{2028}  \ty]\n", "[[\"x\\u2028y\"]]"),
        ("- [x\u{2028} \ty]\n", "[[\"x\\u2028y\"]]"),
        ("- [x\u{2029}y]\n", "[[\"x\\u2029y\"]]"),
        ("- [x\u{2029} y]\n", "[[\"x\\u2029y\"]]"),
        ("- [x\u{2029}  y]\n", "[[\"x\\u2029y\"]]"),
        ("- [x\u{2029}  \ty]\n", "[[\"x\\u2029y\"]]"),
        ("- [x\u{2029} \ty]\n", "[[\"x\\u2029y\"]]"),
        ("- [x\u{85}y]\n", "[[\"x y\"]]"),
        ("- [x\u{85} y]\n", "[[\"x y\"]]"),
        ("- [x\u{85}  y]\n", "[[\"x y\"]]"),
        ("- [x\u{85}  \ty]\n", "[[\"x y\"]]"),
        ("- [x\u{85} \ty]\n", "[[\"x y\"]]"),
        ("- a: [x\u{2028}y]\n", "[{\"a\":[\"x\\u2028y\"]}]"),
        ("- a: [x\u{2028} y]\n", "[{\"a\":[\"x\\u2028y\"]}]"),
        ("- a: [x\u{2028}  y]\n", "[{\"a\":[\"x\\u2028y\"]}]"),
        ("- a: [x\u{2029}y]\n", "[{\"a\":[\"x\\u2029y\"]}]"),
        ("- a: [x\u{2029} y]\n", "[{\"a\":[\"x\\u2029y\"]}]"),
        ("- a: [x\u{2029}  y]\n", "[{\"a\":[\"x\\u2029y\"]}]"),
        ("- a: [x\u{85}y]\n", "[{\"a\":[\"x y\"]}]"),
        ("- a: [x\u{85} y]\n", "[{\"a\":[\"x y\"]}]"),
        ("- a: [x\u{85}  y]\n", "[{\"a\":[\"x y\"]}]"),
        ("a:\n  b: [x\u{2028}y]\n", "{\"a\":{\"b\":[\"x\\u2028y\"]}}"),
        (
            "a:\n  b: [x\u{2028} y]\n",
            "{\"a\":{\"b\":[\"x\\u2028y\"]}}",
        ),
        (
            "a:\n  b: [x\u{2028}  y]\n",
            "{\"a\":{\"b\":[\"x\\u2028y\"]}}",
        ),
        ("a:\n  b: [x\u{2029}y]\n", "{\"a\":{\"b\":[\"x\\u2029y\"]}}"),
        (
            "a:\n  b: [x\u{2029} y]\n",
            "{\"a\":{\"b\":[\"x\\u2029y\"]}}",
        ),
        (
            "a:\n  b: [x\u{2029}  y]\n",
            "{\"a\":{\"b\":[\"x\\u2029y\"]}}",
        ),
        ("a:\n  b: [x\u{85}y]\n", "{\"a\":{\"b\":[\"x y\"]}}"),
        ("a:\n  b: [x\u{85} y]\n", "{\"a\":{\"b\":[\"x y\"]}}"),
        ("a:\n  b: [x\u{85}  y]\n", "{\"a\":{\"b\":[\"x y\"]}}"),
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!((out.trim_end(), code), (expected, 0), "{input:?}");
    }
    Ok(())
}

// The block-context scanners (#6214): the same tab rule after a Unicode break in a block plain scalar
// or key. `REFUSED` also holds flow shapes, which pin that the two paths agree.

/// Each shape with each of U+2028, U+2029 and U+0085 for the break.
const REFUSED: &[&str] = &[
    "a: [x\u{2028}\ty]\n",
    "a: [x\u{2029}\ty]\n",
    "a: [x\u{85}\ty]\n",
    "a: x\u{2028}\ty\n",
    "a: x\u{2029}\ty\n",
    "a: x\u{85}\ty\n",
    "a:\n  b: x\u{2028}\ty\n",
    "a:\n  b: x\u{2029}\ty\n",
    "a:\n  b: x\u{85}\ty\n",
    "a:\n  b: x\u{2028}  \ty\n",
    "a:\n  b: x\u{2029}  \ty\n",
    "a:\n  b: x\u{85}  \ty\n",
    "a:\n  - x\u{2028}\ty\n",
    "a:\n  - x\u{2029}\ty\n",
    "a:\n  - x\u{85}\ty\n",
    "a:\n  - [x\u{2028}\ty]\n",
    "a:\n  - [x\u{2029}\ty]\n",
    "a:\n  - [x\u{85}\ty]\n",
    "- x\u{2028}\ty\n",
    "- x\u{2029}\ty\n",
    "- x\u{85}\ty\n",
    "- - x\u{2028}\ty\n",
    "- - x\u{2029}\ty\n",
    "- - x\u{85}\ty\n",
    "a: {k: x\u{2028}\ty}\n",
    "a: {k: x\u{2029}\ty}\n",
    "a: {k: x\u{85}\ty}\n",
    "? x\u{2028}\ty\n: 1\n",
    "? x\u{2029}\ty\n: 1\n",
    "? x\u{85}\ty\n: 1\n",
    "a: x\u{2028}y\u{2028}\tz\n",
    "a: x\u{2029}y\u{2029}\tz\n",
    "a: x\u{85}y\u{85}\tz\n",
    "a: x\u{2028}\u{2028}\ty\n",
    "a: x\u{2029}\u{2029}\ty\n",
    "a: x\u{85}\u{85}\ty\n",
    "a:\n  x\u{2028}\ty: 1\n",
    "a:\n  x\u{85}\ty: 1\n",
    "a: {x\u{2028}\ty: 1}\n",
    "a: {x\u{85}\ty: 1}\n",
    "a: [x\u{2028}\ty: 1]\n",
    "a: [x\u{85}\ty: 1]\n",
    "a: [? x\u{2028}\ty : 1]\n",
    "a: [? x\u{85}\ty : 1]\n",
    "a: x\u{2028}\t\nb: 1\n",
    "a: x\u{85}\t\nb: 1\n",
    "a: x\u{2028}\t# c\n",
    "a: x\u{85}\t# c\n",
    "a: [x\u{2028}\t]\n",
    "a: [x\u{85}\t]\n",
    "a:\n  b: x\u{2028}\ty\n  more\n",
    "a:\n  b: x\u{85}\ty\n  more\n",
];

#[test]
fn a_tab_after_a_unicode_break_below_the_block_indent_is_refused_6214() -> Result<()> {
    for input in REFUSED {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(out.trim_end(), "", "{input:?}");
        assert_ne!(code, 0, "{input:?}");
    }
    Ok(())
}

/// (input, yq's `-o=json -I=0` output): a tab past the block indent, a collection or scalar at the
/// document root, and a quoted scalar are not refused.
const ACCEPTED: &[(&str, &str)] = &[
    (
        "a:\n  b: x\u{2028}   \ty\n",
        "{\"a\":{\"b\":\"x\\u2028y\"}}",
    ),
    (
        "a:\n  b: x\u{2029}   \ty\n",
        "{\"a\":{\"b\":\"x\\u2029y\"}}",
    ),
    ("a:\n  b: x\u{85}   \ty\n", "{\"a\":{\"b\":\"x y\"}}"),
    ("a: [x\u{2028} \ty]\n", "{\"a\":[\"x\\u2028y\"]}"),
    ("a: [x\u{2029} \ty]\n", "{\"a\":[\"x\\u2029y\"]}"),
    ("a: [x\u{85} \ty]\n", "{\"a\":[\"x y\"]}"),
    ("a: x\u{2028} \ty\n", "{\"a\":\"x\\u2028y\"}"),
    ("a: x\u{2029} \ty\n", "{\"a\":\"x\\u2029y\"}"),
    ("a: x\u{85} \ty\n", "{\"a\":\"x y\"}"),
    ("[x\u{2028}\ty]\n", "[\"x\\u2028y\"]"),
    ("[x\u{2029}\ty]\n", "[\"x\\u2029y\"]"),
    ("[x\u{85}\ty]\n", "[\"x y\"]"),
    ("x\u{2028}\ty\n", "\"x\\u2028y\""),
    ("x\u{2029}\ty\n", "\"x\\u2029y\""),
    ("x\u{85}\ty\n", "\"x y\""),
    ("a: \"x\u{2028}\ty\"\n", "{\"a\":\"x\\u2028y\"}"),
    ("a: \"x\u{2029}\ty\"\n", "{\"a\":\"x\\u2029y\"}"),
    ("a: \"x\u{85}\ty\"\n", "{\"a\":\"x y\"}"),
];

#[test]
fn a_tab_after_a_unicode_break_clear_of_the_block_indent_is_accepted_6214() -> Result<()> {
    for (input, expected) in ACCEPTED {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(out.trim_end(), *expected, "{input:?}");
        assert_eq!(code, 0, "{input:?}");
    }
    Ok(())
}

/// The error names the line of the tab, not the line the scanner had reached.
#[test]
fn the_error_names_the_line_of_the_tab_6214() -> Result<()> {
    let input = "a: p\n  q\u{2028}\tz\n  r\n  s\n";
    let (stdout, stderr, code) = crate::run_yq_stdin_with_stderr(".", input, &[])?;
    assert_ne!(code, 0, "{stdout:?}");
    assert!(stderr.contains("line 2"), "{stderr:?}");
    Ok(())
}
