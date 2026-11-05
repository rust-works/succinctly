//! #6251: Oniguruma's braced hex escape `\x{hhh}`, in a class and out of one, read as it reads it: at most
//! eight digits, a surrogate or a value past U+10FFFF that matches nothing, the letter `x` when no digit
//! follows the brace, and the refusals #6248 pinned for `\o{..}`. Every row was captured from jq 1.7.1
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
fn a_hex_brace_escape_is_a_code_point_6251() -> Result<()> {
    for (pattern, expected) in [
        (r"\x{41}", "[\"A\"]"),
        (r"\x{0041}", "[\"A\"]"),
        (r"\x{00000041}", "[\"A\"]"),
        (r"\x{1}", "[\"\\u0001\"]"),
        (r"\x{e9}", "[\"é\"]"),
        (r"\x{FF}", "[\"ÿ\"]"),
        (r"\x{100}", "[\"Ā\"]"),
        (r"[\x{41}]", "[\"A\"]"),
        (r"[^\x{41}]", "[\"a\",\"B\",\"\\u0001\",\"é\",\"Ā\",\"ÿ\"]"),
        (r"[\x{41}-\x{42}]", "[\"A\",\"B\"]"),
        (r"\x{41}+", "[\"A\"]"),
        (r"(?i)\x{41}", "[\"a\",\"A\"]"),
        (r"\x{41}\o{102}", "[\"AB\"]"),
        (r"\x{13FFFF}", "[]"),
        (r"\x{10FFFF}", "[]"),
        (r"\x{110000}", "[]"),
        (r"\x{13FFFF}", "[]"),
        (r"\x{D800}", "[]"),
        (r"\x{DFFF}", "[]"),
        (r"[\x{110000}]", "[]"),
        (r"[\x{1FFFFF}]", "[]"),
        (r"[\x{D800}]", "[]"),
        (
            r"[^\x{D800}]",
            "[\"a\",\"A\",\"B\",\"\\u0001\",\"é\",\"Ā\",\"ÿ\"]",
        ),
        (r"[\x{D800}\x{41}]", "[\"A\"]"),
        (r"\x{}", "[]"),
        (r"\x{ 1}", "[]"),
        (r"\x{G}", "[]"),
        (r"[\x{}]", "[]"),
        (r"[\x{ 1}]", "[]"),
        (r"x", "[]"),
    ] {
        let (stdout, _, status) = matches(pattern)?;
        assert_eq!((stdout.as_str(), status), (expected, 0), "{pattern}");
    }
    Ok(())
}

#[test]
fn a_malformed_hex_brace_escape_is_refused_as_jq_does_6251() -> Result<()> {
    for (pattern, message) in [
        (r"[a-\x{000000041}]", "too long wide-char value"),
        (r"[a-\x{1G}]", "invalid code point value"),
        (r"\x{140000}", "invalid code point value"),
        (r"\x{1FFFFF}", "invalid code point value"),
        (r"\x{FFFFFFFF}", "invalid code point value"),
        (r"[\x{200000}]", "invalid code point value"),
        (r"[\x{FFFFFFFF}]", "invalid code point value"),
        (r"\x{100000000}", "too long wide-char value"),
        (r"\x{000000041}", "too long wide-char value"),
        (r"[\x{000000041}]", "too long wide-char value"),
        (r"\x{00000000A}", "too long wide-char value"),
        (r"\x{FFFFFFFFF}", "too long wide-char value"),
        (r"\x{101", "invalid code point value"),
        (r"\x{1G}", "invalid code point value"),
        (r"\x{1 }", "invalid code point value"),
        (r"[\x{4]", "invalid code point value"),
        (r"\x{4", "invalid code point value"),
    ] {
        let (stdout, stderr, status) = matches(pattern)?;
        assert_eq!((stdout.as_str(), status), ("", 5), "{pattern}");
        assert!(stderr.contains(message), "{pattern}: {stderr}");
    }
    Ok(())
}

/// A surrogate or a value past U+10FFFF as an end of a range in a class: jq clamps the range, which is
/// not reproduced. It stays refused, not read as an empty set that would answer `[]` where jq answers.
#[test]
fn a_range_ending_in_a_non_character_hex_escape_stays_refused_6251() -> Result<()> {
    for pattern in [
        r"[a-\x{D800}]",
        r"[\x{D800}-z]",
        r"[\x{D800}-\x{DFFF}]",
        r"[a-\x{110000}]",
    ] {
        let (stdout, _, status) = matches(pattern)?;
        assert_eq!((stdout.as_str(), status), ("", 5), "{pattern}");
    }
    Ok(())
}
