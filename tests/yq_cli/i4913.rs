//! #4913: a mapping key that holds U+2028 or U+2029 is no simple key (go-yaml's emitter treats both as a
//! line break), so the streaming route writes it as an explicit `? ` key, as the write route
//! already did: `? 'x\u{2028}  y'`, the break followed by the scalar's indent, and a double-quoted key
//! keeps its quotes and its escape.
//!
//! Every row is captured from yq v4.53.3: the source document, the extra arguments, the filter and
//! the expected output.

use anyhow::Result;

use crate::run_yq_stdin;

type Row = (
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
);

const ROWS: &[Row] = &[
    (
        "? 'x\u{2028}y'\n: 1\nb: 2\n",
        &[],
        ".",
        "? 'x\u{2028}  y'\n: 1\nb: 2\n",
    ),
    (
        "b: 2\n? 'x\u{2028}y'\n: 1\n",
        &[],
        ".",
        "b: 2\n? 'x\u{2028}  y'\n: 1\n",
    ),
    (
        "z:\n  ? 'x\u{2028}y'\n  : 1\n",
        &[],
        ".",
        "z:\n  ? 'x\u{2028}    y'\n  : 1\n",
    ),
    (
        "- ? 'x\u{2028}y'\n  : 1\n",
        &[],
        ".",
        "- ? 'x\u{2028}    y'\n  : 1\n",
    ),
    (
        "z:\n  ? 'x\u{2028}y'\n  : 1\n",
        &[],
        ".[]",
        "? 'x\u{2028}  y'\n: 1\n",
    ),
    (
        "? 'x\u{2029}y'\n: 1\nb: 2\n",
        &[],
        ".",
        "? 'x\u{2029}  y'\n: 1\nb: 2\n",
    ),
    (
        "b: 2\n? 'x\u{2029}y'\n: 1\n",
        &[],
        ".",
        "b: 2\n? 'x\u{2029}  y'\n: 1\n",
    ),
    (
        "z:\n  ? 'x\u{2029}y'\n  : 1\n",
        &[],
        ".",
        "z:\n  ? 'x\u{2029}    y'\n  : 1\n",
    ),
    (
        "- ? 'x\u{2029}y'\n  : 1\n",
        &[],
        ".",
        "- ? 'x\u{2029}    y'\n  : 1\n",
    ),
    (
        "z:\n  ? 'x\u{2029}y'\n  : 1\n",
        &[],
        ".[]",
        "? 'x\u{2029}  y'\n: 1\n",
    ),
    (
        "? 'x\u{2028}\u{2028}y'\n: 1\n",
        &[],
        ".",
        "? 'x\u{2028}\u{2028}  y'\n: 1\n",
    ),
    ("? '\u{2028}x'\n: 1\n", &[], ".", "? '\u{2028}  x'\n: 1\n"),
    ("? 'x\u{2028}'\n: 1\n", &[], ".", "? 'x\u{2028}'\n: 1\n"),
    (
        "? &k 'x\u{2028}y'\n: 1\n",
        &[],
        ".",
        "? &k 'x\u{2028}  y'\n: 1\n",
    ),
    (
        "? !!str 'x\u{2028}y'\n: 1\n",
        &[],
        ".",
        "? !!str 'x\u{2028}  y'\n: 1\n",
    ),
    (
        "? 'x\u{2028}y'\n: 1\n",
        &["-I4"],
        ".",
        "? 'x\u{2028}    y'\n: 1\n",
    ),
    (
        "? \"x\u{2028}y\"\n: 1\nb: 2\n",
        &[],
        ".",
        "? \"x\\Ly\"\n: 1\nb: 2\n",
    ),
    (
        "z:\n  ? \"x\u{2028}y\"\n  : 1\n",
        &[],
        ".",
        "z:\n  ? \"x\\Ly\"\n  : 1\n",
    ),
    (
        "- ? \"x\u{2029}y\"\n  : 1\n",
        &[],
        ".",
        "- ? \"x\\Py\"\n  : 1\n",
    ),
    ("? \"x\\u2028y\"\n: 1\n", &[], ".", "? \"x\\Ly\"\n: 1\n"),
    ("'x y': 1\n", &[], ".", "'x y': 1\n"),
];

#[test]
fn a_key_holding_a_unicode_line_separator_is_an_explicit_key_4913() -> Result<()> {
    for (input, extra, filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, input, extra)?;
        assert_eq!(code, 0, "{input:?} {extra:?} {filter}");
        assert_eq!(out, *want, "{input:?} {extra:?} {filter}");
    }
    Ok(())
}

/// `keys` writes such a key as a list item whose scalar continues two columns in, whatever `-I` says.
const KEYS_ROWS: &[Row] = &[
    (
        "? 'x\u{2028}y'\n: 1\nb: 2\n",
        &[],
        "keys",
        "- 'x\u{2028}  y'\n- b\n",
    ),
    (
        "z:\n  ? 'x\u{2028}y'\n  : 1\n",
        &[],
        ".z | keys",
        "- 'x\u{2028}  y'\n",
    ),
    (
        "? 'x\u{2029}y'\n: 1\nb: 2\n",
        &[],
        "keys",
        "- 'x\u{2029}  y'\n- b\n",
    ),
    (
        "z:\n  ? 'x\u{2029}y'\n  : 1\n",
        &[],
        ".z | keys",
        "- 'x\u{2029}  y'\n",
    ),
    (
        "? 'x\u{2028}y'\n: 1\n",
        &["-I4"],
        "keys",
        "- 'x\u{2028}  y'\n",
    ),
    (
        "? 'x\u{2028}y'\n: 1\n",
        &["-I1"],
        "keys",
        "- 'x\u{2028}  y'\n",
    ),
    (
        "? 'x\u{2028}\u{2028}y'\n: 1\n",
        &[],
        "keys",
        "- 'x\u{2028}\u{2028}  y'\n",
    ),
    ("? '\u{2028}x'\n: 1\n", &[], "keys", "- '\u{2028}  x'\n"),
    (
        "? &k 'x\u{2028}y'\n: 1\n",
        &[],
        "keys",
        "- &k 'x\u{2028}  y'\n",
    ),
    (
        "? !!str 'x\u{2028}y'\n: 1\n",
        &[],
        "keys",
        "- !!str 'x\u{2028}  y'\n",
    ),
    (
        "? 'a b\u{2028}c d'\n: 1\n",
        &[],
        "keys",
        "- 'a b\u{2028}  c d'\n",
    ),
];

#[test]
fn keys_writes_a_key_holding_a_unicode_line_separator_as_a_multi_line_item_4913() -> Result<()> {
    for (input, extra, filter, want) in KEYS_ROWS {
        let (out, code) = run_yq_stdin(filter, input, extra)?;
        assert_eq!(code, 0, "{input:?} {extra:?} {filter}");
        assert_eq!(out, *want, "{input:?} {extra:?} {filter}");
    }
    Ok(())
}
