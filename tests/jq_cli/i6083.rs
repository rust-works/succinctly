//! #6083: under the `n` flag a pattern that `\K` leaves empty (`1\K` on `a1`) is the empty match
//! at the end of what it consumed, as in jq, whose `ONIG_OPTION_FIND_NOT_EMPTY` tests the match
//! as the engine ran it, not the span `\K` keeps. Every expectation was captured from jq 1.7.1
//! (`/usr/bin/jq -nc`). The last rows are the neighbours that must not change: an `n` search
//! that skips an empty match, and `\K` with nothing consumed before it.

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
fn test_a_pattern_k_leaves_empty_is_found_under_n_6083() -> Result<()> {
    assert_rows(&[
        (r#""a1" | [match("1\\K";"n")|[.offset,.length]]"#, "[[2,0]]"),
        (
            r#""a1b" | [match("1\\K";"n")|[.offset,.length]]"#,
            "[[2,0]]",
        ),
        (
            r#""a1" | [match("1\\K";"ng")|[.offset,.length]]"#,
            "[[2,0]]",
        ),
        (
            r#""a1b" | [match("1\\K|b";"ng")|[.offset,.length]]"#,
            "[[2,0]]",
        ),
        (
            r#""a1" | [match("a*\\K";"n")|[.offset,.length]]"#,
            "[[1,0]]",
        ),
        (
            r#""a1b" | [match("a\\K|b";"n")|[.offset,.length]]"#,
            "[[1,0]]",
        ),
        (
            r#""a1b" | [match("a\\K|b";"ng")|[.offset,.length]]"#,
            "[[1,0],[2,1]]",
        ),
        (
            r#""a1" | [match("1*\\K";"n")|[.offset,.length]]"#,
            "[[2,0]]",
        ),
        (
            r#""a1" | [match("1*\\K";"ng")|[.offset,.length]]"#,
            "[[2,0]]",
        ),
        (
            r#""ab" | [match("b\\K";"ng")|[.offset,.length]]"#,
            "[[2,0]]",
        ),
        (
            r#""a1" | [match("a\\K1";"n")|[.offset,.length]]"#,
            "[[1,1]]",
        ),
        (
            r#""a1" | [match("a\\K1";"ng")|[.offset,.length]]"#,
            "[[1,1]]",
        ),
        (
            r#""aab" | [match("a\\Kb";"ng")|[.offset,.length]]"#,
            "[[2,1]]",
        ),
    ])
}

#[test]
fn test_the_other_builtins_find_it_too_6083() -> Result<()> {
    assert_rows(&[
        (r#""a1" | test("1\\K";"n")"#, "true"),
        (r#""a1" | test("\\K";"n")"#, "false"),
        (r#""a1b" | [scan("1\\K";"n")]"#, r#"[""]"#),
        (r#""a1b" | [scan("1\\K|b";"ng")]"#, r#"[""]"#),
        (r#""a1b" | sub("1\\K";"X";"n")"#, r#""a1Xb""#),
        (r#""a1b" | gsub("1\\K";"X";"ng")"#, r#""a1Xb""#),
        (r#""a1b1" | gsub("1\\K";"X";"ng")"#, r#""a1Xb1X""#),
        (r#""a1b" | [splits("1\\K";"n")]"#, r#"["a1","b"]"#),
        (r#""a1b1c" | [splits("1\\K";"ng")]"#, r#"["a1","b1","c"]"#),
    ])
}

#[test]
fn test_n_still_skips_an_empty_match_and_needs_something_consumed_6083() -> Result<()> {
    assert_rows(&[
        (r#""xyz" | [match("x*";"n")|[.offset,.length]]"#, "[[0,1]]"),
        (
            r#""aaxaa" | [match("x*";"n")|[.offset,.length]]"#,
            "[[2,1]]",
        ),
        (r#""a1" | [match("\\K";"n")|[.offset,.length]]"#, "[]"),
        (r#""a1" | [match("(?=1)\\K";"n")|[.offset,.length]]"#, "[]"),
    ])
}
