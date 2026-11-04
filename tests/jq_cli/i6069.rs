//! #6069: a group under a greedy unbounded repeat whose body can match empty reports Oniguruma's
//! last iteration, the empty one, and not the last non-empty one the `regex` crate keeps:
//! `"ab" | match("(a*)*b")` has the group at `[1,0]`. Every expectation is what jq 1.7.1
//! (`/usr/bin/jq`) prints. A match is shown as `[offset, length]` followed by each group's
//! `[offset, length, name]` (an offset of -1 is a group that took no part).

use anyhow::Result;

use crate::run_jq_null;

/// `$t | [match($p; $f) | ...]` as compact JSON.
fn matches(text: &str, pattern: &str, flags: &str) -> Result<String> {
    let (out, code) = run_jq_null(
        "$t | [match($p; $f) | [.offset, .length] + [.captures[] | [.offset, .length, .name]]]",
        &[
            "-c", "--arg", "t", text, "--arg", "p", pattern, "--arg", "f", flags,
        ],
    )?;
    assert_eq!(code, 0, "{text:?} | match({pattern:?}; {flags:?})");
    Ok(out.trim_end().to_string())
}

fn assert_rows(rows: &[(&str, &str, &str, &str)]) -> Result<()> {
    for &(text, pattern, flags, want) in rows {
        assert_eq!(
            matches(text, pattern, flags)?,
            want,
            "{text:?} | match({pattern:?}; {flags:?})"
        );
    }
    Ok(())
}

#[test]
fn the_group_reports_the_empty_last_iteration() -> Result<()> {
    assert_rows(&[
        ("ab", "(a*)*b", "", "[[0,2,[1,0,null]]]"),
        ("aab", "(a*)+b", "", "[[0,3,[2,0,null]]]"),
        ("abc", "(a|b*)*c", "", "[[0,3,[2,0,null]]]"),
        ("ab", "(a?)*b", "", "[[0,2,[1,0,null]]]"),
        ("ab cd", r"(\w*\s*)*$", "", "[[0,5,[5,0,null]]]"),
        // No non-empty iteration at all: the one iteration is empty.
        ("b", "(a*)*b", "", "[[0,1,[0,0,null]]]"),
    ])
}

/// The loop gives a character back to the rest of the pattern, so where it ends is the engine's
/// call, which is why the capture cannot be fixed after the match.
#[test]
fn the_loop_end_follows_backtracking() -> Result<()> {
    assert_rows(&[("aab", "(a*)*ab", "", "[[0,3,[1,0,null]]]")])
}

/// A body that tries the empty match first ends with the same empty iteration.
#[test]
fn a_body_that_prefers_empty() -> Result<()> {
    assert_rows(&[
        ("ab", "(a??)*b", "", "[[0,2,[1,0,null]]]"),
        ("ab", "(|a)*b", "", "[[0,2,[1,0,null]]]"),
        ("aab", "(a*?)*b", "", "[[0,3,[2,0,null]]]"),
    ])
}

/// A group inside the body keeps what an earlier iteration captured when the empty one skips it,
/// and takes the empty iteration's value when it is on that iteration's path.
#[test]
fn inner_groups_keep_earlier_iterations() -> Result<()> {
    assert_rows(&[
        ("abc", "((a)|b*)*c", "", "[[0,3,[2,0,null],[0,1,null]]]"),
        (
            "abc",
            "((a)|(b*))*c",
            "",
            "[[0,3,[2,0,null],[0,1,null],[2,0,null]]]",
        ),
        ("aab", "((a*)*)*b", "", "[[0,3,[2,0,null],[2,0,null]]]"),
    ])
}

/// An assertion in the body is no obstacle when the body has an empty way around it.
#[test]
fn an_assertion_with_a_way_around_it() -> Result<()> {
    assert_rows(&[
        ("aab", r"(a*|\b)*b", "", "[[0,3,[2,0,null]]]"),
        ("ab", "(a*|^)*b", "", "[[0,2,[1,0,null]]]"),
    ])
}

#[test]
fn names_flags_and_the_global_flag() -> Result<()> {
    assert_rows(&[
        ("aab", "(?<x>a*)*b", "", r#"[[0,3,[2,0,"x"]]]"#),
        (
            "aab ab",
            "(a*)*b",
            "g",
            "[[0,3,[2,0,null]],[4,2,[5,0,null]]]",
        ),
        ("AAb", "(A*)*b", "i", "[[0,3,[2,0,null]]]"),
        ("aab", "(?i)(A*)*B", "", "[[0,3,[2,0,null]]]"),
        ("aab", "( a* )* b", "x", "[[0,3,[2,0,null]]]"),
    ])
}

/// Repeats the crate already reports as jq does are left alone: a body that cannot match empty,
/// a lazy repeat, a counted one from `{2,}` up, and a body with an assertion.
#[test]
fn other_repeats_are_unchanged() -> Result<()> {
    assert_rows(&[
        ("aab", "(a+)*b", "", "[[0,3,[0,2,null]]]"),
        ("aab", "(a*)*?b", "", "[[0,3,[0,2,null]]]"),
        ("aab", "(a*){2,}b", "", "[[0,3,[2,0,null]]]"),
        ("ab", "(a|$)*", "", "[[0,1,[0,1,null]]]"),
    ])
}

#[test]
fn capture_scan_and_sub_read_the_same_groups() -> Result<()> {
    for (filter, want) in [
        (
            r#""aab" | capture("(?<x>a*)*(?<y>b)")"#,
            r#"{"x":"","y":"b"}"#,
        ),
        (r#""aab ab" | [scan("(a*)*b")]"#, r#"[[""],[""]]"#),
        (r#""aab" | sub("(?<x>a*)*b"; "[\(.x)]")"#, r#""[]""#),
        // `$` before a final newline: the trimmed view of the text answers.
        (r#""aab\n" | [match("(a*)*$").captures[0].offset]"#, "[3]"),
    ] {
        let (out, code) = run_jq_null(filter, &["-c"])?;
        assert_eq!((out.trim_end(), code), (want, 0), "{filter}");
    }
    Ok(())
}

/// A loop nested in a rewritten body, a body whose repeat counts and groups decide whether it can
/// match empty, and a flag group inside the repeat: the shapes the rewrite walks past without
/// changing the answer jq gives.
#[test]
fn nested_and_counted_bodies_report_jqs_groups() -> Result<()> {
    assert_rows(&[
        ("abcabc", "((a|b*)*c)*", "", "[[0,6,[3,3,null],[5,0,null]]]"),
        ("aabc", "((a+)|b)*c", "", "[[0,4,[2,1,null],[0,2,null]]]"),
        ("aab", "(a{2})*b", "", "[[0,3,[0,2,null]]]"),
        ("aabc", "(a{1,2}|b?)*c", "", "[[0,4,[3,0,null]]]"),
        ("ab", "((a?)*)*b", "", "[[0,2,[1,0,null],[1,0,null]]]"),
        ("abac", "((a)b?)*c", "", "[[0,4,[2,1,null],[2,1,null]]]"),
        ("abcd", "((b|c*)|a)*d", "", "[[0,4,[3,0,null],[3,0,null]]]"),
        ("AAb", "(((?i)a)*)*b", "", "[[0,3,[2,0,null],[1,1,null]]]"),
        ("aAb", "((?i:a)*)*b", "", "[[0,3,[2,0,null]]]"),
        (
            "abdcde",
            "(([ab]|c*)*d)*e",
            "",
            "[[0,6,[3,2,null],[4,0,null]]]",
        ),
        (
            "xabxby",
            "(x(a|b*)*)*y",
            "",
            "[[0,6,[3,2,null],[5,0,null]]]",
        ),
        (
            "abc",
            r"(((?:\b|a)|)b*)*c",
            "",
            "[[0,3,[2,0,null],[2,0,null]]]",
        ),
        (
            "aabc",
            r"(((?:\ba|a)|)b*)*c",
            "",
            "[[0,4,[3,0,null],[3,0,null]]]",
        ),
        ("aaab", "(a{2,}|)*b", "", "[[0,4,[3,0,null]]]"),
        ("ab", r"((?:\b|a)|b?)*b", "", "[[0,2,[1,0,null]]]"),
        ("abab", "((?:^|a)|b?)*b", "", "[[0,4,[3,0,null]]]"),
        (
            "xbc",
            r"((?:\ba?)(x)?|b*)*c",
            "",
            "[[0,3,[2,0,null],[0,1,null]]]",
        ),
        (
            "axbc",
            r"((?:\b|a)(x)?|b*)*c",
            "",
            "[[0,4,[3,0,null],[1,1,null]]]",
        ),
        (
            "xxc",
            r"((?:\ba?)(x)?|b*)*c",
            "",
            "[[2,1,[2,0,null],[-1,0,null]]]",
        ),
        (
            "bxc",
            r"((?:\b|a)(x)?|b*)*c",
            "",
            "[[2,1,[2,0,null],[-1,0,null]]]",
        ),
    ])
}
