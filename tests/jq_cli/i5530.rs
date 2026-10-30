//! #5530: jq's `l` flag is Oniguruma's `ONIG_OPTION_FIND_LONGEST` (the longest match anywhere, not
//! the leftmost) and `n` is `ONIG_OPTION_FIND_NOT_EMPTY` (an empty match is refused and the search
//! backtracks to another way to match at the same position). `fancy-regex` has both, so a pattern
//! that asks for one is compiled by it. Every row was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

/// `succinctly jq -nc <filter>`: stdout, stderr and the exit code.
fn run(filter: &str) -> Result<(String, String, i32)> {
    run_jq_full(&["-nc", filter], None)
}

fn assert_rows(rows: &[(&str, &str)]) -> Result<()> {
    for (filter, expected) in rows {
        let (stdout, stderr, code) = run(filter)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (*expected, 0),
            "{filter}: stderr {stderr}"
        );
    }
    Ok(())
}

/// The longest match at or after the search start wins, and it can begin after the leftmost
/// match: `"abaa"` is matched by `"aa"` at offset 2, not by `"a"` at offset 0.
#[test]
fn test_l_flag_takes_the_longest_match_anywhere_5530() -> Result<()> {
    assert_rows(&[
        (
            r#""abaa" | [match("a|aa|aaa"; "gl") | [.offset, .string]]"#,
            r#"[[2,"aa"]]"#,
        ),
        (
            r#""abaa" | [match("a|aa|aaa"; "l") | [.offset, .string]]"#,
            r#"[[2,"aa"]]"#,
        ),
        (r#""aaa" | sub("a|aa|aaa"; "X"; "l")"#, r#""X""#),
        (r#""abaaab" | gsub("a|aa|aaa"; "X"; "l")"#, r#""abXb""#),
        (r#""abaaab" | gsub("a|aa|aaa"; "X"; "gl")"#, r#""abXb""#),
        (
            r#""abcd" | match("(a|ab)(c|bcd)"; "l") | [.string, (.captures | map(.string))]"#,
            r#"["abcd",["a","bcd"]]"#,
        ),
        (
            r#""xyz" | [match("x*"; "gl") | [.offset, .length]]"#,
            r#"[[0,1],[1,0],[2,0],[3,0]]"#,
        ),
        (
            r#""abcabcab" | [match("abc|ab|c"; "gl") | .string]"#,
            r#"["abc","abc","ab"]"#,
        ),
        (
            r#""foobar" | [match("(?<=foo)(b|ba|bar)"; "l") | .string]"#,
            r#"["bar"]"#,
        ),
        (r#""ABB" | [match("b|bb"; "gli") | .string]"#, r#"["BB"]"#),
        (
            r#""éaéaa" | [match("a|aa"; "gl") | [.offset, .string]]"#,
            r#"[[3,"aa"]]"#,
        ),
        (
            r#""1a22b333" | [splits("[0-9]|[0-9]{2}"; "gl")]"#,
            r#"["1a","b","",""]"#,
        ),
        (r#""abc" | test("a|ab"; "l")"#, r#"true"#),
        (r#""aaa" | [scan("(a|aa)(a|aa)?"; "l")]"#, r#"[["a","aa"]]"#),
        (
            r#""abab" | [match("(ab|a)(ba|b)*"; "l") | [.string, (.captures | map(.string))]]"#,
            r#"[["abab",["a","b"]]]"#,
        ),
        (
            r#""ab" | [capture("(?<x>a|ab)(?<y>b?)"; "l")]"#,
            r#"[{"x":"a","y":"b"}]"#,
        ),
        (
            r#""abaa" | [match("a|aa|aaa"; "gln") | .string]"#,
            r#"["aa"]"#,
        ),
        (
            r#""aaaa" | [match("(?=a)(a|aa)"; "gl") | .string]"#,
            r#"["aa","aa"]"#,
        ),
    ])
}

/// `n` backtracks to a non-empty match at the same position, which a lazy quantifier or an
/// empty-first alternation needs.
#[test]
fn test_n_flag_backtracks_to_a_non_empty_match_5530() -> Result<()> {
    assert_rows(&[
        (
            r#""xaab" | [match("a*?"; "gn") | [.offset, .length]]"#,
            r#"[[1,1],[2,1]]"#,
        ),
        (
            r#""abcabc" | [match("(?:|abc)"; "gn") | [.offset, .string]]"#,
            r#"[[0,"abc"],[3,"abc"]]"#,
        ),
        (r#""a,b;c" | [splits("[,;]*?"; "gn")]"#, r#"["a","b","c"]"#),
        (
            r#""aaa" | [match("(?=a)a??"; "gn") | .string]"#,
            r#"["a","a","a"]"#,
        ),
        (r#""xaab" | gsub("(?:|a)"; "-"; "gn")"#, r#""x--b""#),
        (r#""abc" | [match("^"; "n")]"#, r#"[]"#),
        (r#""abc" | [match("\\b"; "gn")]"#, r#"[]"#),
        (
            r#""aa" | [match("(a)\\1??"; "gn") | .string]"#,
            r#"["a","a"]"#,
        ),
        (r#""ab" | [match("(a|)(b|)"; "n") | .string]"#, r#"["ab"]"#),
        (r#""xaab" | test("a*?"; "n")"#, r#"true"#),
        (
            r#""aaa" | [match("a??"; "gn") | .string]"#,
            r#"["a","a","a"]"#,
        ),
        (
            r#""ab12" | [scan("[a-z]*?[0-9]*?"; "gn")]"#,
            r#"["a","b","1","2"]"#,
        ),
    ])
}

/// A pattern with a lazy quantifier keeps `l` as a no-op: `fancy-regex`'s longest-match mode
/// returns the lazy match, where Oniguruma returns the longest of all the ways to match
/// (`"aa" | match("a*?"; "l")` is `"aa"` in jq 1.7.1). Pinned as the divergence it is
/// (docs/compliance/jq/limitations.md), so closing it is a visible change.
#[test]
fn test_l_flag_on_a_lazy_quantifier_is_still_a_no_op_5530() -> Result<()> {
    let (stdout, stderr, code) = run(r#""aa" | [match("a*?"; "l") | .string]"#)?;
    assert_eq!((stdout.trim_end(), code), (r#"[""]"#, 0), "stderr {stderr}");
    // The same pattern with `n` is not affected by `l` being a no-op: `ln` keeps its old route
    // whole, so it also keeps the skip-forward `n`, which finds no match for a lazy empty-first
    // pattern that matches nothing else at its start.
    let (stdout, _, code) = run(r#""aa" | [match("a*?"; "ln") | .string]"#)?;
    assert_eq!((stdout.trim_end(), code), ("[]", 0));
    Ok(())
}
