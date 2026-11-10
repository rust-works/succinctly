//! #6820 (the block scalar header's line): go-yaml ends a block scalar's header line at U+0085,
//! U+2028 and U+2029 as at `\n`, so `a: |<NEL>  x` has its text on the next "line" and the scalar is
//! `"x\n"`; the index ended the header at `\n` and `\r` alone, so the value came out empty (`"x\n"`
//! lost, or the whole text read as the header's comment). Every answer below was captured from yq
//! v4.53.3 (`-o=json -I=0`); `None` is a document yq refuses. A scalar whose header ends at one of
//! the three is listed in the index like one that holds the break in its body (#6369), so the
//! decoders end the header at the same byte.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, yq's answer or `None` for a refusal).
const ROWS: &[(&str, Option<&str>)] = &[
    ("a: |\u{85}  x\n", Some("{\"a\":\"x\\n\"}")),
    ("a: |\u{85}  x\n  y\n", Some("{\"a\":\"x\\ny\\n\"}")),
    ("a: >\u{85}  x\n  y\n", Some("{\"a\":\"x y\\n\"}")),
    ("a: |-\u{85}  x\n", Some("{\"a\":\"x\"}")),
    ("a: |+\u{85}  x\n\n", Some("{\"a\":\"x\\n\\n\"}")),
    ("a: |2\u{85}   x\n", Some("{\"a\":\" x\\n\"}")),
    ("a: | \u{85}  x\n", Some("{\"a\":\"x\\n\"}")),
    ("a: |  \u{85}  x\nb: 1\n", Some("{\"a\":\"x\\n\",\"b\":1}")),
    ("a: | # c\u{85}  x\n", Some("{\"a\":\"x\\n\"}")),
    ("a: |\u{85}\n  x\n", Some("{\"a\":\"\\nx\\n\"}")),
    ("a: |\u{85}b: 1\n", Some("{\"a\":\"\",\"b\":1}")),
    ("- |\u{85}  x\n- y\n", Some("[\"x\\n\",\"y\"]")),
    (
        "a:\n  b: |\u{85}    x\n  c: 1\n",
        Some("{\"a\":{\"b\":\"x\\n\",\"c\":1}}"),
    ),
    ("a: &x |\u{85}  x\n", Some("{\"a\":\"x\\n\"}")),
    ("a: |\u{85}  x\u{85}  y\n", Some("{\"a\":\"x\\ny\\n\"}")),
    ("a: |\u{85}\n", Some("{\"a\":\"\"}")),
    ("a: |\u{85}", Some("{\"a\":\"\"}")),
    ("a: |\u{85}  x\nb: 1\n", Some("{\"a\":\"x\\n\",\"b\":1}")),
    ("a: |\u{85}x\n", None),
    ("a: |\u{2028}  x\n", Some("{\"a\":\"x\\n\"}")),
    ("a: |\u{2028}  x\n  y\n", Some("{\"a\":\"x\\ny\\n\"}")),
    ("a: >\u{2028}  x\n  y\n", Some("{\"a\":\"x y\\n\"}")),
    ("a: |-\u{2028}  x\n", Some("{\"a\":\"x\"}")),
    ("a: |+\u{2028}  x\n\n", Some("{\"a\":\"x\\n\\n\"}")),
    ("a: |2\u{2028}   x\n", Some("{\"a\":\" x\\n\"}")),
    ("a: | \u{2028}  x\n", Some("{\"a\":\"x\\n\"}")),
    (
        "a: |  \u{2028}  x\nb: 1\n",
        Some("{\"a\":\"x\\n\",\"b\":1}"),
    ),
    ("a: | # c\u{2028}  x\n", Some("{\"a\":\"x\\n\"}")),
    ("a: |\u{2028}\n  x\n", Some("{\"a\":\"\\nx\\n\"}")),
    ("a: |\u{2028}b: 1\n", Some("{\"a\":\"\",\"b\":1}")),
    ("- |\u{2028}  x\n- y\n", Some("[\"x\\n\",\"y\"]")),
    (
        "a:\n  b: |\u{2028}    x\n  c: 1\n",
        Some("{\"a\":{\"b\":\"x\\n\",\"c\":1}}"),
    ),
    ("a: &x |\u{2028}  x\n", Some("{\"a\":\"x\\n\"}")),
    (
        "a: |\u{2028}  x\u{2028}  y\n",
        Some("{\"a\":\"x\\u2028y\\n\"}"),
    ),
    ("a: |\u{2028}\n", Some("{\"a\":\"\"}")),
    ("a: |\u{2028}", Some("{\"a\":\"\"}")),
    ("a: |\u{2028}  x\nb: 1\n", Some("{\"a\":\"x\\n\",\"b\":1}")),
    ("a: |\u{2028}x\n", None),
    ("a: |\u{2029}  x\n", Some("{\"a\":\"x\\n\"}")),
    ("a: |\u{2029}  x\n  y\n", Some("{\"a\":\"x\\ny\\n\"}")),
    ("a: >\u{2029}  x\n  y\n", Some("{\"a\":\"x y\\n\"}")),
    ("a: |-\u{2029}  x\n", Some("{\"a\":\"x\"}")),
    ("a: |+\u{2029}  x\n\n", Some("{\"a\":\"x\\n\\n\"}")),
    ("a: |2\u{2029}   x\n", Some("{\"a\":\" x\\n\"}")),
    ("a: | \u{2029}  x\n", Some("{\"a\":\"x\\n\"}")),
    (
        "a: |  \u{2029}  x\nb: 1\n",
        Some("{\"a\":\"x\\n\",\"b\":1}"),
    ),
    ("a: | # c\u{2029}  x\n", Some("{\"a\":\"x\\n\"}")),
    ("a: |\u{2029}\n  x\n", Some("{\"a\":\"\\nx\\n\"}")),
    ("a: |\u{2029}b: 1\n", Some("{\"a\":\"\",\"b\":1}")),
    ("- |\u{2029}  x\n- y\n", Some("[\"x\\n\",\"y\"]")),
    (
        "a:\n  b: |\u{2029}    x\n  c: 1\n",
        Some("{\"a\":{\"b\":\"x\\n\",\"c\":1}}"),
    ),
    ("a: &x |\u{2029}  x\n", Some("{\"a\":\"x\\n\"}")),
    (
        "a: |\u{2029}  x\u{2029}  y\n",
        Some("{\"a\":\"x\\u2029y\\n\"}"),
    ),
    ("a: |\u{2029}\n", Some("{\"a\":\"\"}")),
    ("a: |\u{2029}", Some("{\"a\":\"\"}")),
    ("a: |\u{2029}  x\nb: 1\n", Some("{\"a\":\"x\\n\",\"b\":1}")),
    ("a: |\u{2029}x\n", None),
    // yq refuses text after the break on the header's own line when it is not a comment
    ("a: |\u{85}garbage\n  x\n", None),
    ("a: |\u{2028}garbage\n  x\n", None),
    ("a: |\u{2029}garbage\n  x\n", None),
];

#[test]
fn a_block_scalar_header_ends_at_a_unicode_break_as_in_go_yaml_6820() -> Result<()> {
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
