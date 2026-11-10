//! #6881: yq's `style` of a node with an explicit tag is `"tagged"` when nothing else styles the node
//! and `"<unknown>"` when something does (quotes, a block scalar, a flow collection): it matches its
//! style flag bits one value at a time and the combination matches none. The non-specific tag `!`,
//! an alias node and an untagged node answer their own style. Every row is captured from yq v4.53.3:
//! the source document, the filter and the whole output (`-o=json -I0`).

use anyhow::Result;

use crate::run_yq_stdin;

const ROWS: &[(&str, &str, &str)] = &[
    ("a: !foo x\n", ".a | style", "\"tagged\"\n"),
    ("a: !!str x\n", ".a | style", "\"tagged\"\n"),
    ("a: !!int 5\n", ".a | style", "\"tagged\"\n"),
    ("a: !!null ~\n", ".a | style", "\"tagged\"\n"),
    ("a: !!str\n", ".a | style", "\"tagged\"\n"),
    ("a: !foo\n  k: v\n", ".a | style", "\"tagged\"\n"),
    ("a: !foo\n  - x\n", ".a | style", "\"tagged\"\n"),
    ("a: !!seq\n  - x\n", ".a | style", "\"tagged\"\n"),
    ("a: !foo \"x\"\n", ".a | style", "\"<unknown>\"\n"),
    ("a: !foo 'x'\n", ".a | style", "\"<unknown>\"\n"),
    ("a: !foo |-\n  x\n", ".a | style", "\"<unknown>\"\n"),
    ("a: !!str |-\n  x\n", ".a | style", "\"<unknown>\"\n"),
    ("a: !foo >\n  x\n", ".a | style", "\"<unknown>\"\n"),
    ("a: !foo [x]\n", ".a | style", "\"<unknown>\"\n"),
    ("a: !foo {k: v}\n", ".a | style", "\"<unknown>\"\n"),
    ("a: &x !foo |\n  x\n", ".a | style", "\"<unknown>\"\n"),
    ("a: &x !!str \"q\"\n", ".a | style", "\"<unknown>\"\n"),
    ("a: &x |\n  x\n", ".a | style", "\"literal\"\n"),
    ("a: !<tag:x> |\n  x\n", ".a | style", "\"<unknown>\"\n"),
    ("a: !foo \"x\" # c\n", ".a | style", "\"<unknown>\"\n"),
    ("a: !foo x # c\n", ".a | style", "\"tagged\"\n"),
    ("a: &x !foo y\nb: *x\n", ".b | style", "\"\"\n"),
    (
        "a: &x !foo y\nb: *x\n",
        "[.. | style]",
        "[\"\",\"tagged\",\"\"]\n",
    ),
    ("a: ! x\n", ".a | style", "\"\"\n"),
    ("!foo\na: 1\n", ". | style", "\"tagged\"\n"),
    ("--- !!map\na: 1\n", "style", "\"tagged\"\n"),
    (
        "a: [!foo x, !!int 1]\n",
        ".a[] | style",
        "\"tagged\"\n\"tagged\"\n",
    ),
    (
        "a: !Ref x\nb: !Sub \"y\"\nc: !GetAtt [d, e]\n",
        "[.[] | style]",
        "[\"tagged\",\"<unknown>\",\"<unknown>\"]\n",
    ),
    (
        "a: 1\nb: \"x\"\nc: [1]\nd: {k: v}\ne: |\n  t\nf: >\n  t\ng: 's'\n",
        "[.[] | style]",
        "[\"\",\"double\",\"flow\",\"flow\",\"literal\",\"folded\",\"single\"]\n",
    ),
    (
        "a: !foo x\nb: !foo \"y\"\n",
        "[.. | style]",
        "[\"\",\"tagged\",\"<unknown>\"]\n",
    ),
    (
        "a: !foo x\n",
        "select(.a | style == \"tagged\")",
        "{\"a\":\"x\"}\n",
    ),
    (
        "a: !foo x\nb: y\n",
        "[.. | select(style == \"tagged\")] | length",
        "1\n",
    ),
    (
        "a: !foo \"x\"\nb: !foo y\n",
        "[.. | select(style == \"double\")] | length",
        "0\n",
    ),
    ("a: !foo x\n", ".a as $v | $v | style", "\"tagged\"\n"),
];

#[test]
fn the_style_of_a_tagged_node_is_tagged_or_unknown_6881() -> Result<()> {
    for (input, filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, input, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "{filter} over {input:?}");
        assert_eq!(out, *want, "{filter} over {input:?}");
    }
    Ok(())
}
