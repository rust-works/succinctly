//! #7092: `-o=shell` decomposes a key with NFKD before it keeps `[A-Za-z0-9_]`, writes the rest of the
//! printable ASCII as `_` and drops everything else. Every answer was captured from yq v4.53.3, which
//! agrees with this for every code point from U+0080 to U+10FFFF (a sweep, not a sample).

use crate::run_yq_stdin;

/// (key, yq's `-o=shell` name for it, with the value `1`).
const ROWS: &[(&str, &str)] = &[
    ("effort_expeñded_tò_preserve_accented_latin_letters", "effort_expended_to_preserve_accented_latin_letters"),
    ("é", "e"),
    ("ñandú", "nandu"),
    ("straße", "strae"),
    ("ﬁ", "fi"),
    ("１", "_1"),
    ("Ａｚ", "Az"),
    ("𝐀𝐛", "Ab"),
    ("²³", "_23"),
    ("a…b", "a___b"),
    ("a\u{a0}b", "a_b"),
    ("№", "No"),
    ("א日本", "_"),
    ("a\u{301}", "a"),
];

#[test]
fn shell_keys_are_decomposed_with_nfkd_7092() {
    for &(key, name) in ROWS {
        let input = format!("{}: 1\n", serde_json_key(key));
        let (out, code) = run_yq_stdin(".", &input, &["-o=shell"]).expect("runs");
        assert_eq!((out, code), (format!("{name}=1\n"), 0), "key {key:?}");
    }
}

/// `key` as a double-quoted YAML key (a backslash and a quote are the only escapes the rows need).
fn serde_json_key(key: &str) -> String {
    format!("\"{}\"", key.replace('\\', "\\\\").replace('"', "\\\""))
}
