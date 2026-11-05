//! #6293: a flow indicator (`,` `[` `]` `{` `}`) right after a tag, with no space, is part of the
//! tag's run of characters for yq's scanner, which refuses it: `[!!str]`, `{a: !!str}`,
//! `[!!str, 1]` and `a: !!str]` are `found character ']' that is not allowed in a YAML tag`.
//! `succinctly yq` stopped the tag at the indicator and read an empty value. A space first
//! (`[!!str ]`) parses in both. Every expectation is what yq v4.53.3 printed.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// Documents yq refuses, in flow and block context, with and without a value, for a core tag, a
/// custom one, the bare `!`, a named handle and a verbatim tag.
const REFUSED: &[&str] = &[
    "[!!str]\n",
    "{a: !!str}\n",
    "[!!str, 1]\n",
    "[a, !!str]\n",
    "[a, !!str, b]\n",
    "[!foo]\n",
    "[!foo, 1]\n",
    "[!]\n",
    "[!, 1]\n",
    "[!h!x]\n",
    "[!<tag:x>]\n",
    "[!<tag:x>, 1]\n",
    "[!a,b]\n",
    "[!!str{}]\n",
    "a: [!!int]\n",
    "a: [!!null]\n",
    "{!!str}\n",
    "{? !!str}\n",
    "a: !!str]\n",
    "a: !!str,x\n",
    "a: !!str}\n",
    "- !!str,\n",
    "- !!str]\n",
    "!!str]\n",
    "!!str,\n",
    "!!str}\n",
];

#[test]
fn test_a_flow_indicator_right_after_a_tag_is_refused_6293() -> Result<()> {
    for document in REFUSED {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", document, &["-o=json", "-I=0"])?;
        assert_ne!(code, 0, "{document:?}: {stdout}");
        assert!(stdout.is_empty(), "{document:?}: {stdout}");
        assert!(!stderr.is_empty(), "{document:?}");
    }
    Ok(())
}

/// A space before the indicator, or a value after the tag, is as before.
const ACCEPTED: &[(&str, &str)] = &[
    ("[!!str ]\n", "[\"\"]\n"),
    ("[!!str ,1]\n", "[\"\",1]\n"),
    ("[!!str a]\n", "[\"a\"]\n"),
    ("[!!str a, !!int 1]\n", "[\"a\",1]\n"),
    ("{!!str : 1}\n", "{\"\":1}\n"),
    ("{!!str a: !!int 1}\n", "{\"a\":1}\n"),
    ("[!!seq [a]]\n", "[[\"a\"]]\n"),
    ("[!!map {a: 1}]\n", "[{\"a\":1}]\n"),
    ("a: !!seq [a]\n", "{\"a\":[\"a\"]}\n"),
    ("a: !!map {b: 1}\n", "{\"a\":{\"b\":1}}\n"),
    ("- !!str a\n", "[\"a\"]\n"),
    ("[ !!str \"x\" ]\n", "[\"x\"]\n"),
    ("{a: !!str \"x\"}\n", "{\"a\":\"x\"}\n"),
];

#[test]
fn test_a_tag_followed_by_a_space_or_a_value_is_unchanged_6293() -> Result<()> {
    for &(document, expected) in ACCEPTED {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", document, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{document:?}: {stderr}");
        assert_eq!(stdout, expected, "{document:?}");
    }
    Ok(())
}
