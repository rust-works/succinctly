//! #6390: a block scalar header on a line of its own, at the indent of an empty `key:`, `:` or `-`
//! above it, is that node's value. A header cannot be a key or an item, so yq reads
//! `a:` / `|` / `  t` as `{"a": "t\n"}` and `-` / `|` / `  t` as `["t\n"]`; `succinctly yq` took the
//! node for null at four places that each decide "null or on the next line" (the explicit value
//! after `:`, a bare dash, a dash with properties, and a key with properties), where #6388 had
//! settled only the key. Now one helper (`Parser::header_at_indent`) answers it for all of them.
//! Every value was captured from yq v4.53.3; the grid behind them (8 owners, 4 property forms,
//! 4 headers: 128 documents) has no difference in value or verdict.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_block_scalar_header_at_the_indent_of_an_empty_node_is_its_value_6390() -> Result<()> {
    for (input, expected) in [
        ("? a\n:\n|\n  t\n", "{\"a\":\"t\\n\"}"),
        ("? a\n:\n>+\n  t\n", "{\"a\":\"t\\n\"}"),
        ("? a\n: &y\n|\n  t\n", "{\"a\":\"t\\n\"}"),
        ("? a\n: &y\n>+\n  t\n", "{\"a\":\"t\\n\"}"),
        ("? a\n: !!str\n|\n  t\n", "{\"a\":\"t\\n\"}"),
        ("? a\n: !!str\n>+\n  t\n", "{\"a\":\"t\\n\"}"),
        ("? a\n: &y !!str\n|\n  t\n", "{\"a\":\"t\\n\"}"),
        ("? a\n: &y !!str\n>+\n  t\n", "{\"a\":\"t\\n\"}"),
        ("x:\n  ? a\n  :\n  |\n    t\n", "{\"x\":{\"a\":\"t\\n\"}}"),
        ("x:\n  ? a\n  :\n  >+\n    t\n", "{\"x\":{\"a\":\"t\\n\"}}"),
        (
            "x:\n  ? a\n  : &y\n  |\n    t\n",
            "{\"x\":{\"a\":\"t\\n\"}}",
        ),
        (
            "x:\n  ? a\n  : &y\n  >+\n    t\n",
            "{\"x\":{\"a\":\"t\\n\"}}",
        ),
        (
            "x:\n  ? a\n  : !!str\n  |\n    t\n",
            "{\"x\":{\"a\":\"t\\n\"}}",
        ),
        (
            "x:\n  ? a\n  : !!str\n  >+\n    t\n",
            "{\"x\":{\"a\":\"t\\n\"}}",
        ),
        (
            "x:\n  ? a\n  : &y !!str\n  |\n    t\n",
            "{\"x\":{\"a\":\"t\\n\"}}",
        ),
        (
            "x:\n  ? a\n  : &y !!str\n  >+\n    t\n",
            "{\"x\":{\"a\":\"t\\n\"}}",
        ),
        ("-\n|\n  t\n", "[\"t\\n\"]"),
        ("-\n>+\n  t\n", "[\"t\\n\"]"),
        ("- &y\n|\n  t\n", "[\"t\\n\"]"),
        ("- &y\n>+\n  t\n", "[\"t\\n\"]"),
        ("- !!str\n|\n  t\n", "[\"t\\n\"]"),
        ("- !!str\n>+\n  t\n", "[\"t\\n\"]"),
        ("- &y !!str\n|\n  t\n", "[\"t\\n\"]"),
        ("- &y !!str\n>+\n  t\n", "[\"t\\n\"]"),
        ("a:\n  -\n  |\n    t\n", "{\"a\":[\"t\\n\"]}"),
        ("a:\n  -\n  >+\n    t\n", "{\"a\":[\"t\\n\"]}"),
        ("a:\n  - &y\n  |\n    t\n", "{\"a\":[\"t\\n\"]}"),
        ("a:\n  - &y\n  >+\n    t\n", "{\"a\":[\"t\\n\"]}"),
        ("a:\n  - !!str\n  |\n    t\n", "{\"a\":[\"t\\n\"]}"),
        ("a:\n  - !!str\n  >+\n    t\n", "{\"a\":[\"t\\n\"]}"),
        ("a:\n  - &y !!str\n  |\n    t\n", "{\"a\":[\"t\\n\"]}"),
        ("a:\n  - &y !!str\n  >+\n    t\n", "{\"a\":[\"t\\n\"]}"),
        ("-\n|\n  t\n- b\n", "[\"t\\n\",\"b\"]"),
        ("-\n>+\n  t\n- b\n", "[\"t\\n\",\"b\"]"),
        ("- &y\n|\n  t\n- b\n", "[\"t\\n\",\"b\"]"),
        ("- &y\n>+\n  t\n- b\n", "[\"t\\n\",\"b\"]"),
        ("- !!str\n|\n  t\n- b\n", "[\"t\\n\",\"b\"]"),
        ("- !!str\n>+\n  t\n- b\n", "[\"t\\n\",\"b\"]"),
        ("- &y !!str\n|\n  t\n- b\n", "[\"t\\n\",\"b\"]"),
        ("- &y !!str\n>+\n  t\n- b\n", "[\"t\\n\",\"b\"]"),
        ("k:\n|\n  t\nm: 1\n", "{\"k\":\"t\\n\",\"m\":1}"),
        ("k:\n>+\n  t\nm: 1\n", "{\"k\":\"t\\n\",\"m\":1}"),
        ("k: &y\n|\n  t\nm: 1\n", "{\"k\":\"t\\n\",\"m\":1}"),
        ("k: &y\n>+\n  t\nm: 1\n", "{\"k\":\"t\\n\",\"m\":1}"),
        ("k: !!str\n|\n  t\nm: 1\n", "{\"k\":\"t\\n\",\"m\":1}"),
        ("k: !!str\n>+\n  t\nm: 1\n", "{\"k\":\"t\\n\",\"m\":1}"),
        ("k: &y !!str\n|\n  t\nm: 1\n", "{\"k\":\"t\\n\",\"m\":1}"),
        ("k: &y !!str\n>+\n  t\nm: 1\n", "{\"k\":\"t\\n\",\"m\":1}"),
        (
            "a:\n  k:\n  |\n    t\n  m: 1\n",
            "{\"a\":{\"k\":\"t\\n\",\"m\":1}}",
        ),
        (
            "a:\n  k:\n  >+\n    t\n  m: 1\n",
            "{\"a\":{\"k\":\"t\\n\",\"m\":1}}",
        ),
        (
            "a:\n  k: &y\n  |\n    t\n  m: 1\n",
            "{\"a\":{\"k\":\"t\\n\",\"m\":1}}",
        ),
        (
            "a:\n  k: &y\n  >+\n    t\n  m: 1\n",
            "{\"a\":{\"k\":\"t\\n\",\"m\":1}}",
        ),
        (
            "a:\n  k: !!str\n  |\n    t\n  m: 1\n",
            "{\"a\":{\"k\":\"t\\n\",\"m\":1}}",
        ),
        (
            "a:\n  k: !!str\n  >+\n    t\n  m: 1\n",
            "{\"a\":{\"k\":\"t\\n\",\"m\":1}}",
        ),
        (
            "a:\n  k: &y !!str\n  |\n    t\n  m: 1\n",
            "{\"a\":{\"k\":\"t\\n\",\"m\":1}}",
        ),
        (
            "a:\n  k: &y !!str\n  >+\n    t\n  m: 1\n",
            "{\"a\":{\"k\":\"t\\n\",\"m\":1}}",
        ),
        ("- k:\n  |\n    t\n", "[{\"k\":\"t\\n\"}]"),
        ("- k:\n  >+\n    t\n", "[{\"k\":\"t\\n\"}]"),
        ("- k: &y\n  |\n    t\n", "[{\"k\":\"t\\n\"}]"),
        ("- k: &y\n  >+\n    t\n", "[{\"k\":\"t\\n\"}]"),
        ("- k: !!str\n  |\n    t\n", "[{\"k\":\"t\\n\"}]"),
        ("- k: !!str\n  >+\n    t\n", "[{\"k\":\"t\\n\"}]"),
        ("- k: &y !!str\n  |\n    t\n", "[{\"k\":\"t\\n\"}]"),
        ("- k: &y !!str\n  >+\n    t\n", "[{\"k\":\"t\\n\"}]"),
        ("-\r\n|\r\n  t\r\n", "[\"t\\n\"]"),
        ("? a\r\n:\r\n>\r\n  t\r\n", "{\"a\":\"t\\n\"}"),
        (
            "a:\r\n  b: &y\r\n  |\r\n    t\r\n",
            "{\"a\":{\"b\":\"t\\n\"}}",
        ),
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{input:?}");
        assert_eq!(out.trim_end(), expected, "{input:?}");
    }
    Ok(())
}

/// A line at the owner's indent that starts with `|` or `>` but is not a header stays refused, as it
/// was and as in yq (the first byte decides that it is the value, the block scalar reader that it is
/// not a valid one).
#[test]
fn a_malformed_header_at_the_indent_of_an_empty_node_stays_refused_6390() -> Result<()> {
    for input in [
        "a:\n>foo\n",
        "a:\n|x\n",
        "-\n>foo\n",
        "? a\n:\n>foo\n",
        "a:\n  -\n  |x\n",
        "a: &y\n>foo\n",
    ] {
        let (_, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_ne!(code, 0, "{input:?}");
    }
    Ok(())
}
