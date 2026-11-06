//! #6393: go-yaml eats a block scalar line's spaces only up to the indent column, so a line made
//! of spaces alone and wider than the indent has the extra spaces as its text: it is a content
//! line (a more-indented one under `>`), not a blank one. `succinctly yq` called such a line at
//! the end of a scalar blank, so `|`, `>`, `|-` and `>-` lost the spaces and the break before them,
//! and a leading one under an explicit indent indicator was dropped too. A line no wider than the
//! indent is still blank, and `|+` already kept both. Every value was captured from yq v4.53.3; the
//! grid behind them (3 hosts, 6 headers, 11 trailing-line shapes, 4 bodies, with and without a
//! following key: 1,584 documents, plus 440 for explicit indents, end of input and the document
//! root) has no difference in value or verdict.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_whitespace_line_wider_than_the_indent_is_block_scalar_content_6393() -> Result<()> {
    for (input, expected) in [
        ("- |\n  line1\n  line2\n   \n", "[\"line1\\nline2\\n \\n\"]"),
        (
            "a: >+\n  line1\n   \nb: 1\n",
            "{\"a\":\"line1\\n \\n\",\"b\":1}",
        ),
        (
            "a:\n  b: |-\n    line1\n     \n\n      \n",
            "{\"a\":{\"b\":\"line1\\n \\n\\n  \"}}",
        ),
        (
            "a: >-\n  line1\n  line2\n   \n",
            "{\"a\":\"line1 line2\\n \"}",
        ),
        (
            "a:\n  b: |-\n    line1\n        \n",
            "{\"a\":{\"b\":\"line1\\n    \"}}",
        ),
        (
            "a:\n  b: >+\n    line1\n    line2\n      \nc: 1\n",
            "{\"a\":{\"b\":\"line1 line2\\n  \\n\"},\"c\":1}",
        ),
        (
            "a:\n  b: >\n    line1\n    line2\n     \nc: 1\n",
            "{\"a\":{\"b\":\"line1 line2\\n \\n\"},\"c\":1}",
        ),
        (
            "- |-\n  line1\n   \n\n    \n- 2\n",
            "[\"line1\\n \\n\\n  \",2]",
        ),
        ("- |\n  line1\n     \n  \n- 2\n", "[\"line1\\n   \\n\",2]"),
        (
            "a: |-\n  line1\n  line2\n     \n  \nb: 1\n",
            "{\"a\":\"line1\\nline2\\n   \",\"b\":1}",
        ),
        ("- |-\n  line1\n   \n- 2\n", "[\"line1\\n \",2]"),
        (
            "a: >-\n  line1\n    \n    \nb: 1\n",
            "{\"a\":\"line1\\n  \\n  \",\"b\":1}",
        ),
        ("- >+\n  line1\n   \n", "[\"line1\\n \\n\"]"),
        (
            "a: |\n  line1\n   \n\n    \n",
            "{\"a\":\"line1\\n \\n\\n  \\n\"}",
        ),
        (
            "a: >\n  line1\n  line2\n      \nb: 1\n",
            "{\"a\":\"line1 line2\\n    \\n\",\"b\":1}",
        ),
        (
            "a:\n  b: >\n    line1\n    line2\n     \n\n      \nc: 1\n",
            "{\"a\":{\"b\":\"line1 line2\\n \\n\\n  \\n\"},\"c\":1}",
        ),
        (
            "- |-\n  line1\n  line2\n   \n\n    \n",
            "[\"line1\\nline2\\n \\n\\n  \"]",
        ),
        (
            "a:\n  b: >\n    line1\n    line2\n    \n       \n",
            "{\"a\":{\"b\":\"line1 line2\\n\\n   \\n\"}}",
        ),
        (
            "a:\n  b: |-\n    line1\n       \n    \nc: 1\n",
            "{\"a\":{\"b\":\"line1\\n   \"},\"c\":1}",
        ),
        (
            "a:\n  b: >\n    line1\n      \n      \n",
            "{\"a\":{\"b\":\"line1\\n  \\n  \\n\"}}",
        ),
        (
            "a: |\n  line1\n  line2\n   \n\n    \nb: 1\n",
            "{\"a\":\"line1\\nline2\\n \\n\\n  \\n\",\"b\":1}",
        ),
        (
            "a: >\n  line1\n  line2\n    \n    \nb: 1\n",
            "{\"a\":\"line1 line2\\n  \\n  \\n\",\"b\":1}",
        ),
        (
            "a: >+\n  line1\n      \nb: 1\n",
            "{\"a\":\"line1\\n    \\n\",\"b\":1}",
        ),
        (
            "a:\n  b: |-\n    line1\n    line2\n     \n\n      \nc: 1\n",
            "{\"a\":{\"b\":\"line1\\nline2\\n \\n\\n  \"},\"c\":1}",
        ),
        (
            "a: |\n  line1\n   \n\n    \nb: 1\n",
            "{\"a\":\"line1\\n \\n\\n  \\n\",\"b\":1}",
        ),
        ("- >-\n  line1\n   \n", "[\"line1\\n \"]"),
        (
            "- |-\n  line1\n  line2\n    \n    \n",
            "[\"line1\\nline2\\n  \\n  \"]",
        ),
        (
            "a:\n  b: |-\n    line1\n    line2\n    \n       \n",
            "{\"a\":{\"b\":\"line1\\nline2\\n\\n   \"}}",
        ),
        (
            "a:\n  b: >-\n    line1\n        \n",
            "{\"a\":{\"b\":\"line1\\n    \"}}",
        ),
        (
            "- |+\n  line1\n  line2\n  \n     \n- 2\n",
            "[\"line1\\nline2\\n\\n   \\n\",2]",
        ),
        (
            "a: >\n  line1\n  line2\n  \n     \nb: 1\n",
            "{\"a\":\"line1 line2\\n\\n   \\n\",\"b\":1}",
        ),
        ("a: >-\n  line1\n    \n", "{\"a\":\"line1\\n  \"}"),
        (
            "a: |\n  line1\n   \nb: 1\n",
            "{\"a\":\"line1\\n \\n\",\"b\":1}",
        ),
        ("- |-\n  line1\n   \n", "[\"line1\\n \"]"),
        (
            "a: |\n  line1\n     \n  \nb: 1\n",
            "{\"a\":\"line1\\n   \\n\",\"b\":1}",
        ),
        ("- >-\n  line1\n    \n- 2\n", "[\"line1\\n  \",2]"),
        (
            "- >-\n  line1\n  line2\n     \n  \n",
            "[\"line1 line2\\n   \"]",
        ),
        ("- |-\n  line1\n     \n  \n- 2\n", "[\"line1\\n   \",2]"),
        (
            "a:\n  b: >-\n    line1\n       \n    \n",
            "{\"a\":{\"b\":\"line1\\n   \"}}",
        ),
        (
            "a: >\n  line1\n  line2\n   \n",
            "{\"a\":\"line1 line2\\n \\n\"}",
        ),
        ("- >\n  line1\n    \n- 2\n", "[\"line1\\n  \\n\",2]"),
        (
            "- |-\n  line1\n    \n    \n- 2\n",
            "[\"line1\\n  \\n  \",2]",
        ),
        (
            "a: >+\n  line1\n  \n     \n",
            "{\"a\":\"line1\\n\\n   \\n\"}",
        ),
        (
            "a: |+\n  line1\n  \n     \n",
            "{\"a\":\"line1\\n\\n   \\n\"}",
        ),
        (
            "a: >\n  line1\n      \nb: 1\n",
            "{\"a\":\"line1\\n    \\n\",\"b\":1}",
        ),
        ("- |\n  line1\n  \n     \n", "[\"line1\\n\\n   \\n\"]"),
        (
            "a:\n  b: |\n    line1\n    line2\n      \nc: 1\n",
            "{\"a\":{\"b\":\"line1\\nline2\\n  \\n\"},\"c\":1}",
        ),
        (
            "a: |\n  line1\n  line2\n  \n     \n",
            "{\"a\":\"line1\\nline2\\n\\n   \\n\"}",
        ),
        ("a: >\n  line1\n      \n", "{\"a\":\"line1\\n    \\n\"}"),
        (
            "- >-\n  line1\n  line2\n  \n     \n",
            "[\"line1 line2\\n\\n   \"]",
        ),
        (
            "a: >\n  line1\n  \n     \nb: 1\n",
            "{\"a\":\"line1\\n\\n   \\n\",\"b\":1}",
        ),
        ("- >\n  line1\n     \n  \n", "[\"line1\\n   \\n\"]"),
        ("- >-\n  line1\n      \n- 2\n", "[\"line1\\n    \",2]"),
        ("- |\n  line1\n    \n    \n", "[\"line1\\n  \\n  \\n\"]"),
        ("a: |-\n  line1\n    \n", "{\"a\":\"line1\\n  \"}"),
        (
            "- >-\n  line1\n  line2\n      \n- 2\n",
            "[\"line1 line2\\n    \",2]",
        ),
        (
            "a:\n  b: |+\n    line1\n    line2\n    \n       \nc: 1\n",
            "{\"a\":{\"b\":\"line1\\nline2\\n\\n   \\n\"},\"c\":1}",
        ),
        (
            "- >-\n  line1\n    \n    \n- 2\n",
            "[\"line1\\n  \\n  \",2]",
        ),
        ("a: |+\n  line1\n   \n", "{\"a\":\"line1\\n \\n\"}"),
        (
            "a:\n  b: |\n    line1\n    line2\n    \n       \nc: 1\n",
            "{\"a\":{\"b\":\"line1\\nline2\\n\\n   \\n\"},\"c\":1}",
        ),
        ("- |1\n  \n", "[\" \\n\"]"),
        ("a: |3\n    \n\n\n", "{\"a\":\" \\n\"}"),
        ("a: >2\n    \nb: 1\n", "{\"a\":\"  \\n\",\"b\":1}"),
        ("- >-2\n   \n- 2\n", "[\" \",2]"),
        ("a: >2\n    \n  x\n", "{\"a\":\"  \\nx\\n\"}"),
        ("a: |1\n  \n      \n", "{\"a\":\" \\n     \\n\"}"),
        ("- |\n     \n  # c\n\n\n", "[\"\"]"),
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{input:?}");
        assert_eq!(out.trim_end(), expected, "{input:?}");
    }
    Ok(())
}
