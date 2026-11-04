//! #5524: a possessive quantifier (`*+`, `++`, `?+`, `{n,}+`, `{n,m}+`) is possessive in jq's
//! Oniguruma (the repetition never gives characters back) and a quantifier on a quantifier in the
//! `regex` crate, so `"aaa" | test("a*+a")` was `true` here and `false` in jq, with no error. The
//! crate cannot express it, so the pattern was refused instead of answered wrongly. Every row
//! below is a live jq 1.7.1 capture. #6172 reads the rest as the atomic group it is.

use anyhow::Result;

use crate::run_jq_full;

/// #6172: a possessive quantifier something follows is the atomic group of its atom and quantifier,
/// `X*+` for `(?>X*)`, so the pattern is read by the second engine and answers as jq does. This was
/// refused (#5524) because the `regex` crate has no atomic group.
#[test]
fn test_a_possessive_quantifier_followed_by_more_pattern_is_atomic_6172() -> Result<()> {
    for (input, program, want) in [
        (r#""aaa""#, r#"[match("a*+a").offset]"#, "[]\n"),
        (r#""aaa""#, r#"[match("a++a").offset]"#, "[]\n"),
        (r#""a""#, r#"[match("a?+a").offset]"#, "[]\n"),
        (r#""aaa""#, r#"[match("a{1,}+a").offset]"#, "[]\n"),
        (r#""abab""#, r#"[match("(ab)*+ab").offset]"#, "[]\n"),
        (r#""aaa""#, r#"test("a*+a")"#, "false\n"),
        (r#""aaa""#, r#"test("^a{2,3}+a$")"#, "false\n"),
        (r#""aaa""#, r#"test("^a{2,}+a$")"#, "false\n"),
        (r#""aaa""#, r#"test("(?:a*+)a")"#, "false\n"),
        (r#""aaa""#, r#"test("[a]*+a")"#, "false\n"),
        (r#""aaa""#, r#"test("\\\\*+a")"#, "true\n"),
        (r#""ab""#, r#"test("b?+*")"#, "true\n"),
        (r#""ab""#, r#"test("a*+*")"#, "true\n"),
        (r#""ab""#, r#"test("a?+{2}")"#, "true\n"),
        (r#""ab""#, r#"test("a++*")"#, "true\n"),
        (
            r#""aaa""#,
            r#"[match("a*+*";"g")|[.offset,.length]]"#,
            "[[0,3],[3,0]]\n",
        ),
        (
            r#""aab""#,
            r#"[match("(?:a*+b)*+";"g")|[.offset,.length]]"#,
            "[[0,3],[3,0]]\n",
        ),
        (r#""aaa""#, r#"test("a{1,2}+*")"#, "true\n"),
        (r#""aab""#, r#"test("a*+(?:b)")"#, "true\n"),
        (r#""aaa""#, r#"test("a*+a*")"#, "true\n"),
        (r#""aA""#, r#"test("(?i)a*+A")"#, "false\n"),
        (r#""aA""#, r#"test("(?i:a)*+A")"#, "false\n"),
        (
            r#""aaab""#,
            r#"[match("(a)*+b").captures[0].offset]"#,
            "[2]\n",
        ),
        (r#""xaay""#, r#"sub("a*+(?<n>y)";"[\(.n)]")"#, "\"x[y]\"\n"),
        (r#""aaa""#, r#"test("^(?=a)a{2,3}+a$")"#, "false\n"),
        (r#""aaa""#, r#"test("a *+ a";"x")"#, "false\n"),
        (r#""aaa""#, r#"test("a*+ # c\na";"x")"#, "false\n"),
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

/// What the scan cannot place an atom for keeps the refusal: a quantifier on a quoted run, on a
/// group of flags, after a back-reference, and a pattern the second engine would read otherwise
/// (a lazy quantifier followed by `+`, which jq reads as a repeat of a repeat).
#[test]
fn test_a_possessive_quantifier_without_a_known_atom_is_refused_5524() -> Result<()> {
    for program in [
        r#"test("\\Qab\\E*+b")"#,
        r#"test("(a)\\1*+a")"#,
        r#"test("(?i)*+a")"#,
        r#"test("c?+a+?+")"#,
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(r#""aab""#))?;
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
/// the pattern is a possessive one.
#[test]
fn test_the_flag_error_comes_before_the_possessive_rewrite_5524() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"test("a*+a";"z")"#], Some(r#""aaa""#))?;
    assert_eq!((stdout.as_str(), code), ("", 5), "{stderr}");
    assert!(
        stderr.contains("z is not a valid modifier string"),
        "{stderr}"
    );
    Ok(())
}
