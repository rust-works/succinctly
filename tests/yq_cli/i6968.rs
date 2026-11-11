//! #6968: a `#` that touches the closing quote of a quoted scalar starts a comment in block context, as it
//! does in yq (`a: "x"#c`). The check that a mapping value or sequence item is the last thing on its line
//! accepted a comment only after a blank, and the quote that precedes the `#` is not one. After a plain
//! scalar the same `#` is still content (`a: x#c` is `x#c`), and a comment between a quoted key and its
//! colon is still refused.
//!
//! Every row is captured from yq v4.53.3 (`-o=json -I=0 .`, and the YAML output of three of them).

use anyhow::Result;

use crate::run_yq_stdin;

/// Accepted by yq: `(document, expected JSON output)`.
const ACCEPTED: &[(&str, &str)] = &[
    ("a: \"x\"#c\n", "{\"a\":\"x\"}\n"),
    ("a: 'x'#c\n", "{\"a\":\"x\"}\n"),
    ("- \"x\"#c\n", "[\"x\"]\n"),
    ("a: \"x\"#c\nb: 1\n", "{\"a\":\"x\",\"b\":1}\n"),
    ("a: &x \"v\"#c\n", "{\"a\":\"v\"}\n"),
    ("a: !!str \"v\"#c\n", "{\"a\":\"v\"}\n"),
    ("? \"x\"#c\n: 1\n", "{\"x\":1}\n"),
    ("a: \"x\"#\n", "{\"a\":\"x\"}\n"),
    ("a: \"x\"# c d\nb: \"y\"#e\n", "{\"a\":\"x\",\"b\":\"y\"}\n"),
    ("\"k\"#c: 1\n", "\"k\"\n"),
    ("- \"a\"#c\n- \"b\"#d\n", "[\"a\",\"b\"]\n"),
    ("a:\n  - \"x\"#c\n  - y\n", "{\"a\":[\"x\",\"y\"]}\n"),
    ("a: \"x\"#c\n#d\nb: 2\n", "{\"a\":\"x\",\"b\":2}\n"),
    ("\"x\"#c\n", "\"x\"\n"),
    ("[ \"x\"#c\n]\n", "[\"x\"]\n"),
    ("{a: \"x\"#c\n}\n", "{\"a\":\"x\"}\n"),
    ("a: x#c\n", "{\"a\":\"x#c\"}\n"),
    ("a: say \"hi\"#c\n", "{\"a\":\"say \\\"hi\\\"#c\"}\n"),
    ("a: it's#c\n", "{\"a\":\"it's#c\"}\n"),
    ("a: \"x\"\t#c\n", "{\"a\":\"x\"}\n"),
];

/// Refused by yq.
const REFUSED: &[&str] = &["a: \"x\"#c\n  b: 1\n", "\"k\"#c\n: 1\n"];

/// YAML output keeps the comment: `(document, extra arguments, expected)`.
const YAML_ROWS: &[(&str, &[&str], &str)] = &[
    ("a: \"x\"#c\nb: 1\n", &[], "a: \"x\" #c\nb: 1\n"),
    ("a: \"x\"#c\nb: 1\n", &["-P"], "a: x #c\nb: 1\n"),
    ("- 'x'#c\n- y\n", &[], "- 'x' #c\n- y\n"),
    ("- 'x'#c\n- y\n", &["-P"], "- x #c\n- y\n"),
    ("a: &x \"v\"#c\n", &[], "a: &x \"v\" #c\n"),
    ("a: &x \"v\"#c\n", &["-P"], "a: &x v #c\n"),
];

#[test]
fn a_hash_touching_a_closing_quote_starts_a_comment_6968() -> Result<()> {
    for (doc, want) in ACCEPTED {
        let (out, code) = run_yq_stdin(".", doc, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{doc:?}");
        assert_eq!(out, *want, "{doc:?}");
    }
    Ok(())
}

#[test]
fn a_comment_that_yq_refuses_after_a_quoted_scalar_is_still_refused_6968() -> Result<()> {
    for doc in REFUSED {
        let (_, code) = run_yq_stdin(".", doc, &["-o=json", "-I=0"])?;
        assert_ne!(code, 0, "{doc:?}");
    }
    Ok(())
}

#[test]
fn a_comment_glued_to_a_closing_quote_prints_back_as_yq_does_6968() -> Result<()> {
    for (doc, extra, want) in YAML_ROWS {
        let (out, code) = run_yq_stdin(".", doc, extra)?;
        assert_eq!(code, 0, "{doc:?} {extra:?}");
        assert_eq!(out, *want, "{doc:?} {extra:?}");
    }
    Ok(())
}
