//! #5529: the `regex` crate accepts two Oniguruma spellings that mean something else in jq, so
//! the pattern compiled and `succinctly jq` answered differently instead of refusing. In jq's
//! Perl syntax `\<` and `\>` are the literal characters `<` and `>`, and `(?P<n>...)` is an
//! error (`undefined group option`). Every row below was captured from jq 1.7.1 (`/usr/bin/jq -n`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_escaped_angle_brackets_are_literals_5529() -> Result<()> {
    for (filter, expected) in [
        (r#""<a>" | [match("\\<a\\>").string]"#, r#"["<a>"]"#),
        (r#""a b" | [match("\\<b").offset]"#, "[]"),
        (r#""a<" | [match("a\\<").string]"#, r#"["a<"]"#),
        (r#""<" | test("[\\<\\>]")"#, "true"),
        (r#""a>" | [match("[\\>]").offset]"#, "[1]"),
        (r#""a<b>" | [match("\\<(.)\\>";"g").string]"#, r#"["<b>"]"#),
        (r#""a<b>" | sub("\\<";"X")"#, r#""aXb>""#),
        (r#""a<b>" | gsub("\\>";"Y")"#, r#""a<bY""#),
        (r#""a<b>" | [scan("\\<.")]"#, r#"["<b"]"#),
        // The `regex` crate's `\b{start}` family is `\b` and a literal `{...}` run in jq.
        (r#""a" | test("\\b{start}a")"#, "false"),
        (r#""b{start}a" | test("\\b{start}a")"#, "true"),
        (r#""a" | test("\\b{end}")"#, "false"),
        (r#""a" | test("\\b{start-half}")"#, "false"),
        (r#""a" | test("\\b{end-half}")"#, "false"),
        // An escaped backslash before `<` is a backslash, then a literal `<`.
        (r#""a\\<" | [match("\\\\<").offset]"#, "[1]"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

#[test]
fn test_python_named_groups_are_refused_5529() -> Result<()> {
    for filter in [
        r#""a" | test("(?P<n>a)")"#,
        r#""a" | capture("(?P<n>a)")"#,
        r#""a" | test("(?P=n)")"#,
        r#""a" | test("(?P>n)")"#,
        r#""a" | test("x(?P<n>a")"#,
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(code, 5, "{filter}: stdout={stdout:?} stderr={stderr:?}");
        assert_eq!(stdout, "", "{filter}");
        assert!(
            stderr.contains("invalid regex: undefined group option"),
            "{filter}: {stderr:?}"
        );
    }
    Ok(())
}

#[test]
fn test_neighbouring_syntax_is_unchanged_5529() -> Result<()> {
    // Oniguruma's own named group, an escaped `(?P` and `(?P` inside a class stay what they were.
    for (filter, expected) in [
        (r#""a" | capture("(?<n>a)")"#, r#"{"n":"a"}"#),
        (r#""(?P" | test("\\(\\?P")"#, "true"),
        (r#""a" | test("[(?P]")"#, "false"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

#[test]
fn test_flags_are_checked_before_the_pattern_is_refused_5529() -> Result<()> {
    // jq reads the modifier string before it compiles the pattern.
    let (stdout, stderr, code) = run_jq_full(&["-nc", r#""a" | test("(?P<n>a)";"z")"#], None)?;
    assert_eq!(code, 5, "stdout={stdout:?} stderr={stderr:?}");
    assert!(
        stderr.contains("z is not a valid modifier string"),
        "{stderr:?}"
    );
    Ok(())
}
