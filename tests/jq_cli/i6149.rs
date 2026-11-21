//! #6149: `regex-syntax` 0.8.12 keeps a prefix that can match more than one way in each branch of an
//! alternation (rust-lang/regex#1400, fixed by rust-lang/regex#1401), so the guard #6134 put in front
//! of such an alternation is gone. These rows pin the first branch's match across the repetition
//! operators, nested and capturing groups, the `i` flag, every regex builtin and both engine routes,
//! so that a `regex-syntax` that factors the prefix again (`a*a|a*b` read as `a*[ab]`) fails here.
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

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

/// Every repetition operator, greedy, lazy and possessive, as the shared prefix.
#[test]
fn test_each_repetition_operator_as_a_shared_prefix_6149() -> Result<()> {
    assert_rows(&[
        (r#""aab" | [match("a+a|a+b") | .string]"#, r#"["aa"]"#),
        (r#""ab" | [match("a?a|a?b") | .string]"#, r#"["a"]"#),
        (
            r#""aaab" | [match("a{1,3}a|a{1,3}b") | .string]"#,
            r#"["aaa"]"#,
        ),
        (
            r#""aaab" | [match("a{1,}a|a{1,}b") | .string]"#,
            r#"["aaa"]"#,
        ),
        (r#""aab" | [match("a*?b|a*?a") | .string]"#, r#"["aab"]"#),
        (r#""aab" | [match("a+?b|a+?a") | .string]"#, r#"["aab"]"#),
        (r#""aab" | [match("a??a|a??b") | .string]"#, r#"["a"]"#),
        (r#""aab" | [match("a*+a|a*+b") | .string]"#, r#"["aab"]"#),
        (r#""abbc" | [match("ab*b|ab*c") | .string]"#, r#"["abb"]"#),
        (
            r#""abbc" | [match("ab+?c|ab+?b") | .string]"#,
            r#"["abbc"]"#,
        ),
        (r#""aab" | [match("\\w*a|\\w*b") | .string]"#, r#"["aa"]"#),
        (r#""aab" | [match("[ab]*a|[ab]*c") | .string]"#, r#"["aa"]"#),
        (
            r#""aab" | [match("(?:a|b)*a|(?:a|b)*b") | .string]"#,
            r#"["aa"]"#,
        ),
        (
            r#""abab" | [match("(?:ab)*a|(?:ab)*b") | .string]"#,
            r#"["aba"]"#,
        ),
        (r#""aab" | [match("a*a|a*b|a*c") | .string]"#, r#"["aa"]"#),
    ])
}

/// The alternation nested in groups, capturing ones included, inside a sequence and under a repeat.
#[test]
fn test_a_shared_prefix_alternation_in_nested_groups_6149() -> Result<()> {
    assert_rows(&[
        (
            r#""aab" | [match("(?:(?:a*)a|(?:a*)b)") | .string]"#,
            r#"["aa"]"#,
        ),
        (
            r#""aab" | [match("((a*)a|(a*)b)") | .captures | map(.string)]"#,
            r#"[["aa","a",null]]"#,
        ),
        (
            r#""xaaby" | [match("x(?:a*a|a*b)") | .string]"#,
            r#"["xaa"]"#,
        ),
        (
            r#""aab" | [match("(?:a*(?:a|c)|a*b)") | .string]"#,
            r#"["aa"]"#,
        ),
        (r#""aab" | [match("^(?:a*a|a*b)") | .string]"#, r#"["aa"]"#),
        (r#""aab" | [match("(?:a*a|a*b)+") | .string]"#, r#"["aab"]"#),
        (r#""aab" | [match("(?:a*a|a*b)*?") | .string]"#, r#"[""]"#),
        // Under `l` the longest match wins whatever the branch order.
        (
            r#""aab" | [match("a*a|a*b"; "gl") | .string]"#,
            r#"["aab"]"#,
        ),
    ])
}

/// The `i` flag, as a flag argument and inline.
#[test]
fn test_a_shared_prefix_alternation_under_the_i_flag_6149() -> Result<()> {
    assert_rows(&[
        (r#""AAB" | [match("a*a|a*b"; "i") | .string]"#, r#"["AA"]"#),
        (
            r#""AAB" | [match("a*a|a*b"; "gi") | .string]"#,
            r#"["AA","B"]"#,
        ),
        (r#""AAB" | [match("(?i:a*a|a*b)") | .string]"#, r#"["AA"]"#),
        (r#""AAB" | capture("(?<x>a*a|a*b)"; "i")"#, r#"{"x":"AA"}"#),
        (r#""AABaab" | gsub("a*a|a*b"; "X"; "i")"#, r#""XXXX""#),
        (r#""AAB" | [scan("a*a|a*b"; "i")]"#, r#"["AA","B"]"#),
        (
            r#""AABcaab" | [splits("a*a|a*b"; "i")]"#,
            r#"["","","c","",""]"#,
        ),
    ])
}

/// `test`, `capture`, `sub`, `gsub`, `scan` and `splits` read the same first-branch match.
#[test]
fn test_every_regex_builtin_takes_the_first_branch_6149() -> Result<()> {
    assert_rows(&[
        (r#""aab" | test("^(?:a*a|a*b)$")"#, "true"),
        (r#""aab" | test("a*a|a*b")"#, "true"),
        (r#""aab" | capture("(?<x>a*a|a*b)")"#, r#"{"x":"aa"}"#),
        (r#""aab" | sub("(?<x>a*a|a*b)"; "<\(.x)>")"#, r#""<aa>b""#),
        (r#""aabaab" | sub("a*a|a*b"; "X"; "g")"#, r#""XXXX""#),
        (r#""aabaab" | gsub("a*a|a*b"; "X")"#, r#""XXXX""#),
        (r#""aabaab" | [scan("a*a|a*b")]"#, r#"["aa","b","aa","b"]"#),
        (
            r#""aabaab" | [scan("(a*a|a*b)")]"#,
            r#"[["aa"],["b"],["aa"],["b"]]"#,
        ),
        (r#""aabcaab" | [splits("a*a|a*b")]"#, r#"["","","c","",""]"#),
    ])
}

/// The `fancy-regex` route (lookaround, a backreference, an atomic group) hands the alternation to
/// the same `regex-syntax` constructor.
#[test]
fn test_a_shared_prefix_alternation_on_the_fancy_regex_route_6149() -> Result<()> {
    assert_rows(&[
        (
            r#""xaab" | [match("(?<=x)(?:a*a|a*b)") | .string]"#,
            r#"["aa"]"#,
        ),
        (
            r#""aab" | [match("(?=a)(?:a*a|a*b)") | .string]"#,
            r#"["aa"]"#,
        ),
        (
            r#""aab" | [match("(?!b)(?:a*a|a*b)") | .string]"#,
            r#"["aa"]"#,
        ),
        (
            r#""aab" | [match("(a)\\1*a|(a)\\1*b") | .string]"#,
            r#"["aa"]"#,
        ),
        (r#""aab" | [match("(?>a*a|a*b)") | .string]"#, r#"["aa"]"#),
    ])
}

/// The shapes the #6134 guard could not read kept the factored match: a `(?#...)` comment that
/// crosses a line (the guard's copy of the pattern gave up on it, since under `x` the newline can
/// end a `#` comment) took the second branch's `"aab"`. The crate's own parser now keeps the prefix.
#[test]
fn test_a_shared_prefix_alternation_beside_a_multi_line_comment_6149() -> Result<()> {
    assert_rows(&[
        (
            r#""aab" | [match("(?#a\nb)a*a|a*b") | .string]"#,
            r#"["aa"]"#,
        ),
        (
            r#""aab" | [match("(?=a)(?#x\ny)(?:a*a|a*b)") | .string]"#,
            r#"["aa"]"#,
        ),
    ])
}
