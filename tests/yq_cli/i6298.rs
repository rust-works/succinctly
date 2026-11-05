//! #6298: a verbatim tag (`!<...>`) must be followed by whitespace, a line break or the end of the
//! input in yq's scanner, which otherwise stops with `did not find expected whitespace or line
//! break`. `succinctly yq` read the text after the closing `>` as the tagged value. Every
//! expectation is what yq v4.53.3 printed.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const REFUSED: &[&str] = &[
    "a: !<tag:x>x\n",
    "[!<tag:x>x]\n",
    "!<tag:x>x\n",
    "a: !<tag:x>\"q\"\n",
    "a: !<tag:x>#c\n",
    "!<tag:x>[a]\n",
    "a: !<tag:x>{b: 1}\n",
    "[!<tag:x>:1]\n",
    "!<tag:x>: 1\n",
];

#[test]
fn test_a_verbatim_tag_followed_by_a_non_space_is_refused_6298() -> Result<()> {
    for document in REFUSED {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", document, &["-o=json", "-I=0"])?;
        assert_ne!(code, 0, "{document:?}: {stdout}");
        assert!(stdout.is_empty(), "{document:?}: {stdout}");
        assert!(!stderr.is_empty(), "{document:?}");
    }
    Ok(())
}

/// A space, a tab, a line break, the end of the input or a comment after a space: as before.
const ACCEPTED: &[(&str, &str)] = &[
    ("a: !<tag:x> v\n", "{\"a\":\"v\"}\n"),
    ("a: !<tag:x>\tv\n", "{\"a\":\"v\"}\n"),
    ("[!<tag:x> a]\n", "[\"a\"]\n"),
    ("a: !<tag:x>\n  b\n", "{\"a\":\"b\"}\n"),
    ("a: !<tag:x>   # c\n", "{\"a\":\"\"}\n"),
    ("a: !<tag:x>\n", "{\"a\":\"\"}\n"),
    ("!<tag:x>\n", "\"\"\n"),
    ("!<!bar> a\n", "\"a\"\n"),
    ("? !<tag:x> a\n", "{\"a\":null}\n"),
];

#[test]
fn test_a_verbatim_tag_followed_by_whitespace_is_unchanged_6298() -> Result<()> {
    for &(document, expected) in ACCEPTED {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", document, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{document:?}: {stderr}");
        assert_eq!(stdout, expected, "{document:?}");
    }
    Ok(())
}
