//! #6128 (item 3): a repeat operator after a comment repeats the item before the comment in
//! Oniguruma, as if the comment were not there: `b(?#)*` is `b*`. The `regex` crate and
//! `fancy-regex` repeat the comment instead, which has no empty way of its own, so a loop around
//! it (`(?:b(?#)*|a)+`) lacked the empty iteration `(?:b*|a)+` has and matched `"bab"` whole where
//! jq 1.7.1 answers `b`, `""`, `b`, `""`. The comment is left out of what the engines read when a
//! repeat operator follows it and an item precedes it. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -n`).

use anyhow::Result;

use crate::run_jq_full;

fn answer(pattern: &str, input: &str, flags: &str) -> Result<String> {
    let program = format!(
        r#""{input}" | [match("{}";"{flags}") | [.offset, .length, .string]]"#,
        pattern.replace('\\', "\\\\")
    );
    let (stdout, stderr, code) = run_jq_full(&["-nc", &program], None)?;
    assert_eq!(code, 0, "{pattern} on {input}: stderr={stderr:?}");
    Ok(stdout.trim_end().to_string())
}

#[test]
fn test_a_repeat_after_a_comment_repeats_the_item_before_it_6128() -> Result<()> {
    for (pattern, input, flags, expected) in [
        (
            r"(?:b(?#)*|a)+",
            "bab",
            "g",
            r#"[[0,1,"b"],[1,0,""],[2,1,"b"],[3,0,""]]"#,
        ),
        (
            r"(?:b(?#)*|a)+",
            "ba",
            "g",
            r#"[[0,1,"b"],[1,0,""],[2,0,""]]"#,
        ),
        (r"(?:b(?#)*|a)+", "bba", "", r#"[[0,2,"bb"]]"#),
        (
            r"(?:b(?#c)*|a)*",
            "bab",
            "g",
            r#"[[0,1,"b"],[1,0,""],[2,1,"b"],[3,0,""]]"#,
        ),
        (
            r"(?:b(?#)?|a)+",
            "bab",
            "g",
            r#"[[0,1,"b"],[1,0,""],[2,1,"b"],[3,0,""]]"#,
        ),
        (
            r"(?:b(?#)?|a)+",
            "ba",
            "g",
            r#"[[0,1,"b"],[1,0,""],[2,0,""]]"#,
        ),
        (r"(?:b(?#){2}|a)+", "bab", "g", r#"[[1,1,"a"]]"#),
        (r"(?:b(?#){1,2}|a)+", "bab", "g", r#"[[0,3,"bab"]]"#),
        (r"(?:a|b(?#)*)+", "bab", "g", r#"[[0,3,"bab"],[3,0,""]]"#),
        (
            r"(?:b(?#)*|a)*",
            "cbb",
            "g",
            r#"[[0,0,""],[1,2,"bb"],[3,0,""]]"#,
        ),
        (
            r"(?:a|b(?#)*?)+",
            "bab",
            "g",
            r#"[[0,0,""],[1,1,"a"],[2,0,""],[3,0,""]]"#,
        ),
        (
            r"(?:b(?#)(?#)*|a)+",
            "bab",
            "g",
            r#"[[0,1,"b"],[1,0,""],[2,1,"b"],[3,0,""]]"#,
        ),
        (
            r"(?:[b](?#)*|a)+",
            "bab",
            "g",
            r#"[[0,1,"b"],[1,0,""],[2,1,"b"],[3,0,""]]"#,
        ),
        (r"(?:\w(?#)*|a)+", "bab", "g", r#"[[0,3,"bab"],[3,0,""]]"#),
        (
            r"(?:(?:b|c)(?#)*|a)+",
            "bcb",
            "g",
            r#"[[0,3,"bcb"],[3,0,""]]"#,
        ),
        (
            r"(?:\*(?#)*|a)+",
            "*a*",
            "g",
            r#"[[0,1,"*"],[1,0,""],[2,1,"*"],[3,0,""]]"#,
        ),
        (
            r"(?:\?(?#)*|a)+",
            "a*?",
            "g",
            r#"[[0,0,""],[1,0,""],[2,1,"?"],[3,0,""]]"#,
        ),
        (
            r"(?:\\(?#)*|a)+",
            "a*?",
            "g",
            r#"[[0,0,""],[1,0,""],[2,0,""],[3,0,""]]"#,
        ),
        (r"(?:b(?#)+|a)+", "bab", "g", r#"[[0,3,"bab"]]"#),
        (r"(?:b(?#)*|a)+c", "bab", "g", r"[]"),
    ] {
        assert_eq!(
            answer(pattern, input, flags)?,
            expected,
            "{pattern} on {input}"
        );
    }
    Ok(())
}

/// A comment after a quantifier followed by an interval or a lazy repeat is kept: `b*(?#){2}` is a
/// repeat of a repeat in jq and `b*(?#)*?` a lazy repeat of `b*`, which the engines read another
/// way inside a loop (`"bab"` whole, where jq answers `b`, `""`, `b`, `""`). An unfixed gap,
/// recorded in `docs/compliance/jq/limitations.md`; the greedy `*`, `+` and `?` after a comment
/// are merged since #6303 (`tests/jq_cli/i6303.rs`).
#[test]
fn test_a_comment_after_a_quantifier_is_kept_6128() -> Result<()> {
    for (pattern, input, flags, jq_answer) in [
        (
            r"(?:b*(?#){2}|a)+",
            "bab",
            "g",
            r#"[[0,1,"b"],[1,0,""],[2,1,"b"],[3,0,""]]"#,
        ),
        (
            r"(?:a|b*(?#)*?)+",
            "bab",
            "g",
            r#"[[0,0,""],[1,1,"a"],[2,0,""],[3,0,""]]"#,
        ),
    ] {
        let ours = answer(pattern, input, flags)?;
        assert_ne!(
            ours, jq_answer,
            "{pattern} on {input}: the gap closed, move the row up"
        );
    }
    Ok(())
}
