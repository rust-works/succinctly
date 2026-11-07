//! #6519: Oniguruma's `tune_look_behind` sizes a look-behind's body before it compiles it. A body
//! that can match the empty string (least length `0`, and sure of it: a group or an assertion makes
//! the length unsure) is no look-behind at all, so the groups in it are never set. Any other body
//! must be of one length, and `(?<=(a)?)` is not, because the group is `(0, 1)` long and not sure to
//! be empty. Every expected answer was captured from jq 1.7.1 (`/usr/bin/jq`); an error is the
//! message `catch` hands back.

use anyhow::Result;

use crate::run_jq_full;

const FILTER: &str =
    r#"try ([$s | match($p; "g") | [.offset, (.captures | map([.offset, .string]))]]) catch ."#;

fn check(rows: &[(&str, &str, &str)]) -> Result<()> {
    for (pattern, subject, want) in rows {
        let (stdout, stderr, code) = run_jq_full(
            &["-nc", "--arg", "p", pattern, "--arg", "s", subject, FILTER],
            None,
        )?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (format!("{want}\n").as_str(), "", 0),
            "{pattern:?} on {subject:?}"
        );
    }
    Ok(())
}

/// A body that can match nothing is no look-behind, so its groups are never set (the issue's rows).
#[test]
fn an_empty_body_never_sets_its_groups_6519() -> Result<()> {
    check(&[
        (r"(?<=(a)|)b", "ab", r"[[1,[[-1,null]]]]"),
        (r"(?<=(?:(a)|))b", "ab", r"[[1,[[-1,null]]]]"),
        (r"(?<=(a))b", "ab", r#"[[1,[[0,"a"]]]]"#),
        (r"(?<=(a)|)b", "b", r"[[0,[[-1,null]]]]"),
        (r"x(?<=(a)|)b", "xb", r"[[0,[[-1,null]]]]"),
        (r"(?<=(a)|b*)b", "bb", r"[[0,[[-1,null]]],[1,[[-1,null]]]]"),
        (r"(?<=(a)|(b)|)b", "ab", r"[[1,[[-1,null],[-1,null]]]]"),
        (r"(?<=(a)|)(b)", "ab", r#"[[1,[[-1,null],[1,"b"]]]]"#),
        (r"((?<=(a)|)b)", "ab", r#"[[1,[[1,"b"],[-1,null]]]]"#),
    ])
}

/// Empty bodies without a group, which the engines do not all read as Oniguruma does.
#[test]
fn an_empty_body_without_a_group_is_no_look_behind_6519() -> Result<()> {
    check(&[
        (r"(?<=a|)b", "ab", r"[[1,[]]]"),
        (r"(?<=a?)b", "ab", r"[[1,[]]]"),
        (r"(?<=a*)b", "ab", r"[[1,[]]]"),
        (r"(?<!a?)b", "ab", r"[]"),
        (r"(?<!a|)b", "ab", r"[]"),
        (r"(?<!(?:a|\b[ab])?)", "a", r"[]"),
        (r"(?<=b|c{0})\z", "ab", r"[[2,[]]]"),
        (r"(?<=(?:a|bc)*)d", "abcd", r"[[3,[]]]"),
        (r"(?<=(?:a|bc){0,2})d", "abcd", r"[[3,[]]]"),
    ])
}

/// A group Oniguruma is not sure is empty keeps the body a look-behind of varying length.
#[test]
fn a_group_makes_a_body_of_varying_length_6519() -> Result<()> {
    check(&[
        (
            r"(?<=(a)?)b",
            "ab",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
        (
            r"(?<=(a)*)b",
            "ab",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
        (
            r"(?<=(a)+)b",
            "ab",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
        (
            r"(?<=b|()|c)\z",
            "ab",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
        (
            r"(?<=bb?)\z",
            "ab",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
        (
            r"(?<=x(a)?)b",
            "xab",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
        (
            r"(?<=(a|b)+)b",
            "ab",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
        (
            r"(?<=(a)|bc)d",
            "abcd",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
        (
            r"(?<=\ba?)b",
            "ab",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
    ])
}

/// Lengths: a body of one length is fine, and the reduction of a plain quantifier counts.
#[test]
fn the_length_of_a_body_is_oniguruma_s_6519() -> Result<()> {
    check(&[
        (r"(?<=a+)b", "aab", r"[[2,[]]]"),
        (r"(?<=a{1,3})b", "aab", r"[[2,[]]]"),
        (r"(?<=b?b)b", "bb", r"[[1,[]]]"),
        (r"(?<=(?:c{2}){1,2})c", "ccc", r"[[2,[]]]"),
        (r"(?<=a*a)b", "aab", r"[[2,[]]]"),
        (
            r"(?<=a*b)b",
            "abb",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
        (
            r"(?<=a*bc)d",
            "abcd",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
        (r"(?<=(?:ab)*abd)c", "ababdc", r"[[5,[]]]"),
        (
            r"(?<=(?:ab)*ac)c",
            "abac",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
        (
            r"(?<=aa*)b",
            "aab",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
        (
            r"(?<=(?:a|b)+)b",
            "ab",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
        (
            r"(?<=(?:ab|cd)+)d",
            "abcd",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
    ])
}

/// A branch of which every item is removed is an error code Oniguruma has no text for.
#[test]
fn a_branch_with_every_item_removed_is_an_undefined_error_6519() -> Result<()> {
    check(&[
        (
            r"(?<=a*b*)b",
            "ab",
            r#""Regex failure: undefined error code""#,
        ),
        (
            r"(?<=a?b?)b",
            "ab",
            r#""Regex failure: undefined error code""#,
        ),
        (
            r"(?<=a|b*c*)b",
            "ab",
            r#""Regex failure: undefined error code""#,
        ),
        (
            r"(?<=(?:a*)(?:b*))b",
            "ab",
            r#""Regex failure: undefined error code""#,
        ),
    ])
}

/// A character written as an escape is one character, a count past 100 bytes is no longer string,
/// and what follows a repeat decides whether a plain item is possessive.
#[test]
fn escapes_counts_and_what_follows_a_repeat_6519() -> Result<()> {
    check(&[
        (r"(?<=\x{e9}|a)b", "éb", r"[[1,[]]]"),
        (r"(?<=(?:\x{e9}{2}){1,2})b", "ééb", r"[[2,[]]]"),
        (r"(?#xy)(?<=(a)|)b", "ab", r"[[1,[[-1,null]]]]"),
        (r"\x{e9}(?<=(a)|)b", "éb", r"[[0,[[-1,null]]]]"),
        (r"(?<=a{0})b", "ab", r"[[1,[]]]"),
        (r"(?<=x|a*)b", "ab", r"[[1,[]]]"),
        (r"(?<=a*(?:b|c))d", "abd", r"[[2,[]]]"),
        (r"(?<=a*(?:a|c))d", "aad", r"[[2,[]]]"),
        (
            r"(?<=a*(b))d",
            "abd",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
        (r"(?<=(?:a+?)*)b", "ab", r"[[1,[]]]"),
        (r"(?<=(?:a+?)+)b", "ab", r"[[1,[]]]"),
        (
            r"(?<=(?:a{300}){300})b",
            "ab",
            r#""Regex failure: invalid pattern in look-behind""#,
        ),
    ])
}
