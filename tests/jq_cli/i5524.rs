//! #5524: a possessive quantifier (`*+`, `++`, `?+`, `{n,}+`, `{n,m}+`) is possessive in jq's
//! Oniguruma (the repetition never gives characters back) and a quantifier on a quantifier in the
//! `regex` crate, so `"aaa" | test("a*+a")` was `true` here and `false` in jq, with no error. The
//! crate cannot express it, so the pattern is refused instead of answered wrongly. Every row
//! below is a live jq 1.7.1 capture.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_a_possessive_quantifier_followed_by_more_pattern_is_refused_5524() -> Result<()> {
    for (input, program) in [
        (r#""aaa""#, r#"[match("a*+a").offset]"#),
        (r#""aaa""#, r#"[match("a++a").offset]"#),
        (r#""a""#, r#"[match("a?+a").offset]"#),
        (r#""aaa""#, r#"[match("a{1,}+a").offset]"#),
        (r#""abab""#, r#"[match("(ab)*+ab").offset]"#),
        (r#""aaa""#, r#"test("a*+a")"#),
        (r#""aaa""#, r#"test("^a{2,3}+a$")"#),
        (r#""aaa""#, r#"test("^a{2,}+a$")"#),
        // Inside a group, and a class before it.
        (r#""aaa""#, r#"test("(?:a*+)a")"#),
        (r#""aaa""#, r#"test("[a]*+a")"#),
        // The escaped backslash leaves the `+` a quantifier.
        (r#""aaa""#, r#"test("\\\\*+a")"#),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(input))?;
        assert_eq!((stdout.as_str(), code), ("", 5), "{program}: {stderr}");
        assert!(
            stderr.contains("possessive quantifier"),
            "{program}: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_what_is_not_possessive_is_untouched_5524() -> Result<()> {
    for (input, program, want) in [
        // `{n}+` and a lazy quantifier followed by `+` are a repeat of a repeat in jq too.
        (r#""aaa""#, r#"test("^a{2}+a$")"#, "true\n"),
        (r#""aa""#, r#"test("^a{1,2}?+a$")"#, "true\n"),
        (r#""aaa""#, r#"[match("a*?+").offset]"#, "[0]\n"),
        (r#""aaa""#, r#"test("^a??+a$")"#, "true\n"),
        (r#""aaa""#, r#"test("^a{2}?+a$")"#, "true\n"),
        // Under `x` whitespace separates the `+` from its quantifier, and a comment is skipped.
        (r#""a""#, r#"test("a* +a";"x")"#, "true\n"),
        (r#""aaa""#, r#"test("a # *+ c\na";"x")"#, "true\n"),
        // Closing parentheses are all that follows.
        (r#""aaa""#, r#"test("(?:a++)")"#, "true\n"),
        // An escaped `+`, a `+` in a class, an ordinary `a+` or `(a+)+`.
        (r#""a+""#, r#"test("a\\+")"#, "true\n"),
        (r#""a+""#, r#"test("[a*+]+$")"#, "true\n"),
        (r#""aaa""#, r#"test("(a+)+")"#, "true\n"),
        // Last in the pattern nothing can fail, so it matches what the plain quantifier does.
        (r#""aaa""#, r#"sub("a*+";"X")"#, "\"X\"\n"),
        (r#""baaab""#, r#"[match("a++").offset]"#, "[1]\n"),
        (r#""baaab""#, r#"gsub("a++";"X")"#, "\"bXb\"\n"),
        (r#""baaab""#, r#"gsub("a{1,}+";"X")"#, "\"bXb\"\n"),
        (r#""baaab""#, r#"gsub("a?+";"X")"#, "\"XbXXXXbX\"\n"),
        (r#""baaab""#, r#"gsub("b|a{1,2}+";"X")"#, "\"XXXX\"\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(input))?;
        assert_eq!(
            (stdout.as_str(), code),
            (want, 0),
            "{program} on {input}: {stderr}"
        );
    }
    Ok(())
}

/// jq checks the flags before it compiles the pattern, so a bad flag string is the error even when
/// the pattern is one that is refused.
#[test]
fn test_the_flag_error_comes_before_the_possessive_refusal_5524() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"test("a*+a";"z")"#], Some(r#""aaa""#))?;
    assert_eq!((stdout.as_str(), code), ("", 5), "{stderr}");
    assert!(
        stderr.contains("z is not a valid modifier string"),
        "{stderr}"
    );
    // A `*+` in an `x` comment is not a quantifier; the same text in the pattern is refused.
    let (_, stderr, code) = run_jq_full(&["-c", r#"test("a*+ # c\na";"x")"#], Some(r#""aaa""#))?;
    assert_eq!(code, 5, "{stderr}");
    Ok(())
}
