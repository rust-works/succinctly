//! #6413: node properties on a line of their own name the node that follows, which needs an owner
//! like any other. yq refuses a properties-only line (`!!map`, `&a`) in the place of a sibling, level
//! with the keys or items of a block mapping or sequence and not under a `key:` or `-` that waits for its
//! value, where this attached them to nothing and went on (`k: |` / `!!map`, `a: 1` / `&a`,
//! `- a` / `!!str`). A document's own properties (`!!map` / `k: 1`), the properties of a value
//! (`a:` / `  !!map` / `  b: 1`) and properties that lead a key (`a: 1` / `&a b: 2`) are fine.
//!
//! A `-` followed by a blank in a flow collection is the block sequence indicator, which go-yaml
//! refuses (`[- x]`, `{a: - x}`) and no plain scalar starts with, where this read `- x` as text.
//! Every row was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const JSON: &[&str] = &["-o=json", "-I=0"];

#[test]
fn a_properties_only_line_with_no_owner_and_a_dash_in_a_flow_collection_are_refused_6413(
) -> Result<()> {
    for doc in [
        "key: |\n!!map\n  cont\n",
        "key: |\n&a\n  cont\n",
        "key: |\n!!str\n",
        "key: >\n!!str\n  x\n",
        "key: |\n  x\n!!str\n",
        "k: !!str\n&a\n",
        "k: &a\n!!str\n",
        "a: !!str\n&b\n",
        "k: !!str\n&a x\n",
        "k:\n!!str\n",
        "k:\n&a\n",
        "k:\n!!str x\n",
        "- !!str\n&a\n",
        "- &a\n!!str\n",
        "- |\n!!str\n  x\n",
        "- k: |\n!!map\n  c\n",
        "a: 1\n!!str\n",
        "a: 1\n&a\n",
        "a: 1\n!!str\nb: 2\n",
        "a: 1\n!!str\n  x\n",
        "a: 1\n&a\n&b\nb: 2\n",
        "a: 1\n!!map\n  b: 2\n",
        "a: 1\n&a\n? b\n",
        "a:\n  b: 1\n  &a\n  c: 2\n",
        "a:\n  b: 1\n  !!str\n",
        "a:\n  b: 1\n&a\nc: 2\n",
        "a:\n  b: 1\n!!str\n",
        "- a\n!!str\n",
        "- a\n&a\n- b\n",
        "- a\n&a\n",
        "b: 1\nk: !!str\n&a\n",
        "k: !!str\n&a\nb: 1\n",
        "k:\n  - a\n  !!str\n",
        "k:\n  - a\n!!str\n",
        "? !!str\n&a\n: v\n",
        "? a\n!!str\n: v\n",
        "k: !!str\n# c\n&a\nb: 1\n",
        "k:\n!!str # c\n",
        "[- x]\n",
        "[a, - x]\n",
        "{a: - x}\n",
        "[1,\n- x\n 2]\n",
        "[1,\n  - x\n 2]\n",
        "b: [1,\n  - x\n  2]\n",
        "[a, - ]\n",
        "[a, -\n x]\n",
        "{- a: 1}\n",
        "{a: 1, - b: 2}\n",
        "[a: - x]\n",
        "[? - x]\n",
        "[-1, - 2]\n",
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, JSON)?;
        assert_ne!(code, 0, "{doc:?} is refused");
        assert_eq!(stdout, "", "{doc:?} prints nothing");
    }
    Ok(())
}

#[test]
fn properties_that_have_an_owner_and_a_dash_that_is_text_are_still_read_6413() -> Result<()> {
    for (doc, expected) in [
        ("a:\n  !!map\n  b: 1\n", "{\"a\":{\"b\":1}}\n"),
        ("a:\n  &x\n  b: 1\n", "{\"a\":{\"b\":1}}\n"),
        ("a:\n  &x\n  - b\n", "{\"a\":[\"b\"]}\n"),
        ("a:\n  !!seq\n  - b\n", "{\"a\":[\"b\"]}\n"),
        ("- !!map\n  a: 1\n", "[{\"a\":1}]\n"),
        ("-\n  &x\n  a: 1\n", "[{\"a\":1}]\n"),
        ("-\n  !!str\n  x\n", "[\"x\"]\n"),
        ("k: !!str\n  &a\n  x\n", "{\"k\":\"x\"}\n"),
        ("k: !!str\n  x\n", "{\"k\":\"x\"}\n"),
        ("k: &a\n  x\n", "{\"k\":\"x\"}\n"),
        ("k: !!str\n- x\n", "{\"k\":[\"x\"]}\n"),
        ("k: &a\n- x\n", "{\"k\":[\"x\"]}\n"),
        ("k: !!str\nb: 1\n", "{\"k\":\"\",\"b\":1}\n"),
        ("k: !!str\n&a: 1\n", "{\"k\":\"\",\"\":1}\n"),
        ("!!map\nk: 1\n", "{\"k\":1}\n"),
        ("&a\nk: 1\n", "{\"k\":1}\n"),
        ("!!str\nx\n", "\"x\"\n"),
        ("!!str\n&a\nx\n", "\"x\"\n"),
        ("--- !!map\nk: 1\n", "\"1\"\n"),
        ("---\n!!map\nk: 1\n", "{\"k\":1}\n"),
        ("---\n!!map\n&a\nk: 1\n", "{\"k\":1}\n"),
        ("a: 1\n---\n!!map\nk: 1\n", "{\"a\":1}\n{\"k\":1}\n"),
        ("a: 1\n&a b: 2\n", "{\"a\":1,\"b\":2}\n"),
        ("a: 1\n!!str b: 2\n", "{\"a\":1,\"b\":2}\n"),
        ("[a, -x]\n", "[\"a\",\"-x\"]\n"),
        ("[a,\n-x\n]\n", "[\"a\",\"-x\"]\n"),
        ("[a, -]\n", "[\"a\",\"-\"]\n"),
        ("[-]\n", "[\"-\"]\n"),
        ("[a, \"- x\"]\n", "[\"a\",\"- x\"]\n"),
        ("[a, '- x']\n", "[\"a\",\"- x\"]\n"),
        ("a: -x\n", "{\"a\":\"-x\"}\n"),
        ("[ -1 ]\n", "[-1]\n"),
        ("[-- x]\n", "[\"-- x\"]\n"),
        ("[a,-b]\n", "[\"a\",\"-b\"]\n"),
        ("[1,\n!!str\n 2]\n", "[1,\"2\"]\n"),
        ("b: [1,\n!!str\n  2]\n", "{\"b\":[1,\"2\"]}\n"),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, JSON)?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{doc:?}");
    }
    Ok(())
}
