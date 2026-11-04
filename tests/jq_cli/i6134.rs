//! #6134: an alternation whose branches start with the same expression that can match more than
//! one way (`a*a|a*b`) takes its first branch's match, as Oniguruma does. `regex-syntax` factored the
//! shared `a*` out (`a*[ab]`, rust-lang/regex#1400), so the second branch's longer match won:
//! `"aab" | match("a*a|a*b")` was `"aab"`. Every row was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

fn assert_rows(rows: &[(&str, &str)]) -> Result<()> {
    for (filter, expected) in rows {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (*expected, 0),
            "{filter}: stderr {stderr}"
        );
    }
    Ok(())
}

/// The issue's rows: a lazy repeat leaves after the first branch's match.
#[test]
fn test_a_lazy_repeat_of_a_shared_prefix_alternation_leaves_early_6134() -> Result<()> {
    assert_rows(&[
        (r#""aab" | [match("(?:a*a|a*b)+?") | .string]"#, r#"["aa"]"#),
        (r#""ab" | [match("(?:a*a+|a*b)+?") | .string]"#, r#"["a"]"#),
        (
            r#""ab" | [match("(?:a*a+|a*b){1,3}?") | .string]"#,
            r#"["a"]"#,
        ),
        (
            r#""ab" | [match("(?:a*a+|a*(?:b|c)){1,3}?") | .string]"#,
            r#"["a"]"#,
        ),
    ])
}

/// No repeat is needed around the alternation: every builtin that reads a match's text, every flag
/// and every engine route gets the first branch's match.
#[test]
fn test_the_first_branch_of_a_shared_prefix_alternation_wins_6134() -> Result<()> {
    assert_rows(&[
        (r#""aab" | [match("a*a|a*b") | .string]"#, r#"["aa"]"#),
        (r#""aab" | sub("a*a|a*b"; "X")"#, r#""Xb""#),
        (r#""aab" | gsub("a*a|a*b"; "X")"#, r#""XX""#),
        (
            r#""aab" | [match("a*a|a*b"; "g") | .string]"#,
            r#"["aa","b"]"#,
        ),
        (r#""abbc" | [match("ab*b|ab*c") | .string]"#, r#"["abb"]"#),
        (r#""ab" | [match("a*?b|a*?a") | .string]"#, r#"["ab"]"#),
        (
            r#""aab" | [match("(a*a|a*b)") | .captures[0].string]"#,
            r#"["aa"]"#,
        ),
        (r#""AAB" | [match("(?i)a*a|A*B") | .string]"#, r#"["AA"]"#),
        (
            r#""aab" | [match("a*a | a*b # comment"; "x") | .string]"#,
            r#"["aa"]"#,
        ),
        (r#""aab" | [match("a*a|a*b"; "n") | .string]"#, r#"["aa"]"#),
        (r#""aab" | [match("a*a|a*b"; "l") | .string]"#, r#"["aab"]"#),
        (
            r#""aab\n" | [match("(?:a*a|a*b)$\\n") | .string]"#,
            r#"["aab\n"]"#,
        ),
        // The dead branch carries the first branch's anchors, which still hold.
        (r#""aab" | [match("^a*a|^a*b") | .string]"#, r#"["aa"]"#),
        (r#""zaab" | [match("^a*a|^a*b") | .string]"#, "[]"),
        (r#""aab" | [match("a*a$|a*b$") | .string]"#, r#"["aab"]"#),
    ])
}

/// The Oniguruma syntax only `fancy-regex` reads: it hands an alternation it does not run itself to
/// the same factoring constructor.
#[test]
fn test_a_shared_prefix_alternation_beside_oniguruma_syntax_6134() -> Result<()> {
    assert_rows(&[
        (
            r#""aab" | [match("(?=a)(?:a*a|a*b)") | .string]"#,
            r#"["aa"]"#,
        ),
        (
            r#""xaab" | [match("(?<=x)(?:a*a|a*b)") | .string]"#,
            r#"["aa"]"#,
        ),
        (r#""aab" | [match("(?>a*a|a*b)") | .string]"#, r#"["aa"]"#),
        (
            r#""xaab" | [match("(x)\\1?(?:a*a|a*b)") | .string]"#,
            r#"["xaa"]"#,
        ),
        (
            r#""xaab" | [match("(?<n>x)\\k<n>?(?:a*a|a*b)") | .string]"#,
            r#"["xaa"]"#,
        ),
        (
            r#""xaab" | [match("x\\K(?:a*a|a*b)") | .string]"#,
            r#"["aa"]"#,
        ),
        (
            r#""aab" | [match("(?#c)(?:a*a|a*b)") | .string]"#,
            r#"["aa"]"#,
        ),
        (
            r#""aab" | [match("(?#a|b)a*a|a*b") | .string]"#,
            r#"["aa"]"#,
        ),
    ])
}
