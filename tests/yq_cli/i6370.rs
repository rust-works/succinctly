//! #6370: a block sequence at a bare `?`'s own column is the key's content (YAML 1.2 lets a block sequence
//! sit at its indicator's indent), so the key is that sequence and the `:` line is the value. It was read as the
//! value of an empty key, and the `: v` line then joined it (`?⏎- a⏎: v` was `{"":["a","v"]}`). Each row is a live yq
//! v4.53.3 capture.

use crate::run_yq_stdin;

/// (document, `-o json -I0 .` output).
const ROWS: &[(&str, &str)] = &[
    ("?\n- a\n: v\n", "{\"\":\"v\"}\n"),
    ("?\n- a\n- b\n: v\nk: w\n", "{\"\":\"v\",\"k\":\"w\"}\n"),
    ("?\n- a\n? b\n: c\n", "{\"\":null,\"b\":\"c\"}\n"),
    ("?\n- a\n", "{\"\":null}\n"),
    ("?\n- a\nk: w\n", "{\"\":null,\"k\":\"w\"}\n"),
    (
        "x:\n  ?\n  - a\n  : v\n  y: 1\n",
        "{\"x\":{\"\":\"v\",\"y\":1}}\n",
    ),
    ("?\n- k: a\n  j: b\n: v\n", "{\"\":\"v\"}\n"),
    ("?\n- - a\n  - b\n: v\n", "{\"\":\"v\"}\n"),
    // The value after the `:` is its own sequence.
    ("?\n- a\n:\n- x\n", "{\"\":[\"x\"]}\n"),
    ("?\n- a\n: v\n? b\n: c\n", "{\"\":\"v\",\"b\":\"c\"}\n"),
    ("? \n- a\n: v\n", "{\"\":\"v\"}\n"),
    ("- ?\n  - a\n  : v\n", "[{\"\":\"v\"}]\n"),
    // Properties, comments and CRLF around the key.
    ("? &x\n- a\n: v\n", "{\"\":\"v\"}\n"),
    ("? !!seq\n- a\n: v\n", "{\"\":\"v\"}\n"),
    ("? # c\n- a\n: v\n", "{\"\":\"v\"}\n"),
    ("?\n# c\n- a\n: v\n", "{\"\":\"v\"}\n"),
    ("?\n- a\n# c\n: v\n", "{\"\":\"v\"}\n"),
    ("?\r\n- a\r\n: v\r\n", "{\"\":\"v\"}\n"),
    // The sequence one column in was the key's content already, and a non-empty key still refuses a
    // same-column sequence (#1104).
    ("?\n - a\n - b\n: v\n", "{\"\":\"v\"}\n"),
    ("? - a\n  - b\n: v\n", "{\"\":\"v\"}\n"),
];

#[test]
fn test_a_block_sequence_at_the_question_marks_column_is_the_key_6370() {
    for (doc, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(".", doc, &["-o", "json", "-I0"]).unwrap();
        assert_eq!((stdout.as_str(), code), (*expected, 0), "{doc:?}");
    }
}

#[test]
fn test_a_nonempty_explicit_key_still_refuses_a_same_column_sequence_6370() {
    let (stdout, code) = run_yq_stdin(".", "? k\n- a\n: v\n", &["-o", "json", "-I0"]).unwrap();
    assert_eq!((stdout.as_str(), code), ("", 1));
}
