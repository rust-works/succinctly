//! #5590 is a jq-mode parser fix. yq reads a `\u` in a double-quoted expression literally and a
//! keyword after the dot as a field name, so `succinctly yq` must keep doing both.

use crate::run_yq_stdin;

#[test]
fn test_yq_keyword_fields_and_literal_unicode_escape_unchanged_5590() {
    let (out, code) = run_yq_stdin(".then + .and", "then: 1\nand: 2\n", &[]).unwrap();
    assert_eq!((out.as_str(), code), ("3\n", 0));
    // yq keeps the backslash: the JSON output escapes it, so two appear.
    let (out, code) = run_yq_stdin(r#""\ud83d\ude00""#, "a: 1\n", &["-o=json", "-I0"]).unwrap();
    assert_eq!((out.as_str(), code), ("\"\\\\ud83d\\\\ude00\"\n", 0));
}
