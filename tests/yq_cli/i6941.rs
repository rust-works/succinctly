//! #6941: a flow mapping entry that is only a key (`{k}`, no `:` and no value) ends where its trailing
//! blanks and line breaks begin, and a comment after it (on its line or on a later one) ends the key.
//! The key scanner trimmed spaces and tabs only, so `{k` / `` / `}` kept the line break as part of the key
//! (`"k\n"`), and a `#` after a break was key text. #6855 had made the value scanner trim the break; the
//! key scanner and the explicit-key scanner now share its one definition (`trim_flow_plain_end`).
//!
//! Every row is captured from yq v4.53.3 (`-o=json -I=0 .`, and the YAML output where it is listed).
//! The comment itself is not printed back in flow style (the open `comment after an empty value` row of
//! #6334), so the YAML rows hold no comment.

use anyhow::Result;

use crate::run_yq_stdin;

/// Accepted by yq: `(document, expected JSON output)`.
const ACCEPTED: &[(&str, &str)] = &[
    ("{k\n\n}\n", "{\"k\":null}\n"),
    ("{ k\n \n}\n", "{\"k\":null}\n"),
    ("k: {\n k\n \n }\n", "{\"k\":{\"k\":null}}\n"),
    ("{a, b\n\n}\n", "{\"a\":null,\"b\":null}\n"),
    ("{a\n\n, b}\n", "{\"a\":null,\"b\":null}\n"),
    ("{a\n\n\n}\n", "{\"a\":null}\n"),
    ("{k\r\n\r\n}\r\n", "{\"k\":null}\n"),
    ("{a b\n\n}\n", "{\"a b\":null}\n"),
    ("{a\n b\n\n}\n", "{\"a b\":null}\n"),
    ("{? k\n\n}\n", "{\"k\":null}\n"),
    ("{? a b\n\n, c}\n", "{\"a b\":null,\"c\":null}\n"),
    (
        "{a, b\n\n, c\n\n}\n",
        "{\"a\":null,\"b\":null,\"c\":null}\n",
    ),
    ("{k # c\n}\n", "{\"k\":null}\n"),
    ("{k\n # c\n}\n", "{\"k\":null}\n"),
    ("{k\n\n# c\n}\n", "{\"k\":null}\n"),
    ("{? a # c\n}\n", "{\"a\":null}\n"),
    ("{? a # c\n, b}\n", "{\"a\":null,\"b\":null}\n"),
    ("{a # c\n, b}\n", "{\"a\":null,\"b\":null}\n"),
    ("{k # c\n, j # d\n}\n", "{\"k\":null,\"j\":null}\n"),
    ("[a\n \n]\n", "[\"a\"]\n"),
    ("{ k: v\n\n}\n", "{\"k\":\"v\"}\n"),
    ("{k\n}\n", "{\"k\":null}\n"),
    ("{k }\n", "{\"k\":null}\n"),
    ("{a#b: c}\n", "{\"a#b\":\"c\"}\n"),
    ("{a\t# b\n}\n", "{\"a\":null}\n"),
];

/// Refused by yq (a key that continues to a `:` on a later line, or a comment that swallows the closer).
const REFUSED: &[&str] = &[
    "{k\n\n: v}\n",
    "{a # c\n: 1}\n",
    "{a # c: 1}\n",
    "{a #b: c}\n",
    "[a # c\n: 1]\n",
    "[k\n\n: v]\n",
];

/// Block-style and flow-style YAML output of the blank-line shapes: `(document, extra arguments, expected)`.
const YAML_ROWS: &[(&str, &[&str], &str)] = &[
    ("{k\n\n}\n", &[], "{k: ''}\n"),
    ("{k\n\n}\n", &["-P"], "k:\n"),
    ("{a, b\n\n}\n", &[], "{a: '', b: ''}\n"),
    ("{a, b\n\n}\n", &["-P"], "a:\nb:\n"),
    ("k: {\n k\n \n }\n", &[], "k: {k: ''}\n"),
    ("k: {\n k\n \n }\n", &["-P"], "k:\n  k:\n"),
    ("a: {k\n\n}\nb: 1\n", &[], "a: {k: ''}\nb: 1\n"),
    ("a: {k\n\n}\nb: 1\n", &["-P"], "a:\n  k:\nb: 1\n"),
];

#[test]
fn a_flow_key_without_a_value_ends_before_its_trailing_break_and_comment_6941() -> Result<()> {
    for (doc, want) in ACCEPTED {
        let (out, code) = run_yq_stdin(".", doc, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{doc:?}");
        assert_eq!(out, *want, "{doc:?}");
    }
    Ok(())
}

#[test]
fn a_flow_key_that_reaches_a_colon_on_a_later_line_is_still_refused_6941() -> Result<()> {
    for doc in REFUSED {
        let (_, code) = run_yq_stdin(".", doc, &["-o=json", "-I=0"])?;
        assert_ne!(code, 0, "{doc:?}");
    }
    Ok(())
}

#[test]
fn a_flow_key_without_a_value_prints_back_as_yq_does_6941() -> Result<()> {
    for (doc, extra, want) in YAML_ROWS {
        let (out, code) = run_yq_stdin(".", doc, extra)?;
        assert_eq!(code, 0, "{doc:?} {extra:?}");
        assert_eq!(out, *want, "{doc:?} {extra:?}");
    }
    Ok(())
}
