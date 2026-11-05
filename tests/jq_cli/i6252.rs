//! #6252: Oniguruma's absence operator `(?~X)` was refused where jq answers. In jq 1.7.1 it repeats
//! any character, a newline included, up to where `X` would match (`(?:(?:(?!X)(?s:.))*)`), so
//! `"abcab" | match("(?~ab)"; "g")` is `""`, `"bc"`, `""`, `"b"`, `""`. Every row was captured from
//! jq 1.7.1 (`/usr/bin/jq -nc`) as `[match($p; $f) | [.offset, .string]]`.

use anyhow::Result;

use crate::run_jq_full;

fn matches(subject: &str, pattern: &str, flags: &str) -> Result<(String, i32, String)> {
    let filter = r"$s | [match($p; $f) | [.offset, .string]]";
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-nc", "--arg", "s", subject, "--arg", "p", pattern, "--arg", "f", flags, filter,
        ],
        None,
    )?;
    Ok((stdout.trim_end().to_string(), code, stderr))
}

#[test]
fn test_the_absence_operator_stops_where_its_body_would_match_6252() -> Result<()> {
    for (subject, pattern, flags, expected) in [
        (
            "abcab",
            "(?~ab)",
            "g",
            r#"[[0,""],[1,"bc"],[3,""],[4,"b"],[5,""]]"#,
        ),
        (
            "abcab",
            "(?~ab)+",
            "g",
            r#"[[0,""],[1,"bc"],[3,""],[4,"b"],[5,""]]"#,
        ),
        ("abcab", "(?~ab)", "gn", r#"[[1,"bc"],[4,"b"]]"#),
        (
            "ABcab",
            "(?~ab)",
            "gi",
            r#"[[0,""],[1,"Bc"],[3,""],[4,"b"],[5,""]]"#,
        ),
        ("a\nb", "(?~b)", "g", r#"[[0,"a\n"],[2,""],[3,""]]"#),
        ("xaby", "x(?~b)b", "g", r#"[[0,"xab"]]"#),
        ("abab", "(?~ab)c|ab", "g", r#"[[0,"ab"],[2,"ab"]]"#),
        (
            "abab",
            "(?~ab){2}",
            "g",
            r#"[[0,""],[1,"b"],[2,""],[3,"b"],[4,""]]"#,
        ),
        ("abcab", "(?=(?~ab)c)", "g", r#"[[1,""],[2,""]]"#),
        ("aXbXc", "(?~X)X(?~X)", "g", r#"[[0,"aXb"],[3,"Xc"]]"#),
        // The C comment of the Oniguruma manual.
        (
            "/* a */ b /* c */",
            r"/\*(?~\*/)\*/",
            "g",
            r#"[[0,"/* a */"],[10,"/* c */"]]"#,
        ),
        (
            "foo<b>bar</b>baz",
            "<b>(?~</b>)</b>",
            "g",
            r#"[[3,"<b>bar</b>"]]"#,
        ),
    ] {
        let (stdout, code, stderr) = matches(subject, pattern, flags)?;
        assert_eq!(
            (stdout.as_str(), code),
            (expected, 0),
            "{pattern} on {subject:?}: stderr {stderr}"
        );
    }
    Ok(())
}

/// What is not the operator is not rewritten: `(?~` in a class or after a backslash is text, a
/// comment is copied, and the forms the rewrite does not read stay refused, as before: a range
/// cutter `(?~|…)`, an operator inside another one, and one under a quoted run.
#[test]
fn test_what_is_not_the_absence_operator_is_left_alone_6252() -> Result<()> {
    let (stdout, code, stderr) = matches("a(?~b", r"[(?~]", "g")?;
    assert_eq!(
        (stdout.as_str(), code),
        (r#"[[1,"("],[2,"?"],[3,"~"]]"#, 0),
        "{stderr}"
    );
    let (stdout, code, stderr) = matches("a(?~b)c", r"\(\?~b\)", "g")?;
    assert_eq!((stdout.as_str(), code), (r#"[[1,"(?~b)"]]"#, 0), "{stderr}");
    let (stdout, code, stderr) = matches("abab", r"(?#(?~x)ab", "g")?;
    assert_eq!(
        (stdout.as_str(), code),
        (r#"[[0,"ab"],[2,"ab"]]"#, 0),
        "{stderr}"
    );
    for pattern in ["(?~|ab)", "(?~a(?~b))", r"\Q(?~ab)\E(?~c)", "(?~ab"] {
        let (stdout, code, stderr) = matches("abab", pattern, "g")?;
        assert_ne!(code, 0, "{pattern}: {stdout}");
        assert!(stderr.contains("invalid regex"), "{pattern}: {stderr}");
    }
    Ok(())
}
