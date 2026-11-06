//! #6381: go-yaml takes an auto-detected block scalar's indent from the widest of the leading blank
//! lines and the first content line (`max_indent` in `yaml_parser_scan_block_scalar_breaks`). A
//! blank line with more spaces than the first content line therefore makes that content line a
//! dedent: the scalar ends empty and the line is read as the next token, which is an error in a
//! mapping or a sequence. `succinctly yq` took the indent from the content line alone and kept
//! the blank line's extra spaces as content. Every verdict and value was captured from yq v4.53.3;
//! the grid behind them (6 hosts, 8 headers, blank-line widths 0-7, content indents 1-6, one or
//! two blank lines, 3 tails: 13,824 documents, 7,384 refused by yq) has no difference in which are
//! refused.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

#[test]
fn a_blank_line_wider_than_the_first_content_line_ends_a_block_scalar_6381() -> Result<()> {
    for input in [
        "a: |\n    \n  x\nb: 1\n",
        "a: |\n       \n      x\nb: 1\n",
        "a: >\n   \n  x\nb: 1\n",
        "a: >\n   \n x\nb: 1\n",
        "a: |-\n     \n    x\nb: 1\n",
        "a: |+\n    \n  x\nb: 1\n",
        "a: |+\n       \n      x\nb: 1\n",
        "a: >-\n   \n  x\nb: 1\n",
        "a: >-\n   \n x\nb: 1\n",
        "- |\n     \n    x\n- 2\n",
        "- >\n    \n  x\n- 2\n",
        "- >\n       \n      x\n- 2\n",
        "- |-\n   \n  x\n- 2\n",
        "- |-\n   \n x\n- 2\n",
        "- |+\n     \n    x\n- 2\n",
        "- >-\n    \n  x\n- 2\n",
        "- >-\n       \n      x\n- 2\n",
        "a:\n  b: |\n     \n    x\n",
        "a:\n  b: |\n     \n   x\n",
        "a:\n  b: >\n       \n      x\n",
        "a:\n  b: |-\n      \n    x\n",
        "a:\n  b: |-\n         \n        x\n",
        "a:\n  b: |+\n     \n    x\n",
        "a:\n  b: |+\n     \n   x\n",
        "a:\n  b: >-\n       \n      x\n",
        "a:\n  - |\n      \n    x\n",
        "a:\n  - |\n         \n        x\n",
        "a:\n  - >\n     \n    x\n",
        "a:\n  - >\n     \n   x\n",
        "a:\n  - |-\n       \n      x\n",
        "a:\n  - |+\n      \n    x\n",
        "a:\n  - |+\n         \n        x\n",
        "a:\n  - >-\n     \n    x\n",
        "a:\n  - >-\n     \n   x\n",
        "a: |\n    \n  x\n      y\n",
        "a: |\n  \n    \n  x\n",
        "a: |\n\n    \n  x\nb: 1\n",
        "a: >+\n    \n  x\n\nb: 1\n",
        "|\n    \n  x\n",
        "a: |\r\n    \r\n  x\r\n",
        "a: |\n    \n  # c\n  x\n",
        "a: |\n    \n# c\n  x\nb: 1\n",
        "- a: |\n      \n    x\n  b: 1\n",
    ] {
        let (out, err, code) = run_yq_stdin_with_stderr(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(out.trim_end(), "", "{input:?}");
        assert_ne!(code, 0, "{input:?}");
        // the stray line is what is refused, not a tab or a crash (CI adds a backtrace after the
        // first line, so only that line is read)
        let first = err.lines().next().unwrap_or_default();
        assert!(first.contains("YAML parse error"), "{input:?}: {err}");
        assert!(!first.contains("tab character"), "{input:?}: {err}");
    }
    Ok(())
}

#[test]
fn a_blank_line_no_wider_than_the_content_is_read_as_before_6381() -> Result<()> {
    for (input, expected) in [
        ("a: |\n  \n  x\nb: 1\n", "{\"a\":\"\\nx\\n\",\"b\":1}"),
        ("a: |\n    \n      x\nb: 1\n", "{\"a\":\"\\nx\\n\",\"b\":1}"),
        ("a: >\n \n  x\nb: 1\n", "{\"a\":\"\\nx\\n\",\"b\":1}"),
        ("a: |-\n  \n  x\nb: 1\n", "{\"a\":\"\\nx\",\"b\":1}"),
        ("a: |-\n    \n      x\nb: 1\n", "{\"a\":\"\\nx\",\"b\":1}"),
        ("a: |+\n \n  x\nb: 1\n", "{\"a\":\"\\nx\\n\",\"b\":1}"),
        ("- |\n  \n  x\n- 2\n", "[\"\\nx\\n\",2]"),
        ("- |\n    \n      x\n- 2\n", "[\"\\nx\\n\",2]"),
        ("- >\n \n  x\n- 2\n", "[\"\\nx\\n\",2]"),
        ("- |-\n  \n  x\n- 2\n", "[\"\\nx\",2]"),
        ("- |-\n    \n      x\n- 2\n", "[\"\\nx\",2]"),
        ("- |+\n \n  x\n- 2\n", "[\"\\nx\\n\",2]"),
        ("a:\n  b: |\n    \n    x\n", "{\"a\":{\"b\":\"\\nx\\n\"}}"),
        (
            "a:\n  b: |\n      \n        x\n",
            "{\"a\":{\"b\":\"\\nx\\n\"}}",
        ),
        ("a:\n  b: >\n   \n    x\n", "{\"a\":{\"b\":\"\\nx\\n\"}}"),
        ("a:\n  b: |-\n    \n    x\n", "{\"a\":{\"b\":\"\\nx\"}}"),
        (
            "a:\n  b: |-\n      \n        x\n",
            "{\"a\":{\"b\":\"\\nx\"}}",
        ),
        ("a:\n  b: |+\n   \n    x\n", "{\"a\":{\"b\":\"\\nx\\n\"}}"),
        ("a:\n  - |\n    \n    x\n", "{\"a\":[\"\\nx\\n\"]}"),
        ("a:\n  - |\n      \n        x\n", "{\"a\":[\"\\nx\\n\"]}"),
        ("a:\n  - >\n   \n    x\n", "{\"a\":[\"\\nx\\n\"]}"),
        ("a:\n  - |-\n    \n    x\n", "{\"a\":[\"\\nx\"]}"),
        ("a:\n  - |-\n      \n        x\n", "{\"a\":[\"\\nx\"]}"),
        ("a:\n  - |+\n   \n    x\n", "{\"a\":[\"\\nx\\n\"]}"),
        ("a: |2\n    \n  x\n", "{\"a\":\"  \\nx\\n\"}"),
        ("a: |\n  \n  \n  x\n", "{\"a\":\"\\n\\nx\\n\"}"),
        ("a: |\n\n\n  x\nb: 1\n", "{\"a\":\"\\n\\nx\\n\",\"b\":1}"),
        ("a: |\n    \n      x\n      y\n", "{\"a\":\"\\nx\\ny\\n\"}"),
        ("--- |\n    \n  x\n", "\"x\\n\""),
        ("--- |\n    \n  x\n--- 2\n", "\"x\\n\"\n2"),
        (
            "a: |\r\n  \r\n  x\r\nb: 1\r\n",
            "{\"a\":\"\\nx\\n\",\"b\":1}",
        ),
        (
            "a: |\n  # c\n     \n    x\n",
            "{\"a\":\"# c\\n   \\n  x\\n\"}",
        ),
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{input:?}");
        assert_eq!(out.trim_end(), expected, "{input:?}");
    }
    Ok(())
}
