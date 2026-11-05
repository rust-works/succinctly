//! #6106: Oniguruma ends a greedy loop at an iteration that matches nothing, so a loop body whose
//! empty way comes before a consuming one stops there (`"ba" | match("(?:b*|a)+")` is `"b"`), where
//! the `regex` crate dropped the empty iteration and matched `"ba"`. A bounded repeat ends at its
//! empty way the same way, before the copies after it are tried. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -nc`).

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

#[test]
fn test_an_empty_way_before_a_consuming_one_ends_the_loop_6106() -> Result<()> {
    assert_rows(&[
        (r#""ba" | [match("(?:b*|a)+") | .string]"#, r#"["b"]"#),
        (r#""ba" | sub("(?:b*|a)+"; "X")"#, r#""Xa""#),
        (
            r#""foo-bar" | [match("(?:\\w*|-)+") | .string]"#,
            r#"["foo"]"#,
        ),
        (r#""foo-bar" | gsub("(?:\\w*|-)+"; "X")"#, r#""XX-XX""#),
        (
            r#""ba" | [match("(?:b*|a)+"; "g") | .string]"#,
            r#"["b","",""]"#,
        ),
        (r#""ba" | [match("(?:b*|a)*c?") | .string]"#, r#"["b"]"#),
        (r#""ba" | [scan("(?:b*|a)+")]"#, r#"["b","",""]"#),
        (r#""ba" | [splits("(?:b*|a)+")]"#, r#"["","","a",""]"#),
        (
            r#""BA" | [match("(?:b*|a)+"; "gi") | .string]"#,
            r#"["B","",""]"#,
        ),
        (
            r#""b a" | [match("(?: b* | a )+"; "gx") | .string]"#,
            r#"["b","","",""]"#,
        ),
        (r#""ba" | [match("(?:b*|a)+"; "n") | .string]"#, r#"["b"]"#),
        (
            r#""ba" | [match("((?:b*|a)+)") | .captures[0].string]"#,
            r#"["b"]"#,
        ),
    ])
}

/// The minimum count does not keep the loop going past an empty iteration, and a bounded repeat
/// ends at its empty way instead of trying the next copy's ways first.
#[test]
fn test_counted_and_lazy_repeats_end_at_an_empty_iteration_6106() -> Result<()> {
    assert_rows(&[
        (
            r#""bab" | [match("(?:b*|a){2,}"; "g") | .string]"#,
            r#"["b","","b",""]"#,
        ),
        (
            r#""bab" | [match("(?:(?:b|c)||[ab]){2}"; "gn") | .string]"#,
            r#"["b","ab"]"#,
        ),
        (
            r#""abab" | [match("(?:b*|ab.?){1,2}"; "gn") | .string]"#,
            r#"["abab"]"#,
        ),
        (
            r#""aac" | [match("(?:(?:[ab]*|b{2}|c)+?)*") | .string]"#,
            r#"["aa"]"#,
        ),
    ])
}

/// An `x`-mode comment between a group and its quantifier, and a loop nested deep enough that
/// `fancy-regex` (the `n` route, and the route for `$` with `\z`) refuses the rewrite but not the
/// pattern as written.
#[test]
fn test_a_comment_before_the_quantifier_and_a_deep_n_loop_6106() -> Result<()> {
    let deep = format!("{}(?:b*|a)+{}", "(?:".repeat(61), ")".repeat(61));
    let deep_end = format!(r#""ba" | [match("{deep}(?:\\z|$|)"; "g") | .string]"#);
    let deep = format!(r#""ba" | [match("{deep}"; "gn") | .string]"#);
    assert_rows(&[
        (
            r#""ba" | [match("(?:b*|a) #c\n +"; "gx") | .string]"#,
            r#"["b","",""]"#,
        ),
        (
            r#""ba" | [match("(?x)(?:b*|a)#c\n+") | .string]"#,
            r#"["b"]"#,
        ),
        (&deep, r#"["b","a"]"#),
        // `$` and `\z` together take the `fancy-regex` route too.
        (&deep_end, r#"["b","",""]"#),
    ])
}

/// A piece that matches only the empty string is not copied into the rewrite: the pattern keeps
/// the error it has without the loop (jq 1.7.1: `invalid character property name {Foo}`).
#[test]
fn test_a_pattern_refused_without_the_loop_is_refused_with_it_6106() -> Result<()> {
    let (_, plain, plain_code) = run_jq_full(&["-nc", r#""ba" | test("\\p{Foo}{0}")"#], None)?;
    let (stdout, looped, code) =
        run_jq_full(&["-nc", r#""ba" | test("(?:\\p{Foo}{0}|a)+")"#], None)?;
    assert_eq!((stdout.as_str(), code, plain_code), ("", 5, 5));
    assert!(plain.contains("Unicode property not found"), "{plain}");
    assert!(looped.contains("Unicode property not found"), "{looped}");
    Ok(())
}

/// Shapes the two engines already agreed on keep their answers.
#[test]
fn test_loops_the_engines_agree_on_are_unchanged_6106() -> Result<()> {
    assert_rows(&[
        (r#""ba" | [match("(?:a|b*)+") | .string]"#, r#"["ba"]"#),
        (r#""ba" | [match("(?:b*|a)+$") | .string]"#, r#"["ba"]"#),
        (r#""ba" | test("^(?:b*|a)+$")"#, "true"),
        (r#""ba" | [match("(?:b*|a)+"; "l") | .string]"#, r#"["ba"]"#),
        (r#""ba" | [match("(?:b*|a)+?") | .string]"#, r#"["b"]"#),
    ])
}

/// Recorded in `docs/compliance/jq/limitations.md`: a body holding a capture group in a way before
/// the empty one (#6069; a loop over a capture group itself is rewritten since #6305, and a group in a
/// later way since #6310) keeps the longer match. A flag switch in the body is scoped over its ways and ends at the empty iteration
/// as jq's loop does (#6301): `["b","",""]`. (A pattern that goes to `fancy-regex` used to keep the
/// longer match too; it no longer does since #6128, pinned in `i6128.rs`.)
#[test]
fn test_a_capture_still_matches_longer_and_a_flag_switch_no_longer_does_6106() -> Result<()> {
    assert_rows(&[
        // #6310: a group in a way after the empty one is rewritten (`(b*|(a))+` is `["b"]`, as in
        // jq); one in a way before it is written twice by the rewrite, so the loop is left as it was
        // (`["ab","","a",""]` in jq).
        (
            r#""abca" | [match("(?:(a)|b*|c)+"; "g") | .string]"#,
            r#"["abca",""]"#,
        ),
        (
            r#""ba" | [match("(?:(?i)b*|a)+"; "g") | .string]"#,
            r#"["b","",""]"#,
        ),
    ])
}
