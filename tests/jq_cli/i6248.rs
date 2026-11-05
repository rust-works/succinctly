//! #6248: Oniguruma's `\o{ddd}`, an octal code point in braces, in a class and out of one. The `regex`
//! crate has no `\o`, so the pattern was refused where jq answers. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -n -r`) over `"aAB\u0001\u00e9\u0100\u00ff"`.

use anyhow::Result;

use crate::run_jq_full;

const SUBJECT: &str = r#""aAB\u0001\u00e9\u0100\u00ff""#;

fn matches(pattern: &str) -> Result<(String, String, i32)> {
    let program = format!("{SUBJECT} | [match($p; \"g\") | .string] | tojson");
    let (stdout, stderr, status) =
        run_jq_full(&["-n", "-r", "--arg", "p", pattern, &program], None)?;
    Ok((stdout.trim_end().to_string(), stderr, status))
}

#[test]
fn an_octal_brace_escape_is_a_code_point_6248() -> Result<()> {
    for (pattern, expected) in [
        (r"\o{101}", "[\"A\"]"),
        (r"\o{0101}", "[\"A\"]"),
        (r"\o{1}", "[\"\\u0001\"]"),
        (r"\o{377}", "[\"ÿ\"]"),
        (r"\o{351}", "[\"é\"]"),
        (r"\o{400}", "[\"Ā\"]"),
        (r"\o{00000000101}", "[\"A\"]"),
        (r"[\o{101}]", "[\"A\"]"),
        (r"[^\o{101}]", "[\"a\",\"B\",\"\\u0001\",\"é\",\"Ā\",\"ÿ\"]"),
        (r"[\o{101}-\o{102}]", "[\"A\",\"B\"]"),
        (r"[\o{101}\o{1}]", "[\"A\",\"\\u0001\"]"),
        (r"[a-\o{142}]", "[\"a\"]"),
        (r"\o{101}+", "[\"A\"]"),
        (r"(?:\o{101}|\o{102})+", "[\"AB\"]"),
        (r"(?i)\o{101}", "[\"a\",\"A\"]"),
        (r"\o{141}", "[\"a\"]"),
        (r"\o{4177777}", "[]"),
        (r"\o{4200000}", "[]"),
        (r"[\o{4200000}]", "[]"),
        (
            r"[^\o{4200000}]",
            "[\"a\",\"A\",\"B\",\"\\u0001\",\"é\",\"Ā\",\"ÿ\"]",
        ),
        (r"\o{154000}", "[]"),
        (r"[\o{7777777}]", "[]"),
        // The engine's limit outside a class is U+13FFFF, one below the first refusal.
        (r"\o{4777777}", "[]"),
        (r"[\o{5000000}]", "[]"),
        (r"\o{ 1}", "[]"),
        (r"\o{x}", "[]"),
        (r"o", "[]"),
    ] {
        let (stdout, _, status) = matches(pattern)?;
        assert_eq!((stdout.as_str(), status), (expected, 0), "{pattern}");
    }
    Ok(())
}

/// The refusals, with the messages Oniguruma gives: a value past 32 bits, a decimal digit after the
/// digits, no closing brace, and a code point the engine cannot hold (U+140000 outside a class,
/// U+200000 inside one).
#[test]
fn a_malformed_octal_brace_escape_is_refused_as_jq_does_6248() -> Result<()> {
    for (pattern, message) in [
        (r"\o{5000000}", "invalid code point value"),
        (r"\o{7777777}", "invalid code point value"),
        (r"\o{10000000}", "invalid code point value"),
        (r"\o{17777777777}", "invalid code point value"),
        (r"\o{8}", "too long wide-char value"),
        (r"\o{18}", "too long wide-char value"),
        (r"\o{000000000101}", "too long wide-char value"),
        (r"\o{40000000000}", "too big number"),
        (r"[\o{40000000000}]", "too big number"),
        (r"\o{101", "invalid code point value"),
        (r"\o{1x}", "invalid code point value"),
        (r"[\o{7777777777}]", "invalid code point value"),
        (r"[\o{10000000}]", "invalid code point value"),
    ] {
        let (stdout, stderr, status) = matches(pattern)?;
        assert_eq!((stdout.as_str(), status), ("", 5), "{pattern}");
        assert!(stderr.contains(message), "{pattern}: {stderr}");
    }
    Ok(())
}

/// A surrogate or a value past U+10FFFF as an end of a range in a class: jq clamps the range
/// (`[a-\o{154000}]` runs to U+D7FF), which is not reproduced. It stays refused, not read as an empty
/// set that would answer `[]` where jq answers.
#[test]
fn a_range_ending_in_a_non_character_octal_escape_stays_refused_6248() -> Result<()> {
    for pattern in [
        r"[a-\o{154000}]",
        r"[\o{154000}-z]",
        r"[\o{101}-\o{154000}]",
        r"[a-\o{4200000}]",
    ] {
        let (stdout, _, status) = matches(pattern)?;
        assert_eq!((stdout.as_str(), status), ("", 5), "{pattern}");
    }
    Ok(())
}
