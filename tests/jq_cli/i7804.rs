//! #7804: under the `x` flag a `#` inside a class, and a Unicode blank there (U+3000), is a literal
//! in Oniguruma, where the `regex` crate reads the `#` as a comment and drops the blank. The class did
//! not parse, the pattern went to `fancy-regex`, and that route left out jq's one-byte step after an
//! empty match (#6326): `"\u00e9" | [match("[#]?"; "gx")]` had two matches where jq has three. Each
//! row is captured from jq 1.7.1 (`/usr/bin/jq`), and jq 1.8.2 gives the same: 21 of the 29 fail
//! before the change, and the last five are controls.

use anyhow::Result;

use crate::run_jq_full;

/// The spans of every match of `$r.re` under `$r.f` in `$r.s`, with each match's group count.
const PROGRAM: &str = ". as $r | $r.s | try ([match($r.re; $r.f) | [.offset, .length, (.captures | length)]] | tojson) catch .";

/// `(row, jq 1.7.1's output)`.
const ROWS: &[(&str, &str)] = &[
    (
        r#"{"s": "\u00e9", "re": "[#]?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9\u00e9", "re": "[#]?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0],[2,0,0],[2,0,0]]""#,
    ),
    (
        r#"{"s": "\u20ac", "re": "[#]?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0],[1,0,0]]""#,
    ),
    (
        r#"{"s": "\ud83d\ude00", "re": "[#]?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0],[1,0,0],[1,0,0]]""#,
    ),
    (
        r#"{"s": "e\u0301", "re": "[#]?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[2,0,0],[2,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9", "re": "[#]*", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9", "re": "[#a]?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9", "re": "(?:[#])?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9", "re": "(?x)[#]?", "f": "g"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9#\u00e9", "re": "[#]?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,1,0],[2,0,0],[3,0,0],[3,0,0]]""#,
    ),
    (
        r#"{"s": "a\u00e9", "re": "[#]{0,1}", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[2,0,0],[2,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9", "re": "[#]{0}", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9 a", "re": "[# ]?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,1,0],[2,0,0],[3,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9", "re": "[\u3000]?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9\u3000", "re": "[\u3000]?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,1,0],[2,0,0]]""#,
    ),
    (
        r#"{"s": "\u00a0\u00e9", "re": "[\u00a0]*", "f": "gx"}"#,
        r#""[[0,1,0],[1,0,0],[2,0,0],[2,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9", "re": "[^ #]?", "f": "gx"}"#,
        r#""[[0,1,0],[1,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9#", "re": "[#-#]?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,1,0],[2,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9", "re": "[]#]?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9a", "re": "[\\w#]?", "f": "gx"}"#,
        r#""[[0,1,0],[1,1,0],[2,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9#", "re": "[#]?", "f": "gxn"}"#,
        r#""[[1,1,0]]""#,
    ),
    (
        r#"{"s": "\u00e9", "re": "[#]?", "f": "gxl"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9", "re": "([#])?", "f": "gx"}"#,
        r#""[[0,0,1],[1,0,1],[1,0,1]]""#,
    ),
    (
        r#"{"s": "\u00e9", "re": "(?x)([#]){0}", "f": "g"}"#,
        r#""[[0,0,1],[1,0,1],[1,0,1]]""#,
    ),
    (
        r#"{"s": "\u00e9", "re": "[#]?", "f": "g"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9", "re": "[\\#]?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9", "re": "\\#?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9", "re": "[a]?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0]]""#,
    ),
    (
        r#"{"s": "\u00e9", "re": "[ ]?", "f": "gx"}"#,
        r#""[[0,0,0],[1,0,0],[1,0,0]]""#,
    ),
];

#[test]
fn a_hash_or_unicode_blank_in_a_class_under_x_keeps_the_byte_step_7804() -> Result<()> {
    let input: String = ROWS.iter().map(|(row, _)| format!("{row}\n")).collect();
    let (stdout, stderr, code) = run_jq_full(&["-c", PROGRAM], Some(&input))?;
    assert_eq!((code, stderr.as_str()), (0, ""));
    let got: Vec<&str> = stdout.lines().collect();
    assert_eq!(got.len(), ROWS.len(), "{stdout}");
    for ((row, expected), got) in ROWS.iter().zip(got) {
        assert_eq!(got, *expected, "{row}");
    }
    Ok(())
}
