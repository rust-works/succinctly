//! #5452: three yq-lexer rejections `succinctly yq` used to accept. A string literal inside a
//! `\(...)` slot and a number with a trailing `.` are jq syntax, so they need `--jq-extensions`;
//! whitespace between a `.` and a name, a quote or a bracket is refused outright (jq rejects it
//! too, #5447). Every row was checked against yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const DOC: &str = "a: [1]\n";

#[test]
fn test_jq_syntax_the_yq_lexer_rejects_needs_jq_extensions_5452() -> Result<()> {
    let plain = &["-o=json", "-I=0"];
    let extended = &["--jq-extensions", "-o=json", "-I=0"];
    for (filter, expected) in [
        (r#""\("x")""#, "\"x\"\n"),
        (r#""\(.a // "d")""#, "\"[1]\"\n"),
        (r#""\(.a)\("y")""#, "\"[1]y\"\n"),
        // A quoted path step is a string to yq's lexer as well.
        (r#""\(."a")""#, "\"[1]\"\n"),
        // The depth is restored after a slot: a string after one is fine, and nested slots count.
        (r#""\(.a)" + "\(.a)\("z")""#, "\"[1][1]z\"\n"),
        ("5.", "5.0\n"),
        ("1. + 1", "2.0\n"),
        ("[1.]", "[1.0]\n"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, DOC, plain)?;
        assert_eq!(stdout, "", "`{filter}` must print nothing without the flag");
        assert_ne!(code, 0, "`{filter}` must be rejected without the flag");
        assert!(stderr.contains("--jq-extensions"), "`{filter}`: {stderr:?}");
        assert_eq!(
            run_yq_stdin_with_stderr(filter, DOC, extended)?,
            (expected.into(), String::new(), 0),
            "--jq-extensions `{filter}`"
        );
    }
    Ok(())
}

#[test]
fn test_whitespace_after_a_dot_ends_the_identity_in_yq_5452() -> Result<()> {
    for args in [
        &["-o=json", "-I=0"][..],
        &["--jq-extensions", "-o=json", "-I=0"],
    ] {
        for filter in [
            ". a",
            ".  a",
            ". length",
            ". tag",
            ". \"a\"",
            ". [\"a\"]",
            ".a . b",
            ".a | . b",
            ".\na",
        ] {
            let (stdout, _stderr, code) = run_yq_stdin_with_stderr(filter, DOC, args)?;
            assert_eq!(stdout, "", "`{filter}` {args:?}");
            assert_ne!(code, 0, "`{filter}` must be rejected {args:?}");
        }
    }
    Ok(())
}

/// What yq accepts keeps working: the root metadata write, the terminators after a gap, a gap
/// before a bracket on a path, adjacent interpolation slots and a fractional number.
#[test]
fn test_yq_forms_around_the_dot_and_interpolation_still_parse_5452() -> Result<()> {
    let args = &["-o=json", "-I=0"];
    for (filter, expected) in [
        (". style = \"flow\" | .a", "[1]\n"),
        (". as $x | $x", "{\"a\":[1]}\n"),
        (". and true", "true\n"),
        (". == 1", "false\n"),
        (".a [0]", "1\n"),
        (r#""\(.a)\(.a)""#, "\"[1][1]\"\n"),
        ("1.5", "1.5\n"),
        (". | tag", "\"!!map\"\n"),
    ] {
        assert_eq!(
            run_yq_stdin_with_stderr(filter, DOC, args)?,
            (expected.into(), String::new(), 0),
            "`{filter}`"
        );
    }
    Ok(())
}
