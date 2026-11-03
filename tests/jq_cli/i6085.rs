//! #6085: under the `l` flag a pattern with `\K` is measured from where its attempt began, as
//! Oniguruma's `ONIG_OPTION_FIND_LONGEST` does, and not from where `\K` moved the start. It used
//! to find nothing (`"a1" | match("a\K1";"l")`) or the wrong match. Every expectation was
//! captured from jq 1.7.1 (`/usr/bin/jq -nc`). The last two rows are the neighbours that must not
//! change: a pattern without `\K` under `l`.
//!
//! Not pinned, because they still differ: a pattern whose ways of matching at one start keep
//! different spans (`"baa" | match("b?\K[ab]a*";"l")` is `[[1,2]]` in jq, `[[0,3]]` here), and `l`
//! with an inline `(?x)`.

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
        (
            r#""\u00e9\u00e91" | [match("\u00e9\\K1";"l")|[.offset,.length]]"#,
            r#"[[2,1]]"#,
        ),
        (
            r#""\u00e9\u00e91\u00e91" | [match("\u00e9\\K1";"lg")|[.offset,.length]]"#,
            r#"[[2,1],[4,1]]"#,
        ),
        (
            r#""a\u00e91a1" | [match("a\\K1|\u00e9\\K1";"lg")|[.offset,.length]]"#,
            r#"[[2,1],[4,1]]"#,
        ),
        (
            r#""a1a1a1" | [match("a\\K1";"lg")|[.offset,.length]]"#,
            r#"[[1,1],[3,1],[5,1]]"#,
        ),
        (
            r#""xaby" | [match("a\\Kb|a";"l")|[.offset,.length]]"#,
            r#"[[2,1]]"#,
        ),
        (
            r#""a1" | [match("a \\K 1";"lx")|[.offset,.length]]"#,
            r#"[[1,1]]"#,
        ),
        (
            r#""a1" | [match("a\\K1 # c";"lx")|[.offset,.length]]"#,
            r#"[[1,1]]"#,
        ),
        (
            r#""a1a1" | [match("a \\K 1";"lgx")|[.offset,.length]]"#,
            r#"[[1,1],[3,1]]"#,
        ),
        (
            r#""a\\K1b" | [match("a\\\\K1";"l")|[.offset,.length]]"#,
            r#"[[0,4]]"#,
        ),
        (r#""a1" | [match("[\\K]1";"l")|[.offset,.length]]"#, r#"[]"#),
        (r#""a1a1" | test("a\\K1";"l")"#, r#"true"#),
        (r#""a1b" | [scan("1\\K|b";"l")]"#, r#"[""]"#),
        (r#""a1a1" | sub("a\\K1";"X";"l")"#, r#""aXa1""#),
        (r#""a1a1" | gsub("a\\K1";"X";"lg")"#, r#""aXaX""#),
        (
            r#""xa1ya1z" | [splits("a\\K1";"lg")]"#,
            r#"["xa","ya","z"]"#,
        ),
    ])
}
