//! #6067: in a double-quoted scalar a backslash before a line break continues the line, and go-yaml
//! then keeps *every* break that follows literally: the first is not folded to a space. `succinctly yq`
//! folded it as it would after ordinary text, so `"z\\<break><break> x"` read `z x` where yq reads
//! `z\nx`. The rule holds for the JSON and the YAML writers, for CRLF, and after an escaped
//! U+0085/U+2028/U+2029 (#4913). Each row is a live capture from yq v4.53.3: the scalar's body (the
//! text between the quotes), then `yq -o=json -I0 .a` and `yq -o=yaml .a` over `a: "<body>"`.

use anyhow::Result;

use crate::run_yq_stdin;

/// `(body, JSON output, YAML output)`.
const ROWS: &[(&str, &str, &str)] = &[
    ("z\\\n\n x", "\"z\\nx\"\n", "z\nx\n"),
    ("z\\\n\n\n x", "\"z\\n\\nx\"\n", "z\n\nx\n"),
    ("z \\\n\n x", "\"z \\nx\"\n", "z \nx\n"),
    ("z\\\n  \\\n x", "\"zx\"\n", "zx\n"),
    ("z\\\n x", "\"zx\"\n", "zx\n"),
    ("z\\\n\n\n\n x", "\"z\\n\\n\\nx\"\n", "z\n\n\nx\n"),
    ("z\\\n   \n x", "\"z\\nx\"\n", "z\nx\n"),
    ("z\\\n\t\n x", "\"z\\nx\"\n", "z\nx\n"),
    ("z\\\n\n\tx", "\"z\\nx\"\n", "z\nx\n"),
    ("z\\\r\n\r\n x", "\"z\\nx\"\n", "z\nx\n"),
    ("z\\\r\n\r\n\r\n x", "\"z\\n\\nx\"\n", "z\n\nx\n"),
    ("z\\\n\r\n x", "\"z\\nx\"\n", "z\nx\n"),
    ("a\nb", "\"a b\"\n", "a b\n"),
    ("a\n\nb", "\"a\\nb\"\n", "a\nb\n"),
    ("a\n\n\nb", "\"a\\n\\nb\"\n", "a\n\nb\n"),
    (
        "a \\\n\n b \\\n\n\n c",
        "\"a \\nb \\n\\nc\"\n",
        "a \nb \n\nc\n",
    ),
    ("\\\n\nx", "\"\\nx\"\n", "\nx\n"),
    ("x\\\n\n", "\"x\\n\"\n", "x\n\n"),
    ("x\\\\\n\ny", "\"x\\\\\\ny\"\n", "x\\\ny\n"),
    ("x\\\\\ny", "\"x\\\\ y\"\n", "x\\ y\n"),
    ("x\\n\n\ny", "\"x\\n\\ny\"\n", "x\n\ny\n"),
    ("x\\\n\u{85} y", "\"x\\ny\"\n", "x\ny\n"),
    ("x\\\u{85}\n y", "\"x\\ny\"\n", "x\ny\n"),
    ("x\\\u{2028}\n y", "\"x\\ny\"\n", "x\ny\n"),
    ("x\\\n\u{2028} y", "\"x\\u2028y\"\n", "x\u{2028}y\n"),
    ("p\\\n\n\\\n\n q", "\"p\\n\\nq\"\n", "p\n\nq\n"),
    ("p\\\n\n \\\n q", "\"p\\nq\"\n", "p\nq\n"),
];

#[test]
fn a_break_after_an_escaped_break_is_kept() -> Result<()> {
    for (body, json, yaml) in ROWS {
        let doc = format!("a: \"{body}\"\n");
        let (out, code) = run_yq_stdin(".a", &doc, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "{body:?}");
        assert_eq!(out, *json, "json of {body:?}");
        let (out, code) = run_yq_stdin(".a", &doc, &["-o=yaml"])?;
        assert_eq!(code, 0, "{body:?}");
        assert_eq!(out, *yaml, "yaml of {body:?}");
    }
    Ok(())
}

/// The same rule inside a mapping, a sequence and a flow mapping, through the whole-document and the
/// streaming writers (identity, a sub-tree, a collect, a recursive walk).
#[test]
fn a_break_after_an_escaped_break_is_kept_in_nested_values() -> Result<()> {
    let doc = "a: \"z\\\n\n x\"\nb: [ \"p\\\n\n q\", 1 ]\nc: {k: \"r \\\n\n\n s\"}\n";
    let rows: &[(&str, &[&str], &str)] = &[
        (".", &["-o=json", "-I0"], "{\"a\":\"z\\nx\",\"b\":[\"p\\nq\",1],\"c\":{\"k\":\"r \\n\\ns\"}}\n"),
        (".", &["-o=json", "-I=2"], "{\n  \"a\": \"z\\nx\",\n  \"b\": [\n    \"p\\nq\",\n    1\n  ],\n  \"c\": {\n    \"k\": \"r \\n\\ns\"\n  }\n}\n"),
        (".b", &["-o=json", "-I0"], "[\"p\\nq\",1]\n"),
        (".c", &["-o=json", "-I=2"], "{\n  \"k\": \"r \\n\\ns\"\n}\n"),
        ("[.[]]", &["-o=json", "-I0"], "[\"z\\nx\",[\"p\\nq\",1],{\"k\":\"r \\n\\ns\"}]\n"),
        (".", &["-o=yaml"], "a: \"z\\nx\"\nb: [\"p\\nq\", 1]\nc: {k: \"r \\n\\ns\"}\n"),
        (".. | select(tag == \"!!str\")", &["-o=json", "-I0"], "\"z\\nx\"\n\"p\\nq\"\n\"r \\n\\ns\"\n"),
    ];
    for (filter, args, want) in rows {
        let (out, code) = run_yq_stdin(filter, doc, args)?;
        assert_eq!(code, 0, "{filter} {args:?}");
        assert_eq!(out, *want, "{filter} {args:?}");
    }
    Ok(())
}
