//! #5257: a plain assignment from a static read (`.c = .b`) copies the source scalar's quoting and
//! explicit tag, and a container's tag, as yq's `Set` does. The source's style is taken when the
//! target is new, is not a scalar, is plain with no tag, or changes type; otherwise the target
//! keeps its own (`c: "old"` stays double-quoted). A custom tag already on the target stays.
//! A flow container copied onto a scalar or flow target keeps the target's same-line comment.
//!
//! Every output is captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, `yq '.c = .b'` output).
const ROWS: &[(&str, &str)] = &[
    ("b: q\n", "b: q\nc: q\n"),
    ("b: q\nc: old\n", "b: q\nc: q\n"),
    ("b: q\nc: 5\n", "b: q\nc: q\n"),
    ("b: q\nc: \"old\"\n", "b: q\nc: \"q\"\n"),
    ("b: q\nc: 'old'\n", "b: q\nc: 'q'\n"),
    ("b: q\nc: !!int 5\n", "b: q\nc: q\n"),
    ("b: q\nc: !foo old\n", "b: q\nc: !foo q\n"),
    ("b: q\nc: [9]\n", "b: q\nc: q\n"),
    ("b: 7\n", "b: 7\nc: 7\n"),
    ("b: 7\nc: old\n", "b: 7\nc: 7\n"),
    ("b: 7\nc: 5\n", "b: 7\nc: 7\n"),
    ("b: 7\nc: \"old\"\n", "b: 7\nc: 7\n"),
    ("b: 7\nc: 'old'\n", "b: 7\nc: 7\n"),
    ("b: 7\nc: !!int 5\n", "b: 7\nc: !!int 7\n"),
    ("b: 7\nc: !foo old\n", "b: 7\nc: !foo 7\n"),
    ("b: 7\nc: [9]\n", "b: 7\nc: 7\n"),
    ("b: \"q\"\n", "b: \"q\"\nc: \"q\"\n"),
    ("b: \"q\"\nc: old\n", "b: \"q\"\nc: \"q\"\n"),
    ("b: \"q\"\nc: 5\n", "b: \"q\"\nc: \"q\"\n"),
    ("b: \"q\"\nc: \"old\"\n", "b: \"q\"\nc: \"q\"\n"),
    ("b: \"q\"\nc: 'old'\n", "b: \"q\"\nc: 'q'\n"),
    ("b: \"q\"\nc: !!int 5\n", "b: \"q\"\nc: \"q\"\n"),
    ("b: \"q\"\nc: !foo old\n", "b: \"q\"\nc: !foo q\n"),
    ("b: \"q\"\nc: [9]\n", "b: \"q\"\nc: \"q\"\n"),
    ("b: \"7\"\n", "b: \"7\"\nc: \"7\"\n"),
    ("b: \"7\"\nc: old\n", "b: \"7\"\nc: \"7\"\n"),
    ("b: \"7\"\nc: 5\n", "b: \"7\"\nc: \"7\"\n"),
    ("b: \"7\"\nc: \"old\"\n", "b: \"7\"\nc: \"7\"\n"),
    ("b: \"7\"\nc: 'old'\n", "b: \"7\"\nc: '7'\n"),
    ("b: \"7\"\nc: !!int 5\n", "b: \"7\"\nc: \"7\"\n"),
    ("b: \"7\"\nc: !foo old\n", "b: \"7\"\nc: !foo 7\n"),
    ("b: \"7\"\nc: [9]\n", "b: \"7\"\nc: \"7\"\n"),
    ("b: 'q'\n", "b: 'q'\nc: 'q'\n"),
    ("b: 'q'\nc: old\n", "b: 'q'\nc: 'q'\n"),
    ("b: 'q'\nc: 5\n", "b: 'q'\nc: 'q'\n"),
    ("b: 'q'\nc: \"old\"\n", "b: 'q'\nc: \"q\"\n"),
    ("b: 'q'\nc: 'old'\n", "b: 'q'\nc: 'q'\n"),
    ("b: 'q'\nc: !!int 5\n", "b: 'q'\nc: 'q'\n"),
    ("b: 'q'\nc: !foo old\n", "b: 'q'\nc: !foo q\n"),
    ("b: 'q'\nc: [9]\n", "b: 'q'\nc: 'q'\n"),
    ("b: !!str q\n", "b: !!str q\nc: !!str q\n"),
    ("b: !!str q\nc: old\n", "b: !!str q\nc: !!str q\n"),
    ("b: !!str q\nc: 5\n", "b: !!str q\nc: !!str q\n"),
    ("b: !!str q\nc: \"old\"\n", "b: !!str q\nc: \"q\"\n"),
    ("b: !!str q\nc: 'old'\n", "b: !!str q\nc: 'q'\n"),
    ("b: !!str q\nc: !!int 5\n", "b: !!str q\nc: !!str q\n"),
    ("b: !!str q\nc: !foo old\n", "b: !!str q\nc: !foo q\n"),
    ("b: !!str q\nc: [9]\n", "b: !!str q\nc: !!str q\n"),
    ("b: !!str 7\n", "b: !!str 7\nc: !!str 7\n"),
    ("b: !!str 7\nc: old\n", "b: !!str 7\nc: !!str 7\n"),
    ("b: !!str 7\nc: 5\n", "b: !!str 7\nc: !!str 7\n"),
    ("b: !!str 7\nc: \"old\"\n", "b: !!str 7\nc: \"7\"\n"),
    ("b: !!str 7\nc: 'old'\n", "b: !!str 7\nc: '7'\n"),
    ("b: !!str 7\nc: !!int 5\n", "b: !!str 7\nc: !!str 7\n"),
    ("b: !!str 7\nc: !foo old\n", "b: !!str 7\nc: !foo 7\n"),
    ("b: !!str 7\nc: [9]\n", "b: !!str 7\nc: !!str 7\n"),
    ("b: !!str \"q\"\n", "b: !!str \"q\"\nc: !!str \"q\"\n"),
    (
        "b: !!str \"q\"\nc: old\n",
        "b: !!str \"q\"\nc: !!str \"q\"\n",
    ),
    ("b: !!str \"q\"\nc: 5\n", "b: !!str \"q\"\nc: !!str \"q\"\n"),
    ("b: !!str \"q\"\nc: \"old\"\n", "b: !!str \"q\"\nc: \"q\"\n"),
    ("b: !!str \"q\"\nc: 'old'\n", "b: !!str \"q\"\nc: 'q'\n"),
    (
        "b: !!str \"q\"\nc: !!int 5\n",
        "b: !!str \"q\"\nc: !!str \"q\"\n",
    ),
    (
        "b: !!str \"q\"\nc: !foo old\n",
        "b: !!str \"q\"\nc: !foo q\n",
    ),
    (
        "b: !!str \"q\"\nc: [9]\n",
        "b: !!str \"q\"\nc: !!str \"q\"\n",
    ),
    ("b: !!int 7\n", "b: !!int 7\nc: !!int 7\n"),
    ("b: !!int 7\nc: old\n", "b: !!int 7\nc: !!int 7\n"),
    ("b: !!int 7\nc: 5\n", "b: !!int 7\nc: !!int 7\n"),
    ("b: !!int 7\nc: \"old\"\n", "b: !!int 7\nc: !!int 7\n"),
    ("b: !!int 7\nc: 'old'\n", "b: !!int 7\nc: !!int 7\n"),
    ("b: !!int 7\nc: !!int 5\n", "b: !!int 7\nc: !!int 7\n"),
    ("b: !!int 7\nc: !foo old\n", "b: !!int 7\nc: !foo 7\n"),
    ("b: !!int 7\nc: [9]\n", "b: !!int 7\nc: !!int 7\n"),
    ("b: !!int \"7\"\n", "b: !!int \"7\"\nc: !!int \"7\"\n"),
    (
        "b: !!int \"7\"\nc: old\n",
        "b: !!int \"7\"\nc: !!int \"7\"\n",
    ),
    ("b: !!int \"7\"\nc: 5\n", "b: !!int \"7\"\nc: !!int \"7\"\n"),
    (
        "b: !!int \"7\"\nc: \"old\"\n",
        "b: !!int \"7\"\nc: !!int \"7\"\n",
    ),
    (
        "b: !!int \"7\"\nc: 'old'\n",
        "b: !!int \"7\"\nc: !!int \"7\"\n",
    ),
    (
        "b: !!int \"7\"\nc: !!int 5\n",
        "b: !!int \"7\"\nc: !!int 7\n",
    ),
    (
        "b: !!int \"7\"\nc: !foo old\n",
        "b: !!int \"7\"\nc: !foo \"7\"\n",
    ),
    (
        "b: !!int \"7\"\nc: [9]\n",
        "b: !!int \"7\"\nc: !!int \"7\"\n",
    ),
    ("b: !foo x\n", "b: !foo x\nc: !foo x\n"),
    ("b: !foo x\nc: old\n", "b: !foo x\nc: !foo x\n"),
    ("b: !foo x\nc: 5\n", "b: !foo x\nc: !foo x\n"),
    ("b: !foo x\nc: \"old\"\n", "b: !foo x\nc: !foo \"x\"\n"),
    ("b: !foo x\nc: 'old'\n", "b: !foo x\nc: !foo 'x'\n"),
    ("b: !foo x\nc: !!int 5\n", "b: !foo x\nc: !foo x\n"),
    ("b: !foo x\nc: !foo old\n", "b: !foo x\nc: !foo x\n"),
    ("b: !foo x\nc: [9]\n", "b: !foo x\nc: !foo x\n"),
    ("b: !foo \"x\"\n", "b: !foo \"x\"\nc: !foo \"x\"\n"),
    ("b: !foo \"x\"\nc: old\n", "b: !foo \"x\"\nc: !foo \"x\"\n"),
    ("b: !foo \"x\"\nc: 5\n", "b: !foo \"x\"\nc: !foo \"x\"\n"),
    (
        "b: !foo \"x\"\nc: \"old\"\n",
        "b: !foo \"x\"\nc: !foo \"x\"\n",
    ),
    ("b: !foo \"x\"\nc: 'old'\n", "b: !foo \"x\"\nc: !foo 'x'\n"),
    (
        "b: !foo \"x\"\nc: !!int 5\n",
        "b: !foo \"x\"\nc: !foo \"x\"\n",
    ),
    ("b: !foo \"x\"\nc: !foo old\n", "b: !foo \"x\"\nc: !foo x\n"),
    ("b: !foo \"x\"\nc: [9]\n", "b: !foo \"x\"\nc: !foo \"x\"\n"),
    ("b: !!bool true\n", "b: !!bool true\nc: !!bool true\n"),
    (
        "b: !!bool true\nc: old\n",
        "b: !!bool true\nc: !!bool true\n",
    ),
    ("b: !!bool true\nc: 5\n", "b: !!bool true\nc: !!bool true\n"),
    (
        "b: !!bool true\nc: \"old\"\n",
        "b: !!bool true\nc: !!bool true\n",
    ),
    (
        "b: !!bool true\nc: 'old'\n",
        "b: !!bool true\nc: !!bool true\n",
    ),
    (
        "b: !!bool true\nc: !!int 5\n",
        "b: !!bool true\nc: !!bool true\n",
    ),
    (
        "b: !!bool true\nc: !foo old\n",
        "b: !!bool true\nc: !foo true\n",
    ),
    (
        "b: !!bool true\nc: [9]\n",
        "b: !!bool true\nc: !!bool true\n",
    ),
    ("b: \"true\"\n", "b: \"true\"\nc: \"true\"\n"),
    ("b: \"true\"\nc: old\n", "b: \"true\"\nc: \"true\"\n"),
    ("b: \"true\"\nc: 5\n", "b: \"true\"\nc: \"true\"\n"),
    ("b: \"true\"\nc: \"old\"\n", "b: \"true\"\nc: \"true\"\n"),
    ("b: \"true\"\nc: 'old'\n", "b: \"true\"\nc: 'true'\n"),
    ("b: \"true\"\nc: !!int 5\n", "b: \"true\"\nc: \"true\"\n"),
    ("b: \"true\"\nc: !foo old\n", "b: \"true\"\nc: !foo true\n"),
    ("b: \"true\"\nc: [9]\n", "b: \"true\"\nc: \"true\"\n"),
    ("b: !!seq [1]\n", "b: !!seq [1]\nc: !!seq [1]\n"),
    ("b: !!seq [1]\nc: [9]\n", "b: !!seq [1]\nc: [1]\n"),
    ("b: !!seq [1]\nc: {y: 2}\n", "b: !!seq [1]\nc: !!seq [1]\n"),
    (
        "b: !!seq [1]\nc: !!seq [9]\n",
        "b: !!seq [1]\nc: !!seq [1]\n",
    ),
    ("b: !!seq [1]\nc: !bar [9]\n", "b: !!seq [1]\nc: !bar [1]\n"),
    (
        "b: !!seq [1]\nc: x # tc\n",
        "b: !!seq [1]\nc: !!seq [1] # tc\n",
    ),
    ("b: !!seq [1]\nc: []\n", "b: !!seq [1]\nc: !!seq [1]\n"),
    ("b: !!map {x: 1}\n", "b: !!map {x: 1}\nc: !!map {x: 1}\n"),
    (
        "b: !!map {x: 1}\nc: [9]\n",
        "b: !!map {x: 1}\nc: !!map {x: 1}\n",
    ),
    (
        "b: !!map {x: 1}\nc: {y: 2}\n",
        "b: !!map {x: 1}\nc: {x: 1}\n",
    ),
    (
        "b: !!map {x: 1}\nc: !!seq [9]\n",
        "b: !!map {x: 1}\nc: !!map {x: 1}\n",
    ),
    (
        "b: !!map {x: 1}\nc: !bar [9]\n",
        "b: !!map {x: 1}\nc: !bar {x: 1}\n",
    ),
    (
        "b: !!map {x: 1}\nc: x # tc\n",
        "b: !!map {x: 1}\nc: !!map {x: 1} # tc\n",
    ),
    (
        "b: !!map {x: 1}\nc: []\n",
        "b: !!map {x: 1}\nc: !!map {x: 1}\n",
    ),
    ("b: !foo [1]\n", "b: !foo [1]\nc: !foo [1]\n"),
    ("b: !foo [1]\nc: [9]\n", "b: !foo [1]\nc: !foo [1]\n"),
    ("b: !foo [1]\nc: {y: 2}\n", "b: !foo [1]\nc: !foo [1]\n"),
    ("b: !foo [1]\nc: !!seq [9]\n", "b: !foo [1]\nc: !foo [1]\n"),
    ("b: !foo [1]\nc: !bar [9]\n", "b: !foo [1]\nc: !bar [1]\n"),
    (
        "b: !foo [1]\nc: x # tc\n",
        "b: !foo [1]\nc: !foo [1] # tc\n",
    ),
    ("b: !foo [1]\nc: []\n", "b: !foo [1]\nc: !foo [1]\n"),
    ("b: !foo {x: 1}\n", "b: !foo {x: 1}\nc: !foo {x: 1}\n"),
    (
        "b: !foo {x: 1}\nc: [9]\n",
        "b: !foo {x: 1}\nc: !foo {x: 1}\n",
    ),
    (
        "b: !foo {x: 1}\nc: {y: 2}\n",
        "b: !foo {x: 1}\nc: !foo {x: 1}\n",
    ),
    (
        "b: !foo {x: 1}\nc: !!seq [9]\n",
        "b: !foo {x: 1}\nc: !foo {x: 1}\n",
    ),
    (
        "b: !foo {x: 1}\nc: !bar [9]\n",
        "b: !foo {x: 1}\nc: !bar {x: 1}\n",
    ),
    (
        "b: !foo {x: 1}\nc: x # tc\n",
        "b: !foo {x: 1}\nc: !foo {x: 1} # tc\n",
    ),
    (
        "b: !foo {x: 1}\nc: []\n",
        "b: !foo {x: 1}\nc: !foo {x: 1}\n",
    ),
    ("b: [1]\n", "b: [1]\nc: [1]\n"),
    ("b: [1]\nc: [9]\n", "b: [1]\nc: [1]\n"),
    ("b: [1]\nc: {y: 2}\n", "b: [1]\nc: [1]\n"),
    ("b: [1]\nc: !!seq [9]\n", "b: [1]\nc: !!seq [1]\n"),
    ("b: [1]\nc: !bar [9]\n", "b: [1]\nc: !bar [1]\n"),
    ("b: [1]\nc: x # tc\n", "b: [1]\nc: [1] # tc\n"),
    ("b: [1]\nc: []\n", "b: [1]\nc: [1]\n"),
    ("b: {x: 1}\n", "b: {x: 1}\nc: {x: 1}\n"),
    ("b: {x: 1}\nc: [9]\n", "b: {x: 1}\nc: {x: 1}\n"),
    ("b: {x: 1}\nc: {y: 2}\n", "b: {x: 1}\nc: {x: 1}\n"),
    ("b: {x: 1}\nc: !!seq [9]\n", "b: {x: 1}\nc: {x: 1}\n"),
    ("b: {x: 1}\nc: !bar [9]\n", "b: {x: 1}\nc: !bar {x: 1}\n"),
    ("b: {x: 1}\nc: x # tc\n", "b: {x: 1}\nc: {x: 1} # tc\n"),
    ("b: {x: 1}\nc: []\n", "b: {x: 1}\nc: {x: 1}\n"),
    ("b: !!seq\n  - 1\n", "b: !!seq\n  - 1\nc: !!seq\n  - 1\n"),
    ("b: !!seq\n  - 1\nc: [9]\n", "b: !!seq\n  - 1\nc: [1]\n"),
    (
        "b: !!seq\n  - 1\nc: {y: 2}\n",
        "b: !!seq\n  - 1\nc: !!seq\n  - 1\n",
    ),
    (
        "b: !!seq\n  - 1\nc: !!seq [9]\n",
        "b: !!seq\n  - 1\nc: !!seq [1]\n",
    ),
    (
        "b: !!seq\n  - 1\nc: !bar [9]\n",
        "b: !!seq\n  - 1\nc: !bar\n  - 1\n",
    ),
    (
        "b: !!seq\n  - 1\nc: x # tc\n",
        "b: !!seq\n  - 1\nc: !!seq\n  - 1\n",
    ),
    (
        "b: !!seq\n  - 1\nc: []\n",
        "b: !!seq\n  - 1\nc: !!seq\n  - 1\n",
    ),
    ("b: !foo\n  x: 1\n", "b: !foo\n  x: 1\nc: !foo\n  x: 1\n"),
    (
        "b: !foo\n  x: 1\nc: [9]\n",
        "b: !foo\n  x: 1\nc: !foo\n  x: 1\n",
    ),
    (
        "b: !foo\n  x: 1\nc: {y: 2}\n",
        "b: !foo\n  x: 1\nc: !foo\n  x: 1\n",
    ),
    (
        "b: !foo\n  x: 1\nc: !!seq [9]\n",
        "b: !foo\n  x: 1\nc: !foo\n  x: 1\n",
    ),
    (
        "b: !foo\n  x: 1\nc: !bar [9]\n",
        "b: !foo\n  x: 1\nc: !bar\n  x: 1\n",
    ),
    (
        "b: !foo\n  x: 1\nc: x # tc\n",
        "b: !foo\n  x: 1\nc: !foo\n  x: 1\n",
    ),
    (
        "b: !foo\n  x: 1\nc: []\n",
        "b: !foo\n  x: 1\nc: !foo\n  x: 1\n",
    ),
    ("b: >\n  q\n", "b: >\n  q\n\nc: >\n  q\n\n"),
    ("b: >\n  q\nc: [9]\n", "b: >\n  q\n\nc: >\n  q\n\n"),
    ("b: >\n  q\nc: {y: 2}\n", "b: >\n  q\n\nc: >\n  q\n\n"),
    ("b: >\n  q\nc: !!seq [9]\n", "b: >\n  q\n\nc: >\n  q\n\n"),
    (
        "b: >\n  q\nc: !bar [9]\n",
        "b: >\n  q\n\nc: !bar >\n  q\n\n",
    ),
    ("b: >\n  q\nc: x # tc\n", "b: >\n  q\n\nc: > # tc\n  q\n\n"),
    ("b: >\n  q\nc: []\n", "b: >\n  q\n\nc: >\n  q\n\n"),
    ("b: |-\n  q\n  r\n", "b: |-\n  q\n  r\nc: |-\n  q\n  r\n"),
    (
        "b: |-\n  q\n  r\nc: [9]\n",
        "b: |-\n  q\n  r\nc: |-\n  q\n  r\n",
    ),
    (
        "b: |-\n  q\n  r\nc: {y: 2}\n",
        "b: |-\n  q\n  r\nc: |-\n  q\n  r\n",
    ),
    (
        "b: |-\n  q\n  r\nc: !!seq [9]\n",
        "b: |-\n  q\n  r\nc: |-\n  q\n  r\n",
    ),
    (
        "b: |-\n  q\n  r\nc: !bar [9]\n",
        "b: |-\n  q\n  r\nc: !bar |-\n  q\n  r\n",
    ),
    (
        "b: |-\n  q\n  r\nc: x # tc\n",
        "b: |-\n  q\n  r\nc: |- # tc\n  q\n  r\n",
    ),
    (
        "b: |-\n  q\n  r\nc: []\n",
        "b: |-\n  q\n  r\nc: |-\n  q\n  r\n",
    ),
];

#[test]
fn test_5257_a_a_copy_takes_the_sources_quoting_and_tag() -> Result<()> {
    for (input, want) in ROWS {
        let (out, code) = run_yq_stdin(".c = .b", input, &[])?;
        assert_eq!(code, 0, "{input:?}");
        assert_eq!(&out, want, "{input:?}");
    }
    Ok(())
}

/// The value never changes with the presentation: `-o=json` is the same with or without the tag.
#[test]
fn test_5257_b_json_output_is_unchanged() -> Result<()> {
    let (out, code) = run_yq_stdin(".c = .b", "b: !!str 5\nc: 1\n", &["-o=json", "-I=0"])?;
    assert_eq!((out.as_str(), code), ("{\"b\":\"5\",\"c\":\"5\"}\n", 0));
    Ok(())
}
