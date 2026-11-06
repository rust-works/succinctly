//! #6372: go-yaml eats a block scalar's leading spaces while the column is below the indent, and a
//! tab met there is `found a tab character where an indentation space is expected`. #6295 made that
//! so for an auto-detected indent; this file pins the rest: a tab after fewer spaces than an
//! explicit indent indicator gives (the rows with `|2`, which read as an empty scalar and a stray
//! key), and a tab after the spaces of the dedented line that ends the scalar (which read as the
//! next token). The auto-detected rows are kept as a cross-check of the three together. Every
//! verdict and value was captured from yq v4.53.3; the grid behind them (4 hosts, 9 headers, 5
//! leading-line shapes, 0-6 spaces, 3 tab shapes, 3 tails: 11,340 documents) has no difference in
//! which are refused.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_tab_in_the_leading_whitespace_of_a_block_scalar_is_refused_6372() -> Result<()> {
    for input in [
        "a: |\n  \tx\n",
        "a: |\n \tx\n  y\n",
        "a: |\n  \n  \tx\n",
        "a: |\n  \t\n  x\n",
        "- |\n  \ta\n",
        "a:\n  b: |\n      \tx\n",
        "a: >\n  \tx\n",
        "a: |-\n  \tx\n",
        "a: |+\n  \tx\n\n",
        "a: |2\n \tx\n",
        "a: |2\n  \n \tx\n",
        "a: |\n  x\n \ty\nb: 1\n",
        "a: |2\n  x\n \ty\n",
        "- |\n  x\n \ty\n- 2\n",
        "a:\n  b: |\n      x\n    \ty\n",
        "a: >-\n  x\n \t\nb: 1\n",
        "a: |\n  x\n\t# c\n",
        "a: |\n  x\n \t\n",
        "a: |\n  x\n \t",
        "a: |\r\n  x\r\n \ty\r\nb: 1\r\n",
        "a: |\r\n  \tx\r\n",
        "a:\n  b: |\n  \tc: 1\n",
        "a:\n  b: |\n \tc: 1\n",
        "a:\n  - |\n  \tc\n",
        "a: |\n  x\n \t# c\nb: 1\n",
        "a: |\n  x\n\n \ty\n",
        "a: |+\n  x\n\n\n \ty\n",
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(out.trim_end(), "", "{input:?}");
        assert_ne!(code, 0, "{input:?}");
    }
    Ok(())
}

#[test]
fn a_tab_where_the_indent_is_known_is_content_6372() -> Result<()> {
    for (input, expected) in [
        ("a: |\n  x\n  \t\nb: 1\n", "{\"a\":\"x\\n\\t\\n\",\"b\":1}"),
        ("a: |2\n  \tx\n", "{\"a\":\"\\tx\\n\"}"),
        ("a: |\n  x\n  \ty\n", "{\"a\":\"x\\n\\ty\\n\"}"),
        (
            "a: |\n  x\n   \ty\nb: 1\n",
            "{\"a\":\"x\\n \\ty\\n\",\"b\":1}",
        ),
        ("a: |2\n    \tx\n", "{\"a\":\"  \\tx\\n\"}"),
        ("a: >\n  x\n\n  \ty\n", "{\"a\":\"x\\n\\n\\ty\\n\"}"),
        ("a: |\n  x\nb: 1\n", "{\"a\":\"x\\n\",\"b\":1}"),
        ("a: |\n\n  x\t\n", "{\"a\":\"\\nx\\t\\n\"}"),
        (
            "a:\n  b: |\n      x\n      \ty\n",
            "{\"a\":{\"b\":\"x\\n\\ty\\n\"}}",
        ),
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{input:?}");
        assert_eq!(out.trim_end(), expected, "{input:?}");
    }
    Ok(())
}
