//! #6085: under the `l` flag a pattern with `\\K` is measured from where its attempt began, as
//! Oniguruma's `ONIG_OPTION_FIND_LONGEST` does, and not from where `\\K` moved the start. It used
//! to find nothing (`"a1" | match("a\\K1";"l")`) or the wrong match. Every expectation was
//! captured from jq 1.7.1 (`/usr/bin/jq -nc`). The last two rows are the neighbours that must not
//! change: a pattern without `\\K` under `l`.
//!
//! Not pinned, because they still differ: a pattern whose ways of matching at one start keep
//! different spans (`"baa" | match("b?\\K[ab]a*";"l")` is `[[1,2]]` in jq, `[[0,3]]` here), and `l`
//! with the extended mode.

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
fn test_l_measures_from_the_attempt_not_from_k_6085() -> Result<()> {
    assert_rows(&[
        (
            r#""a1" | [match("a\\K1";"l")|[.offset,.length]]"#,
            "[[1,1]]",
        ),
        (
            r#""ab" | [match("a\\Kb";"l")|[.offset,.length]]"#,
            "[[1,1]]",
        ),
        (r#""a1" | [match("1\\K";"l")|[.offset,.length]]"#, "[[2,0]]"),
        (
            r#""a1" | [match("1*\\K";"l")|[.offset,.length]]"#,
            "[[2,0]]",
        ),
        (
            r#""a1b" | [match("1\\K|b";"l")|[.offset,.length]]"#,
            "[[2,0]]",
        ),
        (
            r#""a1b" | [match("a\\K|b";"l")|[.offset,.length]]"#,
            "[[1,0]]",
        ),
        (
            r#""a1" | [match("a\\K1";"lg")|[.offset,.length]]"#,
            "[[1,1]]",
        ),
        (
            r#""a1" | [match("1\\K";"ln")|[.offset,.length]]"#,
            "[[2,0]]",
        ),
        (
            r#""a1" | [match("a*\\K";"ln")|[.offset,.length]]"#,
            "[[1,0]]",
        ),
        (
            r#""a1" | [match("1*\\K";"ln")|[.offset,.length]]"#,
            "[[2,0]]",
        ),
        (
            r#""a1b" | [match("a\\K|b";"ln")|[.offset,.length]]"#,
            "[[1,0]]",
        ),
        (
            r#""a1b" | [match("1\\K|b";"ln")|[.offset,.length]]"#,
            "[[2,0]]",
        ),
        (
            r#""aab" | [match("a\\Kb";"l")|[.offset,.length]]"#,
            "[[2,1]]",
        ),
        (r#""xyz" | [match("x*";"l")|[.offset,.length]]"#, "[[0,1]]"),
        (
            r#""abaa" | [match("a|aa|aaa";"l")|[.offset,.length]]"#,
            "[[2,2]]",
        ),
        (
            r#""aab1" | [match("a*\\K1";"l")|[.offset,.length]]"#,
            "[[3,1]]",
        ),
        (
            r#""a1a1" | [match("a\\K1";"lg")|[.offset,.length]]"#,
            "[[1,1],[3,1]]",
        ),
        (
            r#""a1a1" | [match("1\\K";"lg")|[.offset,.length]]"#,
            "[[2,0],[4,0]]",
        ),
        (r#""ab" | [match("b\\K";"l")|[.offset,.length]]"#, "[[2,0]]"),
    ])
}
