//! #6909: `style = ""` makes a node plain, and go-yaml's encoder quotes a string whose plain spelling it
//! reads as another type, so a quoted date or underscored number keeps its quotes (`"2023-07-12"` stays
//! quoted: plain it is a `!!timestamp`). The spellings the core schema already types, a plain scalar
//! and `style = "single"` / `"double"` are unchanged. Every row is captured from yq v4.53.3: the source
//! document, the filter and the whole YAML output.

use anyhow::Result;

use crate::run_yq_stdin;

const ROWS: &[(&str, &str, &str)] = &[
    (
        "a: \"2023-07-12\"\nb: x\n",
        ".a style=\"\"",
        "a: \"2023-07-12\"\nb: x\n",
    ),
    (
        "a: \"2023-07-12\"\nb: x\n",
        ".. style=\"\"",
        "a: \"2023-07-12\"\nb: x\n",
    ),
    (
        "a: \"2023-07-12\"\nb: x\n",
        ".a style=\"single\"",
        "a: '2023-07-12'\nb: x\n",
    ),
    (
        "a: \"2023-07-12\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"2023-07-12\"\nb: x\n",
    ),
    (
        "a: \"2023-07-12\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: \"2023-07-12\"\nb: x\n",
    ),
    (
        "a: \"2001-12-14T21:59:43Z\"\nb: x\n",
        ".a style=\"\"",
        "a: \"2001-12-14T21:59:43Z\"\nb: x\n",
    ),
    (
        "a: \"2001-12-14T21:59:43Z\"\nb: x\n",
        ".. style=\"\"",
        "a: \"2001-12-14T21:59:43Z\"\nb: x\n",
    ),
    (
        "a: \"2001-12-14T21:59:43Z\"\nb: x\n",
        ".a style=\"single\"",
        "a: '2001-12-14T21:59:43Z'\nb: x\n",
    ),
    (
        "a: \"2001-12-14T21:59:43Z\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"2001-12-14T21:59:43Z\"\nb: x\n",
    ),
    (
        "a: \"2001-12-14T21:59:43Z\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: \"2001-12-14T21:59:43Z\"\nb: x\n",
    ),
    ("a: \"123\"\nb: x\n", ".a style=\"\"", "a: \"123\"\nb: x\n"),
    ("a: \"123\"\nb: x\n", ".. style=\"\"", "a: \"123\"\nb: x\n"),
    (
        "a: \"123\"\nb: x\n",
        ".a style=\"single\"",
        "a: '123'\nb: x\n",
    ),
    (
        "a: \"123\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"123\"\nb: x\n",
    ),
    (
        "a: \"123\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: \"123\"\nb: x\n",
    ),
    ("a: \"1.5\"\nb: x\n", ".a style=\"\"", "a: \"1.5\"\nb: x\n"),
    ("a: \"1.5\"\nb: x\n", ".. style=\"\"", "a: \"1.5\"\nb: x\n"),
    (
        "a: \"1.5\"\nb: x\n",
        ".a style=\"single\"",
        "a: '1.5'\nb: x\n",
    ),
    (
        "a: \"1.5\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"1.5\"\nb: x\n",
    ),
    (
        "a: \"1.5\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: \"1.5\"\nb: x\n",
    ),
    (
        "a: \"true\"\nb: x\n",
        ".a style=\"\"",
        "a: \"true\"\nb: x\n",
    ),
    (
        "a: \"true\"\nb: x\n",
        ".. style=\"\"",
        "a: \"true\"\nb: x\n",
    ),
    (
        "a: \"true\"\nb: x\n",
        ".a style=\"single\"",
        "a: 'true'\nb: x\n",
    ),
    (
        "a: \"true\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"true\"\nb: x\n",
    ),
    (
        "a: \"true\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: \"true\"\nb: x\n",
    ),
    (
        "a: \"null\"\nb: x\n",
        ".a style=\"\"",
        "a: \"null\"\nb: x\n",
    ),
    (
        "a: \"null\"\nb: x\n",
        ".. style=\"\"",
        "a: \"null\"\nb: x\n",
    ),
    (
        "a: \"null\"\nb: x\n",
        ".a style=\"single\"",
        "a: 'null'\nb: x\n",
    ),
    (
        "a: \"null\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"null\"\nb: x\n",
    ),
    (
        "a: \"null\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: \"null\"\nb: x\n",
    ),
    ("a: \"~\"\nb: x\n", ".a style=\"\"", "a: \"~\"\nb: x\n"),
    ("a: \"~\"\nb: x\n", ".. style=\"\"", "a: \"~\"\nb: x\n"),
    ("a: \"~\"\nb: x\n", ".a style=\"single\"", "a: '~'\nb: x\n"),
    (
        "a: \"~\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"~\"\nb: x\n",
    ),
    (
        "a: \"~\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: \"~\"\nb: x\n",
    ),
    (
        "a: \"0x1F\"\nb: x\n",
        ".a style=\"\"",
        "a: \"0x1F\"\nb: x\n",
    ),
    (
        "a: \"0x1F\"\nb: x\n",
        ".. style=\"\"",
        "a: \"0x1F\"\nb: x\n",
    ),
    (
        "a: \"0x1F\"\nb: x\n",
        ".a style=\"single\"",
        "a: '0x1F'\nb: x\n",
    ),
    (
        "a: \"0x1F\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"0x1F\"\nb: x\n",
    ),
    (
        "a: \"0x1F\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: \"0x1F\"\nb: x\n",
    ),
    (
        "a: \"1_000\"\nb: x\n",
        ".a style=\"\"",
        "a: \"1_000\"\nb: x\n",
    ),
    (
        "a: \"1_000\"\nb: x\n",
        ".. style=\"\"",
        "a: \"1_000\"\nb: x\n",
    ),
    (
        "a: \"1_000\"\nb: x\n",
        ".a style=\"single\"",
        "a: '1_000'\nb: x\n",
    ),
    (
        "a: \"1_000\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"1_000\"\nb: x\n",
    ),
    (
        "a: \"1_000\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: \"1_000\"\nb: x\n",
    ),
    (
        "a: '2023-07-12'\nb: x\n",
        ".a style=\"\"",
        "a: \"2023-07-12\"\nb: x\n",
    ),
    (
        "a: '2023-07-12'\nb: x\n",
        ".. style=\"\"",
        "a: \"2023-07-12\"\nb: x\n",
    ),
    (
        "a: '2023-07-12'\nb: x\n",
        ".a style=\"single\"",
        "a: '2023-07-12'\nb: x\n",
    ),
    (
        "a: '2023-07-12'\nb: x\n",
        ".a style=\"double\"",
        "a: \"2023-07-12\"\nb: x\n",
    ),
    (
        "a: '2023-07-12'\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: \"2023-07-12\"\nb: x\n",
    ),
    (
        "a: \"12:30:45\"\nb: x\n",
        ".a style=\"\"",
        "a: 12:30:45\nb: x\n",
    ),
    (
        "a: \"12:30:45\"\nb: x\n",
        ".. style=\"\"",
        "a: 12:30:45\nb: x\n",
    ),
    (
        "a: \"12:30:45\"\nb: x\n",
        ".a style=\"single\"",
        "a: '12:30:45'\nb: x\n",
    ),
    (
        "a: \"12:30:45\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"12:30:45\"\nb: x\n",
    ),
    (
        "a: \"12:30:45\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: 12:30:45\nb: x\n",
    ),
    ("a: \"yes\"\nb: x\n", ".a style=\"\"", "a: yes\nb: x\n"),
    ("a: \"yes\"\nb: x\n", ".. style=\"\"", "a: yes\nb: x\n"),
    (
        "a: \"yes\"\nb: x\n",
        ".a style=\"single\"",
        "a: 'yes'\nb: x\n",
    ),
    (
        "a: \"yes\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"yes\"\nb: x\n",
    ),
    (
        "a: \"yes\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: yes\nb: x\n",
    ),
    ("a: \"08\"\nb: x\n", ".a style=\"\"", "a: \"08\"\nb: x\n"),
    ("a: \"08\"\nb: x\n", ".. style=\"\"", "a: \"08\"\nb: x\n"),
    (
        "a: \"08\"\nb: x\n",
        ".a style=\"single\"",
        "a: '08'\nb: x\n",
    ),
    (
        "a: \"08\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"08\"\nb: x\n",
    ),
    (
        "a: \"08\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: \"08\"\nb: x\n",
    ),
    ("a: \"x y\"\nb: x\n", ".a style=\"\"", "a: x y\nb: x\n"),
    ("a: \"x y\"\nb: x\n", ".. style=\"\"", "a: x y\nb: x\n"),
    (
        "a: \"x y\"\nb: x\n",
        ".a style=\"single\"",
        "a: 'x y'\nb: x\n",
    ),
    (
        "a: \"x y\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"x y\"\nb: x\n",
    ),
    (
        "a: \"x y\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: x y\nb: x\n",
    ),
    ("a: \"a: b\"\nb: x\n", ".a style=\"\"", "a: 'a: b'\nb: x\n"),
    ("a: \"a: b\"\nb: x\n", ".. style=\"\"", "a: 'a: b'\nb: x\n"),
    (
        "a: \"a: b\"\nb: x\n",
        ".a style=\"single\"",
        "a: 'a: b'\nb: x\n",
    ),
    (
        "a: \"a: b\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"a: b\"\nb: x\n",
    ),
    (
        "a: \"a: b\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: 'a: b'\nb: x\n",
    ),
    ("a: \"#c\"\nb: x\n", ".a style=\"\"", "a: '#c'\nb: x\n"),
    ("a: \"#c\"\nb: x\n", ".. style=\"\"", "a: '#c'\nb: x\n"),
    (
        "a: \"#c\"\nb: x\n",
        ".a style=\"single\"",
        "a: '#c'\nb: x\n",
    ),
    (
        "a: \"#c\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"#c\"\nb: x\n",
    ),
    (
        "a: \"#c\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: '#c'\nb: x\n",
    ),
    ("a: \"\"\nb: x\n", ".a style=\"\"", "a: \"\"\nb: x\n"),
    ("a: \"\"\nb: x\n", ".. style=\"\"", "a: \"\"\nb: x\n"),
    ("a: \"\"\nb: x\n", ".a style=\"single\"", "a: ''\nb: x\n"),
    ("a: \"\"\nb: x\n", ".a style=\"double\"", "a: \"\"\nb: x\n"),
    (
        "a: \"\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: \"\"\nb: x\n",
    ),
    (
        "a: 2023-07-12\nb: x\n",
        ".a style=\"\"",
        "a: 2023-07-12\nb: x\n",
    ),
    (
        "a: 2023-07-12\nb: x\n",
        ".. style=\"\"",
        "a: 2023-07-12\nb: x\n",
    ),
    (
        "a: 2023-07-12\nb: x\n",
        ".a style=\"single\"",
        "a: '2023-07-12'\nb: x\n",
    ),
    (
        "a: 2023-07-12\nb: x\n",
        ".a style=\"double\"",
        "a: \"2023-07-12\"\nb: x\n",
    ),
    (
        "a: 2023-07-12\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: 2023-07-12\nb: x\n",
    ),
    ("a: 1_000\nb: x\n", ".a style=\"\"", "a: 1_000\nb: x\n"),
    ("a: 1_000\nb: x\n", ".. style=\"\"", "a: 1_000\nb: x\n"),
    (
        "a: 1_000\nb: x\n",
        ".a style=\"single\"",
        "a: '1_000'\nb: x\n",
    ),
    (
        "a: 1_000\nb: x\n",
        ".a style=\"double\"",
        "a: \"1_000\"\nb: x\n",
    ),
    (
        "a: 1_000\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: 1_000\nb: x\n",
    ),
    ("a: plain\nb: x\n", ".a style=\"\"", "a: plain\nb: x\n"),
    ("a: plain\nb: x\n", ".. style=\"\"", "a: plain\nb: x\n"),
    (
        "a: plain\nb: x\n",
        ".a style=\"single\"",
        "a: 'plain'\nb: x\n",
    ),
    (
        "a: plain\nb: x\n",
        ".a style=\"double\"",
        "a: \"plain\"\nb: x\n",
    ),
    (
        "a: plain\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: plain\nb: x\n",
    ),
    ("a: 'plain'\nb: x\n", ".a style=\"\"", "a: plain\nb: x\n"),
    ("a: 'plain'\nb: x\n", ".. style=\"\"", "a: plain\nb: x\n"),
    (
        "a: 'plain'\nb: x\n",
        ".a style=\"single\"",
        "a: 'plain'\nb: x\n",
    ),
    (
        "a: 'plain'\nb: x\n",
        ".a style=\"double\"",
        "a: \"plain\"\nb: x\n",
    ),
    (
        "a: 'plain'\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: plain\nb: x\n",
    ),
    (
        "a: \"2023-07-12\" # c\nb: x\n",
        ".a style=\"\"",
        "a: \"2023-07-12\" # c\nb: x\n",
    ),
    (
        "a: \"2023-07-12\" # c\nb: x\n",
        ".. style=\"\"",
        "a: \"2023-07-12\" # c\nb: x\n",
    ),
    (
        "a: \"2023-07-12\" # c\nb: x\n",
        ".a style=\"single\"",
        "a: '2023-07-12' # c\nb: x\n",
    ),
    (
        "a: \"2023-07-12\" # c\nb: x\n",
        ".a style=\"double\"",
        "a: \"2023-07-12\" # c\nb: x\n",
    ),
    (
        "a: \"2023-07-12\" # c\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: \"2023-07-12\" # c\nb: x\n",
    ),
    (
        "a: \"0b11\"\nb: x\n",
        ".a style=\"\"",
        "a: \"0b11\"\nb: x\n",
    ),
    (
        "a: \"0b11\"\nb: x\n",
        ".. style=\"\"",
        "a: \"0b11\"\nb: x\n",
    ),
    (
        "a: \"0b11\"\nb: x\n",
        ".a style=\"single\"",
        "a: '0b11'\nb: x\n",
    ),
    (
        "a: \"0b11\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"0b11\"\nb: x\n",
    ),
    (
        "a: \"0b11\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: \"0b11\"\nb: x\n",
    ),
    (
        "a: \"2001-12-14 21:59:43.10 -5\"\nb: x\n",
        ".a style=\"\"",
        "a: 2001-12-14 21:59:43.10 -5\nb: x\n",
    ),
    (
        "a: \"2001-12-14 21:59:43.10 -5\"\nb: x\n",
        ".. style=\"\"",
        "a: 2001-12-14 21:59:43.10 -5\nb: x\n",
    ),
    (
        "a: \"2001-12-14 21:59:43.10 -5\"\nb: x\n",
        ".a style=\"single\"",
        "a: '2001-12-14 21:59:43.10 -5'\nb: x\n",
    ),
    (
        "a: \"2001-12-14 21:59:43.10 -5\"\nb: x\n",
        ".a style=\"double\"",
        "a: \"2001-12-14 21:59:43.10 -5\"\nb: x\n",
    ),
    (
        "a: \"2001-12-14 21:59:43.10 -5\"\nb: x\n",
        "(.a | select(tag == \"!!str\")) style=\"\"",
        "a: 2001-12-14 21:59:43.10 -5\nb: x\n",
    ),
];

#[test]
fn clearing_the_style_of_a_quoted_typed_spelling_keeps_the_quotes_6909() -> Result<()> {
    for (input, filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(code, 0, "{filter} over {input:?}");
        assert_eq!(out, *want, "{filter} over {input:?}");
    }
    Ok(())
}
