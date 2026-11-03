//! #5416: `-C` colors YAML the way yq does -- by tokenizing the text with go-yaml's scanner and
//! printing each line of a token with the color of its type and its neighbours'. Every expected
//! output in `ORACLE_ROWS` is what yq v4.53.3 prints for the same document and filter, byte for
//! byte: the empty spans after a line break, a span that opens on the space before a value,
//! numbers and booleans in magenta, anchors and aliases in bright yellow, comments in grey, and a
//! scalar result printed bare.

use anyhow::Result;

use crate::run_yq_stdin;

/// (name, document, filter, extra args, expected stdout).
const ORACLE_ROWS: &[(&str, &str, &str, &[&str], &str)] = &[
    (
        "scalars",
        "e: true\nb: 2\nf: 1.5\nn: null\nt: ~\ns: text\nq: \"quoted\"\nk: 'single'\n",
        ".",
        &["-C"],
        "\u{1b}[36me\u{1b}[0m:\u{1b}[95m true\u{1b}[0m\n\u{1b}[95m\u{1b}[0m\u{1b}[36mb\u{1b}[0m:\u{1b}[95m 2\u{1b}[0m\n\u{1b}[95m\u{1b}[0m\u{1b}[36mf\u{1b}[0m:\u{1b}[95m 1.5\u{1b}[0m\n\u{1b}[95m\u{1b}[0m\u{1b}[36mn\u{1b}[0m: null\n\u{1b}[36mt\u{1b}[0m: ~\n\u{1b}[36ms\u{1b}[0m:\u{1b}[32m text\u{1b}[0m\n\u{1b}[32m\u{1b}[0m\u{1b}[36mq\u{1b}[0m:\u{1b}[32m \"quoted\"\u{1b}[0m\u{1b}[36m\u{1b}[0m\n\u{1b}[36mk\u{1b}[0m:\u{1b}[32m 'single'\u{1b}[0m\n",
    ),
    (
        "apostrophe",
        "a: it's ok\nb: 2\n",
        ".",
        &["-C"],
        "\u{1b}[36ma\u{1b}[0m:\u{1b}[32m it's ok\u{1b}[0m\n\u{1b}[32m\u{1b}[0m\u{1b}[36mb\u{1b}[0m:\u{1b}[95m 2\u{1b}[0m\n",
    ),
    (
        "quoted_key",
        "\"quoted key\": 1\n'single key': 2\n",
        ".",
        &["-C"],
        "\u{1b}[36m\"quoted key\"\u{1b}[0m:\u{1b}[95m 1\u{1b}[0m\n\u{1b}[95m\u{1b}[0m\u{1b}[36m'single key'\u{1b}[0m:\u{1b}[95m 2\u{1b}[0m\n",
    ),
    (
        "anchor_alias_comment",
        "a: &x 1 # c\nb: *x\n",
        ".",
        &["-C"],
        "\u{1b}[36ma\u{1b}[0m:\u{1b}[93m &\u{1b}[0m\u{1b}[93mx\u{1b}[0m\u{1b}[95m 1 \u{1b}[0m\u{1b}[90m# c\u{1b}[0m\n\u{1b}[90m\u{1b}[0m\u{1b}[36mb\u{1b}[0m:\u{1b}[93m *\u{1b}[0m\u{1b}[93mx\u{1b}[0m\n",
    ),
    (
        "sequence",
        "- x\n- \"s\"\n- 1.5\n- - 1\n  - 2\n- k: v\n  j: w\n",
        ".",
        &["-C"],
        "-\u{1b}[32m x\u{1b}[0m\n\u{1b}[32m\u{1b}[0m-\u{1b}[32m \"s\"\u{1b}[0m\n-\u{1b}[95m 1.5\u{1b}[0m\n\u{1b}[95m\u{1b}[0m- -\u{1b}[95m 1\u{1b}[0m\n\u{1b}[95m  \u{1b}[0m-\u{1b}[95m 2\u{1b}[0m\n\u{1b}[95m\u{1b}[0m-\u{1b}[36m k\u{1b}[0m:\u{1b}[32m v\u{1b}[0m\n\u{1b}[32m  \u{1b}[0m\u{1b}[36mj\u{1b}[0m:\u{1b}[32m w\u{1b}[0m\n",
    ),
    (
        "block_scalars",
        "lit: |\n  - x\n  k: v\nfold: >\n  text\n  more\nstrip: |-\n  keep\n",
        ".",
        &["-C"],
        "\u{1b}[36mlit\u{1b}[0m: |\n\u{1b}[32m  - x\u{1b}[0m\n\u{1b}[32m  k: v\u{1b}[0m\n\u{1b}[32m\u{1b}[0m\u{1b}[36mfold\u{1b}[0m: >\n\u{1b}[32m  text more\u{1b}[0m\n\u{1b}[32m\u{1b}[0m\n\u{1b}[32m\u{1b}[0m\u{1b}[36mstrip\u{1b}[0m: |-\n\u{1b}[32m  keep\u{1b}[0m\n\u{1b}[32m\u{1b}[0m\n",
    ),
    (
        "flow",
        "a: [1, \"b\", c]\nd: {x: 1, y: z}\n",
        ".",
        &["-C"],
        "\u{1b}[36ma\u{1b}[0m: [\u{1b}[95m1\u{1b}[0m,\u{1b}[32m \"b\"\u{1b}[0m,\u{1b}[32m c\u{1b}[0m]\u{1b}[36m\u{1b}[0m\n\u{1b}[36md\u{1b}[0m: {\u{1b}[36mx\u{1b}[0m:\u{1b}[95m 1\u{1b}[0m,\u{1b}[36m y\u{1b}[0m:\u{1b}[32m z\u{1b}[0m}\n",
    ),
    (
        "numbers",
        "h: 0x1f\no: 0o17\nb: 0b101\ni: .inf\nm: -.inf\nn: .nan\nz: 007\nd: 2024-01-01\nt: 12:30\n",
        ".",
        &["-C"],
        "\u{1b}[36mh\u{1b}[0m: 0x1f\n\u{1b}[36mo\u{1b}[0m: 0o17\n\u{1b}[36mb\u{1b}[0m: 0b101\n\u{1b}[36mi\u{1b}[0m: .inf\n\u{1b}[36mm\u{1b}[0m: -.inf\n\u{1b}[36mn\u{1b}[0m: .nan\n\u{1b}[36mz\u{1b}[0m: 007\n\u{1b}[36md\u{1b}[0m:\u{1b}[32m 2024-01-01\u{1b}[0m\n\u{1b}[32m\u{1b}[0m\u{1b}[36mt\u{1b}[0m:\u{1b}[32m 12:30\u{1b}[0m\n",
    ),
    (
        "tags",
        "a: !!str 1\nb: !tag x\nc: !custom 5\nd: !!int 3\n",
        ".",
        &["-C"],
        "\u{1b}[36ma\u{1b}[0m: !!str \u{1b}[95m1\u{1b}[0m\n\u{1b}[95m\u{1b}[0m\u{1b}[36mb\u{1b}[0m: !tag \u{1b}[32mx\u{1b}[0m\n\u{1b}[32m\u{1b}[0m\u{1b}[36mc\u{1b}[0m: !custom \u{1b}[32m5\u{1b}[0m\n\u{1b}[32m\u{1b}[0m\u{1b}[36md\u{1b}[0m: !!int \u{1b}[95m3\u{1b}[0m\n",
    ),
    (
        "tag_before_block",
        "a: !tag\n  b: 1\nc: !!map\n  d: 2\n",
        ".",
        &["-C"],
        "\u{1b}[36ma\u{1b}[0m: !tag\n\u{1b}[36m\u{1b}[0m\n\u{1b}[36m  b\u{1b}[0m:\u{1b}[95m 1\u{1b}[0m\n\u{1b}[95m\u{1b}[0m\u{1b}[36mc\u{1b}[0m: !!map\n\u{1b}[36m\u{1b}[0m\n\u{1b}[36m  d\u{1b}[0m:\u{1b}[95m 2\u{1b}[0m\n",
    ),
    (
        "head_comment",
        "# head\na: 1\n",
        ".",
        &["-C"],
        "\u{1b}[90m# head\n\u{1b}[0m\u{1b}[36ma\u{1b}[0m:\u{1b}[95m 1\u{1b}[0m\n",
    ),
    (
        "head_blank_foot",
        "# head\n\na: 1\n# foot\n",
        ".",
        &["-C"],
        "\u{1b}[90m# head\n\u{1b}[0m\n\u{1b}[36ma\u{1b}[0m:\u{1b}[95m 1\u{1b}[0m\n\u{1b}[95m\u{1b}[0m\u{1b}[90m# foot\u{1b}[0m\n\u{1b}[90m\u{1b}[0m\n",
    ),
    (
        "foot_block",
        "a: 1\n# foot\n\n# foot2\n",
        ".",
        &["-C"],
        "\u{1b}[36ma\u{1b}[0m:\u{1b}[95m 1\u{1b}[0m\n\u{1b}[95m\u{1b}[0m\u{1b}[90m# foot\u{1b}[0m\n\u{1b}[90m\u{1b}[0m\u{1b}[32m\u{1b}[90m# foot2\u{1b}[0m\u{1b}[0m\n",
    ),
    (
        "leading_marker",
        "# c\n---\na: 1\n",
        ".",
        &["-C"],
        "\u{1b}[90m# c\n\u{1b}[0m---\n\u{1b}[36ma\u{1b}[0m:\u{1b}[95m 1\u{1b}[0m\n",
    ),
    (
        "multi_doc",
        "a: 1\n---\n# h2\nb: 2\n# f2\n",
        ".",
        &["-C"],
        "\u{1b}[36ma\u{1b}[0m:\u{1b}[95m 1\u{1b}[0m\n---\n\u{1b}[90m# h2\u{1b}[0m\n\u{1b}[90m\u{1b}[0m\u{1b}[36mb\u{1b}[0m:\u{1b}[95m 2\u{1b}[0m\n\u{1b}[95m\u{1b}[0m\u{1b}[90m# f2\u{1b}[0m\n\u{1b}[90m\u{1b}[0m\n",
    ),
    (
        "bare_scalars",
        "a: 1\nb: text\n",
        ".a, .b",
        &["-C"],
        "1\ntext\n",
    ),
    (
        "computed_scalar",
        "a: 1\n",
        ".a + 1",
        &["-C"],
        "2\n",
    ),
    (
        "string_result",
        "a: x y\n",
        ".a",
        &["-C"],
        "x y\n",
    ),
    (
        "results",
        "- a: 1\n  a: 2\n- b: 3\n",
        ".[]",
        &["-C"],
        "\u{1b}[36ma\u{1b}[0m:\u{1b}[95m 1\u{1b}[0m\n\u{1b}[95m\u{1b}[0m\u{1b}[36ma\u{1b}[0m:\u{1b}[95m 2\u{1b}[0m\n\u{1b}[36mb\u{1b}[0m:\u{1b}[95m 3\u{1b}[0m\n",
    ),
    (
        "comments_inline",
        "a: 1 # one\nb: [1, 2] # two\n# between\nc: x\n",
        ".",
        &["-C"],
        "\u{1b}[36ma\u{1b}[0m:\u{1b}[95m 1 \u{1b}[0m\u{1b}[90m# one\u{1b}[0m\n\u{1b}[90m\u{1b}[0m\u{1b}[36mb\u{1b}[0m: [\u{1b}[95m1\u{1b}[0m,\u{1b}[95m 2\u{1b}[0m]\u{1b}[90m # two\u{1b}[0m\n\u{1b}[90m\u{1b}[0m\u{1b}[90m# between\u{1b}[0m\n\u{1b}[90m\u{1b}[0m\u{1b}[36mc\u{1b}[0m:\u{1b}[32m x\u{1b}[0m\n",
    ),
    (
        "head_comment_written",
        "a: 1\n",
        ". head_comment=\"hc\"",
        &["-C"],
        "\u{1b}[90m# hc\u{1b}[0m\n\u{1b}[90m\u{1b}[0m\u{1b}[36ma\u{1b}[0m:\u{1b}[95m 1\u{1b}[0m\n",
    ),
    (
        "foot_comment_written",
        "a: 1\n",
        ". foot_comment=\"fc\"",
        &["-C"],
        "\u{1b}[36ma\u{1b}[0m:\u{1b}[95m 1\u{1b}[0m\n\u{1b}[95m\u{1b}[0m\u{1b}[32m\u{1b}[90m# fc\u{1b}[0m\u{1b}[0m\n",
    ),
    (
        "flow_style",
        "a: {b: 1}\nc: [x]\n",
        ".. style=\"flow\"",
        &["-C"],
        "{\u{1b}[36ma\u{1b}[0m: {\u{1b}[36mb\u{1b}[0m:\u{1b}[95m 1\u{1b}[0m},\u{1b}[36m c\u{1b}[0m: [\u{1b}[32mx\u{1b}[0m]}\n",
    ),
    (
        "merge_key",
        "base: &b {x: 1}\nd:\n  <<: *b\n  y: 2\n",
        ".",
        &["-C"],
        "\u{1b}[36mbase\u{1b}[0m:\u{1b}[93m &\u{1b}[0m\u{1b}[93mb\u{1b}[0m {\u{1b}[36mx\u{1b}[0m:\u{1b}[95m 1\u{1b}[0m}\u{1b}[36m\u{1b}[0m\n\u{1b}[36md\u{1b}[0m:\n  !!merge \u{1b}[36m<<\u{1b}[0m:\u{1b}[93m *\u{1b}[0m\u{1b}[93mb\u{1b}[0m\u{1b}[36m\u{1b}[0m\n\u{1b}[36m  y\u{1b}[0m:\u{1b}[95m 2\u{1b}[0m\n",
    ),
    (
        "empty_values",
        "a:\nb: {}\nc: []\nd: ''\n",
        ".",
        &["-C"],
        "\u{1b}[36ma\u{1b}[0m:\u{1b}[36m\u{1b}[0m\n\u{1b}[36mb\u{1b}[0m: {}\u{1b}[36m\u{1b}[0m\n\u{1b}[36mc\u{1b}[0m: []\u{1b}[36m\u{1b}[0m\n\u{1b}[36md\u{1b}[0m:\u{1b}[32m ''\u{1b}[0m\n",
    ),
    (
        "multiline_plain",
        "a: one\n  two\n  three\nb: 1\n",
        ".",
        &["-C"],
        "\u{1b}[36ma\u{1b}[0m:\u{1b}[32m one two three\u{1b}[0m\n\u{1b}[32m\u{1b}[0m\u{1b}[36mb\u{1b}[0m:\u{1b}[95m 1\u{1b}[0m\n",
    ),
    (
        "unicode",
        "日本: é\n\"ü\": ñ\n",
        ".",
        &["-C"],
        "\u{1b}[36m日本\u{1b}[0m:\u{1b}[32m é\u{1b}[0m\n\u{1b}[32m\u{1b}[0m\u{1b}[36m\"ü\"\u{1b}[0m:\u{1b}[32m ñ\u{1b}[0m\n",
    ),
    (
        "long_key_compact",
        "- ? complex\n  : v\n",
        ".",
        &["-C"],
        "-\u{1b}[36m complex\u{1b}[0m:\u{1b}[32m v\u{1b}[0m\n",
    ),
    (
        "nul",
        "a: 1\nb: 2\n",
        ".",
        &["-C", "-0"],
        "\u{1b}[36ma\u{1b}[0m:\u{1b}[95m 1\u{1b}[0m\n\u{1b}[95m\u{1b}[0m\u{1b}[36mb\u{1b}[0m:\u{1b}[95m 2\u{1b}[0m\0",
    ),
    (
        "compact_indent",
        "a: {b: [1, 2]}\n",
        ".",
        &["-C", "-I0"],
        "\u{1b}[36ma\u{1b}[0m: {\u{1b}[36mb\u{1b}[0m: [\u{1b}[95m1\u{1b}[0m,\u{1b}[95m 2\u{1b}[0m]}\n",
    ),
    (
        "pretty",
        "a: {b: 1}\n",
        ".",
        &["-C", "-P"],
        "\u{1b}[36ma\u{1b}[0m:\u{1b}[36m\u{1b}[0m\n\u{1b}[36m  b\u{1b}[0m:\u{1b}[95m 1\u{1b}[0m\n",
    ),
];

#[test]
fn colors_match_yq_byte_for_byte_5416() -> Result<()> {
    for &(name, document, filter, args, expected) in ORACLE_ROWS {
        let (output, code) = run_yq_stdin(filter, document, args)?;
        assert_eq!(code, 0, "{name}");
        assert_eq!(output, expected, "{name}: `{filter}` over {document:?}");
    }
    Ok(())
}

/// The colors are the same whichever way the output reaches the writer: a document streamed from
/// its index, and the same document rebuilt by a write.
#[test]
fn a_write_that_changes_nothing_prints_the_colors_of_the_streamed_route_5416() -> Result<()> {
    for document in [
        "a: 1\nb: [x, y]\nc: &z {d: true}\ne: *z\n",
        "# head\n- a: it's\n  b: 'q'\n# foot\n",
    ] {
        let (streamed, _) = run_yq_stdin(".", document, &["-C"])?;
        let (rebuilt, _) = run_yq_stdin(". |= .", document, &["-C"])?;
        assert_eq!(rebuilt, streamed, "{document:?}");
    }
    Ok(())
}

/// go-yaml drops the `x`, `u` and `U` and the hex digits of an escape in a double-quoted scalar
/// from the text it prints, so yq's `-C` turns `"x\u0001y"` into `"x\y"`. That changes the
/// string, so this implementation keeps the escape: the only place its `-C` text differs from
/// yq's other than in a document whose plain output already differs (recorded in
/// `docs/compliance/yq/limitations.md`).
#[test]
fn an_escape_in_a_double_quoted_scalar_keeps_its_digits_5416() -> Result<()> {
    let (output, code) = run_yq_stdin(".", "a: \"x\\u0001y\\x02z\"\n", &["-C"])?;
    assert_eq!(code, 0);
    let (plain, _) = run_yq_stdin(".", "a: \"x\\u0001y\\x02z\"\n", &[])?;
    let stripped = strip_ansi(&output);
    assert_eq!(stripped, plain);
    assert!(
        stripped.contains("\\x01") || stripped.contains("\\u0001"),
        "{stripped:?}"
    );
    Ok(())
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}
