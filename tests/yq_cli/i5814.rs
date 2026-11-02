//! #5814: a tab between a quoted or alias key and its `:` is `s-separate-in-line`, as a space is, in
//! a block mapping and in a sequence item's compact mapping. The lookahead that decides a line is a
//! mapping entry only let spaces through, so `- "k"\t: 1` was a parse error and `"k"\t: 1` a
//! misleading "tab character used for indentation". Every expected string is a live yq v4.53.3
//! capture.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, expected `-o=json -I0 .`, expected `.`).
const ROWS: &[(&str, &str, &str)] = &[
    (
        "c:\n- \"k\"\t: 1\n",
        "{\"c\":[{\"k\":1}]}\n",
        "c:\n  - \"k\": 1\n",
    ),
    (
        "c:\n  \"k\"\t: 1\n",
        "{\"c\":{\"k\":1}}\n",
        "c:\n  \"k\": 1\n",
    ),
    ("\"k\"\t: 1\n", "{\"k\":1}\n", "\"k\": 1\n"),
    ("'k'\t: 1\n", "{\"k\":1}\n", "'k': 1\n"),
    ("\"k\"\t:\t1\n", "{\"k\":1}\n", "\"k\": 1\n"),
    ("- \"k\"\t: 1\n", "[{\"k\":1}]\n", "- \"k\": 1\n"),
    (
        "x:\n  - \"a\"\t: 1\n    \"b\"\t: 2\n",
        "{\"x\":[{\"a\":1,\"b\":2}]}\n",
        "x:\n  - \"a\": 1\n    \"b\": 2\n",
    ),
    (
        "a: &a x\nc:\n- *a\t: 1\n",
        "{\"a\":\"x\",\"c\":[{\"x\":1}]}\n",
        "a: &a x\nc:\n  - *a : 1\n",
    ),
    (
        "a: &a x\n*a\t: 1\n",
        "{\"a\":\"x\",\"x\":1}\n",
        "a: &a x\n*a : 1\n",
    ),
    ("\"k\" \t : 1\n", "{\"k\":1}\n", "\"k\": 1\n"),
    ("\"k\"\t: [1,\t2]\n", "{\"k\":[1,2]}\n", "\"k\": [1, 2]\n"),
    ("- \"k\"\t:\n    v\n", "[{\"k\":\"v\"}]\n", "- \"k\": v\n"),
    ("- 'k'\t: 1 # c\n", "[{\"k\":1}]\n", "- 'k': 1 # c\n"),
];

#[test]
fn test_a_tab_before_the_colon_of_a_quoted_or_alias_key_reads_as_yq_does_5814() -> Result<()> {
    for (doc, json, yaml) in ROWS {
        let (stdout, code) = run_yq_stdin(".", doc, &["-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (*json, 0), "{doc:?}");
        let (stdout, code) = run_yq_stdin(".", doc, &[])?;
        assert_eq!((stdout.as_str(), code), (*yaml, 0), "{doc:?}");
    }
    Ok(())
}

/// A tab that really is indentation is still an error, before the quoted key and the colon alike.
#[test]
fn test_a_leading_tab_before_a_quoted_key_is_still_tab_indentation_5814() -> Result<()> {
    for doc in ["\t\"k\": 1\n", "c:\n\t\"k\": 1\n", "c:\n\t- 1\n"] {
        let (stdout, code) = run_yq_stdin(".", doc, &[])?;
        assert_ne!(code, 0, "{doc:?} gave {stdout:?}");
    }
    Ok(())
}
