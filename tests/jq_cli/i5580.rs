//! #5580: with the `l` flag, an alternation of multi-byte branches gave the shorter one
//! (`"éé" | match("é|éé"; "l")` was `é`). `fancy-regex`'s `leftmost_longest` stops looking for a
//! longer match once one is as long as the pattern's `max_size`, which it counts in characters and
//! compares with a length in bytes. Every row was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

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

/// The issue's rows: the byte length of the shorter branch equals the pattern's length in
/// characters.
#[test]
fn test_l_flag_multibyte_alternation_takes_the_longer_branch_5580() -> Result<()> {
    assert_rows(&[
        (
            r#""éé" | [match("é|éé"; "l") | [.offset, .length]]"#,
            "[[0,2]]",
        ),
        (
            r#""aééé" | [match("é|éé"; "l") | [.offset, .length]]"#,
            "[[1,2]]",
        ),
        (
            r#""aééé" | [match("aé|aéé"; "l") | [.offset, .length]]"#,
            "[[0,3]]",
        ),
        (
            r#""aaa" | [match("a|aa|aaa"; "l") | [.offset, .length]]"#,
            "[[0,3]]",
        ),
    ])
}

/// The same fault with the alternation inside a group, which the issue believed answered
/// correctly (`(?:é|éé)x?` only did because its three characters are never a match's byte length).
#[test]
fn test_l_flag_multibyte_alternation_in_a_group_5580() -> Result<()> {
    assert_rows(&[
        (
            r#""éaé" | [match("(?:éa|éaé)"; "l") | [.offset, .length]]"#,
            "[[0,3]]",
        ),
        (
            r#""aééé" | [match("(?:é|éé)x?"; "l") | [.offset, .length]]"#,
            "[[1,2]]",
        ),
    ])
}

/// A class that is one character and several bytes, captures, `g`, and a case-insensitive flag
/// go through the same search.
#[test]
fn test_l_flag_multibyte_dot_captures_and_flags_5580() -> Result<()> {
    assert_rows(&[
        (
            r#""aéé" | [match("a.|a.."; "l") | [.offset, .length]]"#,
            "[[0,3]]",
        ),
        (
            r#""aéé" | [match("(a)(é|éé)"; "lg") | [.offset, .length, (.captures | map(.string))]]"#,
            r#"[[0,3,["a","éé"]]]"#,
        ),
        (
            r#""aéÉé" | [match("(?i)é|éé"; "l") | [.offset, .length]]"#,
            "[[1,2]]",
        ),
        (
            r#""abaa" | [match("a|aa|aaa"; "lg") | [.offset, .length]]"#,
            "[[2,2]]",
        ),
    ])
}
