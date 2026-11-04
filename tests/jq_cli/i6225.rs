//! #6225: Oniguruma's multi-character case folding under the `i` option. `ß` is `ss`, `ﬁ` is `fi`
//! and `ﬃ` is `ffi`, which the `regex` crate (one-to-one folds only) did not read. The pattern is
//! rewritten to the expansion Oniguruma makes of it (`unravel_case_fold_string`): a run of literals
//! is walked from the left, each fold found there (three characters, then two, then one) becomes an
//! alternation, and the walk goes on past the characters it covered. Every row was captured from
//! jq 1.7.1 (`/usr/bin/jq -nc`), the segmentation rows (`sss` against `sß`) included.

use anyhow::Result;

use crate::run_jq_full;

fn check(rows: &[(&str, &str)]) -> Result<()> {
    for (filter, expected) in rows {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), *expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

/// A character that folds to several, and the spellings of its fold, are read both ways (the table of #6225).
#[test]
fn a_character_that_folds_to_several_matches_its_spellings_6225() -> Result<()> {
    check(&[
        (
            r#""SS" | [match("ß"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""STRASSE" | [match("ß"; "gi") | [.offset,.length]]"#,
            r"[[4,2]]",
        ),
        (
            r#""Stra\u00dfe" | [match("ss"; "gi") | [.offset,.length]]"#,
            r"[[4,1]]",
        ),
        (
            r#""ss" | [match("ẞ"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""ss" | [match("[ß]"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""SS" | [match("ß+"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""fi" | [match("ﬁ"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""\ufb01" | [match("fi"; "gi") | [.offset,.length]]"#,
            r"[[0,1]]",
        ),
        (
            r#""ff" | [match("ﬀ"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""STRASSE" | [match("straße"; "gi") | [.offset,.length]]"#,
            r"[[0,7]]",
        ),
        (
            r#""\u00df" | [match("s+"; "gi") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""\u00df" | [match("[s]"; "gi") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""\u00df" | [match("s"; "gi") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""\u017fs" | [match("ß"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""\u00df" | [match("ss"; "gi") | [.offset,.length]]"#,
            r"[[0,1]]",
        ),
        (
            r#""\u1e9e" | [match("ss"; "gi") | [.offset,.length]]"#,
            r"[[0,1]]",
        ),
    ])
}

/// The fold found at a place covers the characters it read, and the walk goes on after them: `sss` is `ss` then `s`, so it finds `ßs` and not `sß`; `ffi` is a three-character fold, so `ﬀi` is not it.
#[test]
fn a_fold_is_found_from_the_left_and_covers_what_it_reads_6225() -> Result<()> {
    check(&[
        (
            r#""\ufb03" | [match("ffi"; "gi") | [.offset,.length]]"#,
            r"[[0,1]]",
        ),
        (
            r#""ffi" | [match("ﬃ"; "gi") | [.offset,.length]]"#,
            r"[[0,3]]",
        ),
        (
            r#""\ufb00i" | [match("ffi"; "gi") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""f\ufb01" | [match("ffi"; "gi") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""ffi" | [match("ﬀi"; "gi") | [.offset,.length]]"#,
            r"[[0,3]]",
        ),
        (
            r#""\u00dfs" | [match("sss"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""s\u00df" | [match("sss"; "gi") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""\u00df\u00df" | [match("ssss"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""ssss" | [match("ßß"; "gi") | [.offset,.length]]"#,
            r"[[0,4]]",
        ),
        (
            r#""s\u00dfs" | [match("ssß"; "gi") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""\u00dfss" | [match("ssß"; "gi") | [.offset,.length]]"#,
            r"[[0,3]]",
        ),
    ])
}

/// A string is the literals with nothing between them but a group that is not repeated, an option that leaves `i` as it was, or a blank in the extended mode; a quantifier takes the last character off it.
#[test]
fn a_string_is_literals_with_only_a_plain_group_between_6225() -> Result<()> {
    check(&[
        (
            r#""\u00df" | [match("(?:s)s"; "gi") | [.offset,.length]]"#,
            r"[[0,1]]",
        ),
        (
            r#""\u00df" | [match("s(?:s)"; "gi") | [.offset,.length]]"#,
            r"[[0,1]]",
        ),
        (
            r#""\u00df" | [match("(?:ss)"; "gi") | [.offset,.length]]"#,
            r"[[0,1]]",
        ),
        (
            r#""\u00df\u00df" | [match("(?:ss)+"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""\u00df\u00df" | [match("s(?:s)+"; "gi") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""\u00df" | [match("(s)s"; "gi") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""\u00df" | [match("s(?i)s"; "g") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""\u00df" | [match("s(?i:s)"; "gi") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""\u00df" | [match("(?i)ss"; "g") | [.offset,.length]]"#,
            r"[[0,1]]",
        ),
        (
            r#""\u00df" | [match("(?-i)ss"; "gi") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""\u00df" | [match("ss"; "g") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""\u00df" | [match("s\\x73"; "gi") | [.offset,.length]]"#,
            r"[[0,1]]",
        ),
        (
            r#""ss" | [match("\\x{DF}"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""\u00df" | [match("ss*"; "gi") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""x\u00df\u00df" | [match("xss*"; "gi") | [.offset,.length]]"#,
            r"[]",
        ),
    ])
}

/// A bracketed class that holds a character folding to several gains the alternative that spells it; a negated class and one holding only the letters do not.
#[test]
fn a_class_holding_a_folding_character_gains_the_spelling_6225() -> Result<()> {
    check(&[
        (
            r#""ss" | [match("[ß-ß]"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""ss" | [match("[^ß]"; "gi") | [.offset,.length]]"#,
            r"[[0,1],[1,1]]",
        ),
        (
            r#""ss" | [match("[ßx]"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""ssx" | [match("[à-ÿ]x"; "gi") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""\u00dfx" | [match("[\\w]x"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""ssss" | [match("ß{2}"; "gi") | [.offset,.length]]"#,
            r"[[0,4]]",
        ),
        (
            r#""ss\u00df" | [match("ß{2}"; "gi") | [.offset,.length]]"#,
            r"[[0,3]]",
        ),
    ])
}

/// Other entries of the tables: `ʼn`/`ŉ`, the Greek iota subscripts, the Latin ligatures `st`.
#[test]
fn other_folds_of_the_tables_6225() -> Result<()> {
    check(&[
        (
            r#""\u0149" | [match("ʼn"; "gi") | [.offset,.length]]"#,
            r"[[0,1]]",
        ),
        (
            r#""\u02bcn" | [match("ŉ"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""\u1fb3" | [match("αι"; "gi") | [.offset,.length]]"#,
            r"[[0,1]]",
        ),
        (
            r#""\u03b1\u03b9" | [match("ᾳ"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""\u0390" | [match("ΐ"; "gi") | [.offset,.length]]"#,
            r"[[0,1]]",
        ),
        (
            r#""\ufb05" | [match("st"; "gi") | [.offset,.length]]"#,
            r"[[0,1]]",
        ),
        (
            r#""\ufb06" | [match("ﬅ"; "gi") | [.offset,.length]]"#,
            r"[[0,1]]",
        ),
        (
            r#""st" | [match("ﬅ"; "gi") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
    ])
}

/// Inside a look-behind the library expands one character at a time, so none of this applies there, and the modifiers `x`, `l` and `n` leave the fold alone.
#[test]
fn a_look_behind_expands_one_character_at_a_time_6225() -> Result<()> {
    check(&[
        (
            r#""ssa" | [match("(?<=ß)a"; "gi") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""\u00dfa" | [match("(?<=ss)a"; "gi") | [.offset,.length]]"#,
            r"[]",
        ),
        (
            r#""ss\u00df" | [match("(?=ss)ß"; "gi") | [.offset,.length]]"#,
            r"[[0,2],[2,1]]",
        ),
        (
            r#""SS" | [match("ß"; "gix") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""SS" | [match("ß"; "il") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
        (
            r#""SS" | [match("ß"; "in") | [.offset,.length]]"#,
            r"[[0,2]]",
        ),
    ])
}
