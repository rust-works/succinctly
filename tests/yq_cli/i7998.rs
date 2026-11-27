//! #7998: yq's lexer tries its number tokens (`-?\d+`, `-?\d+(\.\d+)`,
//! `-?[1-9](\.\d+)?[Ee][-+]?\d+`, `lexer_participle.go` v4.53.3) before `Subtract`, so a `-`
//! directly followed by a digit is the sign of a negative number wherever it stands. After a value
//! that leaves two values side by side, which yq's tree builder refuses with `bad expression,
//! please check expression syntax` (exit 1). A `-` before a space, a newline, a comment, `(`, `.`
//! or another `-` is still a subtraction. Every row is a live yq v4.53.3 capture over `a: 5`.
//! jq has no such token: `succinctly jq` still reads `1 -16` as a subtraction.

use anyhow::Result;

use crate::{run_jq_stdin_with_stderr, run_yq_stdin, run_yq_stdin_with_stderr};

const DOC: &str = "a: 5\n";

#[test]
fn test_value_then_negative_number_is_bad_expression_7998() -> Result<()> {
    for filter in [
        "1 -16",
        "1-16",
        "0 -1",
        "1\n-16",
        ".a -1",
        ".a -1.5",
        "1 -1.0",
        "1 -1e3",
        "1 -1E+3",
        "1 -0",
        "1 -01",
        "[.a -1]",
        "{\"b\": .a -1}",
        "(1) -2",
        "\"x\" -1",
        "true -1",
        "null -1",
        "[1] -1",
        "{} -1",
        ".a[0] -1",
        ". as $x | $x -1",
        "1 -1 as $x | $x",
        ".a | . -1",
        ".a |= . -1",
        ".a = 1 -1",
        "1 -1 + 2",
        "1 -1 -1",
        "1 -1 == 0",
        "1 -1 | 2",
        ".a -1, 2",
        ".a -1 # c",
        "select(. -1)",
        "with(.a; . -1)",
        "\"\\(1 -1)\"",
        "eval(\"1 -1\")",
    ] {
        let (out, err, code) = run_yq_stdin_with_stderr(filter, DOC, &["-o=json", "-I0", "--"])?;
        assert_eq!((out.as_str(), code), ("", 1), "{filter:?}: {err}");
        assert!(
            err.contains("bad expression, please check expression syntax"),
            "{filter:?}: {err}"
        );
    }
    Ok(())
}

/// yq refuses these in its lexer instead (`-0` is a number, then `x1` or `e3` is no token: `lexer:
/// invalid input text "x1"`), so only the exit code is pinned.
#[test]
fn test_value_then_negative_number_lexer_refusals_7998() -> Result<()> {
    for filter in [".a -0x1", "1 -0x1", "1 -0e3"] {
        let (out, err, code) = run_yq_stdin_with_stderr(filter, DOC, &["-o=json", "-I0", "--"])?;
        assert_eq!((out.as_str(), code), ("", 1), "{filter:?}: {err}");
    }
    Ok(())
}

/// A `-` that is not directly followed by a digit is still `Subtract`, and a negative number where
/// a value may start is still one.
#[test]
fn test_subtraction_and_negative_operands_unchanged_7998() -> Result<()> {
    for (filter, want) in [
        ("1 - 16", "-15"),
        ("1 -\n16", "-15"),
        ("1 -\t1", "0"),
        ("1 -#c\n16", "-15"),
        ("1 -.a", "-4"),
        (".a -(1)", "4"),
        (".a - -1", "6"),
        ("1 --1", "2"),
        ("1 + -1", "0"),
        ("1 * -1", "-1"),
        (".a *-1", "-5"),
        (".a // -1", "5"),
        (".a //-1", "5"),
        (".a == -1", "false"),
        (".a==-1", "false"),
        (".a and -1", "true"),
        (".a|-1", "-1"),
        (".a,-1", "5\n-1"),
        ("[.a, -1]", "[5,-1]"),
        ("[1,-1]", "[1,-1]"),
        ("{\"a\": -1}", "{\"a\":-1}"),
        (".a -= 1", "{\"a\":4}"),
        (".a -=1", "{\"a\":4}"),
        (".a -=-1", "{\"a\":6}"),
        (".a -= -1", "{\"a\":6}"),
        (".a-1", "null"),
        (".a\t-1", "null"),
        (". as $x | $x-1", ""),
        ("-1", "-1"),
        ("-01 | tostring", "\"-01\""),
        ("-1.50 | tostring", "\"-1.50\""),
        ("-1e3 | tostring", "\"-1e3\""),
    ] {
        let (out, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0", "--"])?;
        assert_eq!((out.trim_end(), code), (want, 0), "{filter:?}");
    }
    Ok(())
}

/// jq lexes `-` as its own token, so `1 -16` is a subtraction there (jq 1.7.1: `-15`).
#[test]
fn test_jq_mode_still_subtracts_a_minus_digit_7998() -> Result<()> {
    for (filter, want) in [("1 -16", "-15"), ("1-16", "-15"), (".a -1", "4")] {
        let (out, err, code) = run_jq_stdin_with_stderr(filter, "{\"a\":5}", &["-c"])?;
        assert_eq!((out.trim_end(), code), (want, 0), "{filter:?}: {err}");
    }
    Ok(())
}
