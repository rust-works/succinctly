//! #5837: yq's `@sh` leaves a run of characters that need no quotes bare and single-quotes from
//! the first one that does to the end of the string or the next `'`, which it writes `\\'`
//! outside the quotes. succinctly quoted the whole string, as jq does. Every row is
//! `yq '.a | @sh'` from yq v4.53.3.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

/// (document, stdout)
const ROWS: &[(&str, &str)] = &[
    ("a: \"hello\"\n", "hello\n"),
    ("a: \"a b\"\n", "a' b'\n"),
    ("a: \"it's\"\n", "it\\'s\n"),
    ("a: \"\"\n", "\n"),
    ("a: \"x$y\"\n", "x'$y'\n"),
    ("a: \"*\"\n", "'*'\n"),
    ("a: \"-n\"\n", "-n\n"),
    ("a: \"1x\"\n", "1x\n"),
    ("a: \"a_b-c.d/e:f@g%h+i=j,k\"\n", "a_b-c.d/e:f@g%h+i=j,k\n"),
    ("a: \"a~b\"\n", "a'~b'\n"),
    ("a: \"a#b\"\n", "a'#b'\n"),
    ("a: \"!x\"\n", "'!x'\n"),
    ("a: \"a\\\"b\"\n", "a'\"b'\n"),
    ("a: \"a\\\\b\"\n", "a'\\b'\n"),
    ("a: \"tab\\tx\"\n", "tab'\tx'\n"),
    ("a: \"é\"\n", "'é'\n"),
    ("a: \"a;b\"\n", "a';b'\n"),
    ("a: \"(x)\"\n", "'(x)'\n"),
    ("a: \"a|b\"\n", "a'|b'\n"),
    ("a: \"a`b\"\n", "a'`b'\n"),
    ("a: \"a''b\"\n", "a\\'\\'b\n"),
    ("a: \"'\"\n", "\\'\n"),
    ("a: \"''\"\n", "\\'\\'\n"),
    ("a: \"x'\"\n", "x\\'\n"),
    ("a: \"'x\"\n", "\\'x\n"),
    ("a: \"a  b\"\n", "a'  b'\n"),
    ("a: \"^x\"\n", "'^x'\n"),
    ("a: \",x\"\n", ",x\n"),
    ("a: \"a b'c d\"\n", "a' b'\\'c' d'\n"),
    ("a: \"it's a 'x' y\"\n", "it\\'s' a '\\'x\\'' y'\n"),
    ("a: \"αβ\"\n", "'αβ'\n"),
    ("a: \"a\\nb\"\n", "a'\nb'\n"),
];

#[test]
fn test_at_sh_quotes_as_yq_does_5837() -> Result<()> {
    for (doc, want) in ROWS {
        let (stdout, code) = run_yq_stdin(".a | @sh", doc, &[])?;
        assert_eq!((stdout.as_str(), code), (*want, 0), "{doc:?}");
    }
    Ok(())
}

/// The same on a computed string and a flow string, and through `key`.
#[test]
fn test_at_sh_on_other_routes_5837() -> Result<()> {
    for (filter, doc, want) in [
        ("(.a + \" x\") | @sh", "a: hello\n", "hello' x'\n"),
        (".a[] | @sh", "a: [x y, z]\n", "x' y'\nz\n"),
        ("keys[] | @sh", "a b: 1\nc: 2\n", "a' b'\nc\n"),
        ("\"it's\" | @sh", "null\n", "it\\'s\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}

/// A non-string is refused in the `@uri` wording yq uses, under `-n` as for a document node, and a
/// string that does not decode reports that first.
#[test]
fn test_at_sh_refusals_5837() -> Result<()> {
    for (filter, tag) in [
        ("5", "!!int"),
        ("true", "!!bool"),
        ("null", "!!null"),
        ("[1]", "!!seq"),
    ] {
        let (stdout, stderr, code) =
            run_yq_stdin_with_stderr(&format!("{filter} | @sh"), "", &["-n"])?;
        assert_eq!(
            (stdout.as_str(), stderr.trim(), code),
            (
                "",
                format!(
                    "Error: cannot encode {tag} as URI, can only operate on strings. Please first pipe through another encoding operator to convert the value to a string"
                )
                .as_str(),
                1
            ),
            "{filter}"
        );
    }
    let (_, stderr, code) = run_yq_stdin_with_stderr(".a | @sh", "a: \"\\ud83d\"\n", &[])?;
    assert_eq!((stderr.trim(), code), ("Error: invalid escape sequence", 1));
    Ok(())
}
