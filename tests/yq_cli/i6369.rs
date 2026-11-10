//! #6369 (the block-scalar half of #4913): go-yaml ends a block scalar's line at U+2028, U+2029 and
//! U+0085 as at `\n`, so the spaces after one are indentation, a dedent after one ends the scalar
//! (in the middle of what a `\n`-only reader sees as one physical line), what follows is read as
//! structure (a key, a comment, a sequence item, a document marker), and U+0085 reads as `\n` in
//! the value while U+2028 and U+2029 stay as they are and never fold. Every answer below was
//! captured from yq v4.53.3 (`-o=json -I=0`); `None` is a document yq refuses. The three breaks
//! were also run through a grid of 960 documents (four headers, three breaks, 0-3 spaces before and
//! after, five tails) and a random one of 30,000 with no difference outside the shapes listed in
//! `docs/compliance/yq/limitations.md`.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, yq's answer or `None` for a refusal).
const ROWS: &[(&str, Option<&str>)] = &[
    // dedent after LS ends the scalar
    (
        "a: |\n  x\u{2028}b: 1\n",
        Some("{\"a\":\"x\\u2028\",\"b\":1}"),
    ),
    // comment after LS is a comment...
    ("a: |\n  x\u{2028}# c\n", Some("{\"a\":\"x\\u2028\"}")),
    // indent after LS stripped (literal)
    ("a: |\n  x\u{2028}  y\n", Some("{\"a\":\"x\\u2028y\\n\"}")),
    // indent after LS stripped (folded)
    (
        "a: >\n  x\u{2028}  y\n  z\n",
        Some("{\"a\":\"x\\u2028y z\\n\"}"),
    ),
    // indent after PS stripped
    ("a: |\n  x\u{2029}  y\n", Some("{\"a\":\"x\\u2029y\\n\"}")),
    // NEL becomes a newline, tab after indent kept
    ("a: |\n  x\u{85}  \tq\n", Some("{\"a\":\"x\\n\\tq\\n\"}")),
    // LS then text at col 0 is refused
    ("a: |\n  x\u{2028}y\n", None),
    // two LS
    ("a: |\n  x\u{2028}\u{2028}y\n", None),
    // LS then a sequence item at col 0
    ("a: |\n  x\u{2028}- y\n", None),
    // LS then document marker
    ("a: |\n  x\u{2028}---\n", Some("{\"a\":\"x\\u2028\"}\nnull")),
    // LS alone on first line
    ("a: |\n   \u{2028} z\n", None),
    // LS then newline then text
    (
        "a: |\n  x\u{2028}\n  y\n",
        Some("{\"a\":\"x\\u2028\\ny\\n\"}"),
    ),
    // NEL folds like newline
    ("a: >\n  x\u{85}  y\n", Some("{\"a\":\"x y\\n\"}")),
    // LS never folds
    ("a: >\n  x\u{2028}  y\n", Some("{\"a\":\"x\\u2028y\\n\"}")),
    // keep chomping last LS
    ("a: |+\n  x\u{2028}\n\n", Some("{\"a\":\"x\\u2028\\n\\n\"}")),
    // strip chomping last LS
    ("a: |-\n  x\u{2028}\n", Some("{\"a\":\"x\"}")),
    // clip chomping last LS at eof
    ("a: |\n  x\u{2028}", Some("{\"a\":\"x\\u2028\"}")),
    // empty line of spaces and LS
    (
        "a: |\n  x\n \u{2028}  y\nb: 1\n",
        Some("{\"a\":\"x\\n\\u2028y\\n\",\"b\":1}"),
    ),
    // empty line then dedent
    (
        "a: |\n  x\n \u{2029}b: 1\n",
        Some("{\"a\":\"x\\n\",\"b\":1}"),
    ),
    // dedent ends: sequence
    ("- |\n  x\u{2029}- y\n", Some("[\"x\\u2029\",\"y\"]")),
    // dedent after NEL in sequence host
    ("- |\n  x\u{85}  y\n- 2\n", Some("[\"x\\ny\\n\",2]")),
    // nested key
    ("k:\n  a: |\n    x\u{2028}  y\n  b: 1\n", None),
    // explicit indent
    (
        "a: |2\n  x\u{2028}  y\nb: 1\n",
        Some("{\"a\":\"x\\u2028y\\n\",\"b\":1}"),
    ),
    // tab below indent after NEL
    ("a: |\n  x\u{85} \ty\n", None),
    // em dash is no break
    (
        "a: |\n  x\u{2014}  y\nb: 1\n",
        Some("{\"a\":\"x\u{2014}  y\\n\",\"b\":1}"),
    ),
    // nbsp is no break
    (
        "a: |\n  x\u{a0}  y\nb: 1\n",
        Some("{\"a\":\"x\u{a0}  y\\n\",\"b\":1}"),
    ),
    // CRLF doc
    (
        "a: |\r\n  x\u{2028}  y\r\nb: 1\r\n",
        Some("{\"a\":\"x\\u2028y\\n\",\"b\":1}"),
    ),
];

#[test]
fn a_block_scalar_holding_a_unicode_break_is_read_as_go_yaml_reads_it_6369() -> Result<()> {
    for &(input, expected) in ROWS {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        match expected {
            Some(want) => {
                assert_eq!(code, 0, "{input:?}: {out}");
                assert_eq!(out.trim_end(), want, "{input:?}");
            }
            None => {
                assert_ne!(code, 0, "{input:?}: {out}");
                assert_eq!(out.trim_end(), "", "{input:?}");
            }
        }
    }
    Ok(())
}
