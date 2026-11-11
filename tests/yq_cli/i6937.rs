//! #6937: a tab directly after the closing quote of a quoted scalar, or after an alias, in block context is
//! separation before the line break or a comment, as it is after a plain scalar or a flow collection. The
//! scanners for those stop just past their closing delimiter, so the main loop was entered mid-line and read
//! the tab at the cursor as indentation. A tab that starts a line is still refused, and so is content after it.
//!
//! Every row is captured from yq v4.53.3 (`-o=json -I=0 .`).

use anyhow::Result;

use crate::run_yq_stdin;

/// Accepted by yq: `(document, expected output)`.
const ACCEPTED: &[(&str, &str)] = &[
    ("\"a\"\t\n", "\"a\"\n"),
    ("'a'\t\n", "\"a\"\n"),
    ("a: \"b\"\t\n", "{\"a\":\"b\"}\n"),
    ("a: \"b\"\t# c\n", "{\"a\":\"b\"}\n"),
    ("a: \"b\"\t \n", "{\"a\":\"b\"}\n"),
    ("a: \"b\" \t\n", "{\"a\":\"b\"}\n"),
    ("- \"x\"\t\n- y\n", "[\"x\",\"y\"]\n"),
    ("? \"a\"\t\n: 1\n", "{\"a\":1}\n"),
    ("a: &x \"b\"\t\n", "{\"a\":\"b\"}\n"),
    ("x: &x 1\na: *x\t\n", "{\"x\":1,\"a\":1}\n"),
    ("a: !!str \"b\"\t\n", "{\"a\":\"b\"}\n"),
    ("a: \"b\n  c\"\t\n", "{\"a\":\"b c\"}\n"),
    ("\"a\"\t", "\"a\"\n"),
    ("\"a\"\t\t\n", "\"a\"\n"),
    ("a: \"b\"\t\nc: 1\n", "{\"a\":\"b\",\"c\":1}\n"),
    ("a:\n  - \"b\"\t\n  - c\n", "{\"a\":[\"b\",\"c\"]}\n"),
    ("a: b\t\n", "{\"a\":\"b\"}\n"),
    ("[1]\t\n", "[1]\n"),
    ("a: {x: 1}\t\n", "{\"a\":{\"x\":1}}\n"),
];

/// Refused by yq (a tab that starts a line, or content after the tab).
const REFUSED: &[&str] = &[
    "\"a\"\n\t\n",
    "a: \"b\"\n\t\n",
    "\t\"a\"\n",
    "a:\n\t\"b\"\n",
    "a: \"b\"\tc\n",
    "\"a\"\tb\n",
    "- \"a\"\t- b\n",
    "\"a\"\n \t\n",
];

#[test]
fn a_tab_after_a_closing_quote_or_an_alias_is_separation_6937() -> Result<()> {
    for (doc, want) in ACCEPTED {
        let (out, code) = run_yq_stdin(".", doc, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{doc:?}");
        assert_eq!(out, *want, "{doc:?}");
    }
    Ok(())
}

#[test]
fn a_tab_that_starts_a_line_or_precedes_content_is_still_refused_6937() -> Result<()> {
    for doc in REFUSED {
        let (_, code) = run_yq_stdin(".", doc, &["-o=json", "-I=0"])?;
        assert_ne!(code, 0, "{doc:?}");
    }
    Ok(())
}
