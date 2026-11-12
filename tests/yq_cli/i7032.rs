//! #7032: a mapping key written as a literal block scalar (`? |-`) is an explicit key that yq
//! writes back as one: it cannot stand as a simple key when its text has a line break or is longer
//! than 128 bytes, and it is double-quoted when it is one short line. The key keeps its literal
//! style and its anchor and tag, and the value follows on the `:` line as it does for any explicit
//! key. Every row is `yq FLAGS .` on the document, captured from yq v4.53.3.
//! `docs/compliance/yq/limitations.md` records the keys that still differ.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, flags, yq's output), each with exit status 0.
const ROWS: &[(&str, &[&str], &str)] = &[
    ("? |-\n  a\n  b\n: \"1\"\n", &[], "? |-\n  a\n  b\n: \"1\"\n"),
    ("? |-\n  a\n  b\n: \"1\"\n", &["-I=4"], "? |-\n    a\n    b\n: \"1\"\n"),
    ("? |-\n  a\n  b\n: \"1\"\n", &["-I=3"], "? |-\n   a\n   b\n: \"1\"\n"),
    ("? |-\n  a\n  b\n: \"1\"\n", &["-I=1"], "? |-\n  a\n  b\n: \"1\"\n"),
    ("? |\n  a\n  b\n: 1\n", &[], "? |\n  a\n  b\n: 1\n"),
    ("? |\n  a\n  b\n: 1\n", &["-I=4"], "? |\n    a\n    b\n: 1\n"),
    ("? |-\n  a\n  b\n: c: \"1\"\n", &[], "? |-\n  a\n  b\n: c: \"1\"\n"),
    ("? |-\n  a\n  b\n: c: \"1\"\n", &["-I=4"], "? |-\n    a\n    b\n:   c: \"1\"\n"),
    ("? |-\n  a\n  b\n: [1, 2]\n", &[], "? |-\n  a\n  b\n: [1, 2]\n"),
    ("? |-\n  a\n  b\n:\n  - x\n  - y\n", &[], "? |-\n  a\n  b\n: - x\n  - y\n"),
    ("? |-\n  a\n  b\n:\n  - x\n  - y\n", &["-I=4"], "? |-\n    a\n    b\n:   - x\n    - y\n"),
    ("x: 1\n? |-\n  a\n  b\n: 2\ny: 3\n", &[], "x: 1\n? |-\n  a\n  b\n: 2\ny: 3\n"),
    ("x: 1\n? |-\n  a\n  b\n: 2\ny: 3\n", &["-I=4"], "x: 1\n? |-\n    a\n    b\n: 2\ny: 3\n"),
    ("- ? |-\n    a\n    b\n  : 1\n", &[], "- ? |-\n    a\n    b\n  : 1\n"),
    ("a:\n  ? |-\n    a\n    b\n  : 1\n", &[], "a:\n  ? |-\n    a\n    b\n  : 1\n"),
    ("a:\n  ? |-\n    a\n    b\n  : 1\n", &["-I=4"], "a:\n    ? |-\n        a\n        b\n    : 1\n"),
    ("? |-\n  a\n  b\n:\n", &[], "? |-\n  a\n  b\n:\n"),
    ("? |2-\n   a\n   b\n: 1\n", &[], "? |2-\n   a\n   b\n: 1\n"),
    ("? |2-\n   a\n   b\n: 1\n", &["-I=4"], "? |4-\n     a\n     b\n: 1\n"),
    ("? |+\n  a\n  b\n\n: 1\n", &[], "? |+\n  a\n  b\n\n: 1\n"),
    ("? |-\n  a:\n  b\n: 1\n", &[], "? |-\n  a:\n  b\n: 1\n"),
    ("? &k |-\n  a\n  b\n: 1\n", &[], "? &k |-\n  a\n  b\n: 1\n"),
    ("? !!str |-\n  a\n  b\n: 1\n", &[], "? !!str |-\n  a\n  b\n: 1\n"),
    ("? |-\n  a\n: 1\n", &[], "\"a\": 1\n"),
    ("? |\n  a\n: 1\n", &[], "? |\n  a\n: 1\n"),
    ("? |-\n  a b c\n: 1\n", &[], "\"a b c\": 1\n"),
    ("? |-\n  xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n: 1\n", &[], "? |-\n  xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n: 1\n"),
    ("? |-\n  a\n  b\n: \"1\"\n", &["-P"], "? |-\n  a\n  b\n: \"1\"\n"),
    ("\"a\\nb\": 1\n", &[], "? \"a\\nb\"\n: 1\n"),
    ("? \"a\n  b\"\n: 1\n", &[], "\"a b\": 1\n"),
    ("? 'a\n\n  b'\n: 1\n", &[], "? 'a\n\n  b'\n: 1\n"),
];

#[test]
fn a_literal_block_scalar_key_is_written_back_as_an_explicit_key_7032() -> Result<()> {
    for (document, flags, want) in ROWS {
        let (out, code) = run_yq_stdin(".", document, flags)?;
        assert_eq!((out.as_str(), code), (*want, 0), "{document:?} {flags:?}");
    }
    Ok(())
}
