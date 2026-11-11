//! #6949: a flow mapping entry that is a plain scalar with no `:` may continue on the next line
//! (`{a` / ` b}` is the key `"a b"` with a null), and a line of it may end in blanks before the break.
//! The implicit-key scanner ended the key at the blanks and refused the continuation line; the
//! explicit-key scanner had handed the decision to its line-break arm since #402, and the implicit one
//! does the same now. A key that reaches a `:` on a later line, a comment between the lines, and a
//! quoted key with a continuation are still refused, as in yq.
//!
//! Every row is captured from yq v4.53.3 (`-o=json -I=0 .`, and the YAML output of three of them).

use anyhow::Result;

use crate::run_yq_stdin;

/// Accepted by yq: `(document, expected JSON output)`.
const ACCEPTED: &[(&str, &str)] = &[
    ("{a\n b}\n", "{\"a b\":null}\n"),
    ("{a \n b}\n", "{\"a b\":null}\n"),
    ("{a\t\n b}\n", "{\"a b\":null}\n"),
    (
        "{ multi\t\n  line, a: b}\n",
        "{\"multi line\":null,\"a\":\"b\"}\n",
    ),
    ("{ a \n b \n c}\n", "{\"a b c\":null}\n"),
    (
        "{ k: v,\n omitted \n value}\n",
        "{\"k\":\"v\",\"omitted value\":null}\n",
    ),
    ("{a b \n c}\n", "{\"a b c\":null}\n"),
    ("x: {a \n b}\n", "{\"x\":{\"a b\":null}}\n"),
    ("- {a \n b}\n", "[{\"a b\":null}]\n"),
    ("{ multi \r\n  line}\r\n", "{\"multi line\":null}\n"),
    ("{a: b,\n c \n }\n", "{\"a\":\"b\",\"c\":null}\n"),
    ("[ a \n b ]\n", "[\"a b\"]\n"),
    ("{a \n}\n", "{\"a\":null}\n"),
    ("{a \n , b}\n", "{\"a\":null,\"b\":null}\n"),
    ("{a  \n\n b}\n", "{\"a\\nb\":null}\n"),
    ("{a \n }\n", "{\"a\":null}\n"),
    ("{ k \n \n}\n", "{\"k\":null}\n"),
    ("{a  \n  b  \n}\n", "{\"a b\":null}\n"),
    ("[k: v \n]\n", "[{\"k\":\"v\"}]\n"),
    ("[a b \n c]\n", "[\"a b c\"]\n"),
    ("{a \t\n b \t\n c \t\n}\n", "{\"a b c\":null}\n"),
    ("{a \n b, c \n d}\n", "{\"a b\":null,\"c d\":null}\n"),
];

/// Refused by yq.
const REFUSED: &[&str] = &[
    "{\"x\" \n y}\n",
    "{a \n b: 1}\n",
    "{a \n # c\n b}\n",
    "[a \n b: 1]\n",
    "[a \n : 1]\n",
    "{a \n : 1}\n",
];

/// YAML output: `(document, extra arguments, expected)`.
const YAML_ROWS: &[(&str, &[&str], &str)] = &[
    ("{a \n b}\n", &[], "{a b: ''}\n"),
    ("{a \n b}\n", &["-P"], "a b:\n"),
    (
        "{ multi\t\n  line, a: b}\n",
        &[],
        "{multi line: '', a: b}\n",
    ),
    ("{ multi\t\n  line, a: b}\n", &["-P"], "multi line:\na: b\n"),
    ("- {a \n b}\n", &[], "- {a b: ''}\n"),
    ("- {a \n b}\n", &["-P"], "- a b:\n"),
];

#[test]
fn a_multi_line_flow_key_with_a_trailing_blank_is_one_key_6949() -> Result<()> {
    for (doc, want) in ACCEPTED {
        let (out, code) = run_yq_stdin(".", doc, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{doc:?}");
        assert_eq!(out, *want, "{doc:?}");
    }
    Ok(())
}

#[test]
fn a_flow_key_continuation_that_yq_refuses_is_still_refused_6949() -> Result<()> {
    for doc in REFUSED {
        let (_, code) = run_yq_stdin(".", doc, &["-o=json", "-I=0"])?;
        assert_ne!(code, 0, "{doc:?}");
    }
    Ok(())
}

#[test]
fn a_multi_line_flow_key_prints_back_as_yq_does_6949() -> Result<()> {
    for (doc, extra, want) in YAML_ROWS {
        let (out, code) = run_yq_stdin(".", doc, extra)?;
        assert_eq!(code, 0, "{doc:?} {extra:?}");
        assert_eq!(out, *want, "{doc:?} {extra:?}");
    }
    Ok(())
}
