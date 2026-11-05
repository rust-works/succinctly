//! #6262: a backreference inside the group it refers to, with that group under a repeat that can run
//! twice (`(\\1|\\w)+`). Oniguruma reads it as the group's last completed value; `fancy-regex` (the
//! second engine, taken for a backreference) reads from the group's new start to its old end, which
//! is empty for contiguous passes, so `"ab" | match("(\\1|\\w)+")` split into `a` and `b` where jq has
//! one match and the group at `[1,"b"]`. The shape is refused on that engine (`invalid regex`), as
//! #6096's is. A reference to a group that is open under no such repeat, or to one that has closed,
//! keeps its route and answers as `/usr/bin/jq` 1.7.1 does for `match($p; $f)`.

use anyhow::Result;

use crate::run_jq_full;

fn match_on(text: &str, pattern: &str, flags: &str) -> Result<String> {
    let program = r#"$s | try ([match($p; $f) | [.offset, .string, (.captures[] | [.offset, .string])]]) catch "ERR""#;
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-nc", "--arg", "s", text, "--arg", "p", pattern, "--arg", "f", flags, program,
        ],
        None,
    )?;
    assert_eq!(code, 0, "{text:?} ~ {pattern:?} [{flags}]: {stderr}");
    Ok(stdout.trim().to_string())
}

/// The group is under a repeat that can run twice, so the reference may read a previous pass.
/// Refused whatever the text: the engine answers some texts as jq does and others not, and a
/// pattern cannot tell which.
#[test]
fn a_reference_to_an_open_group_of_a_repeat_is_refused() -> Result<()> {
    const SHAPES: &[&str] = &[
        r"(\1|\w)+",
        r"(a|\1)+",
        r"(\w|\1)*",
        r"(a|\1){2}",
        r"(a|\1){1,2}",
        r"(a|\1){2,}",
        r"(a|\1)+?",
        r"(a|\1)*?",
        r"((?!\1)){2}",
        r"((?!\1)){2,}",
        r"(a(?=\1)|b)+",
        r"(a(?!\1))+",
        r"(a\1)+",
        // the group sits in an outer repeat, or the reference in a lookaround or an atomic group
        r"(?:(a|\1)b)+",
        r"(?:(?:(a|\1))|x)+",
        r"(?:(a(?>\1)?))+",
        // a named reference is the same reference
        r"(?<n>a|\k<n>)+",
        // the open group is not the first: only its own number counts
        r"(x)(a|\2)+",
        r"((b)|a(?:\1|c))+",
    ];
    for pattern in SHAPES {
        for flags in ["", "g"] {
            for text in ["", "a", "ab", "aab", "abab"] {
                assert_eq!(
                    match_on(text, pattern, flags)?,
                    r#""ERR""#,
                    "{text:?} ~ {pattern:?} [{flags}]"
                );
            }
        }
    }
    Ok(())
}

/// What stays as it was: the group is under no repeat that can run twice (the reference then reads
/// a group that never completed, unset in both engines), or the group it refers to has closed.
#[test]
fn a_reference_that_reads_an_unset_or_a_closed_group_keeps_its_route() -> Result<()> {
    const ROWS: &[(&str, &str, &str, &str)] = &[
        (r"ab", r"(a|\1)", r"", r#"[[0,"a",[0,"a"]]]"#),
        (
            r"ab",
            r"(a|\1)?",
            r"g",
            r#"[[0,"a",[0,"a"]],[1,"",[1,""]],[2,"",[2,""]]]"#,
        ),
        (r"ab", r"(a|\1){1}", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(a|\1){0,1}", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(a|\1){0}", r"", r#"[[0,"",[0,""]]]"#),
        (r"ab", r"(a(?=\1))", r"", r"[]"),
        (r"ab", r"(a)(?=\1)", r"", r"[]"),
        (r"ab", r"(a)\1+", r"", r"[]"),
        (r"aab", r"(a)\1+", r"", r#"[[0,"aa",[0,"a"]]]"#),
        (r"aa", r"(a)(\1)+", r"", r#"[[0,"aa",[0,"a"],[1,"a"]]]"#),
        (r"aa", r"(a)\1", r"", r#"[[0,"aa",[0,"a"]]]"#),
        (r"abab", r"(ab)\1", r"", r#"[[0,"abab",[0,"ab"]]]"#),
        (r"aa", r"(?:(a)|b\1)+", r"", r#"[[0,"aa",[1,"a"]]]"#),
        (r"ab", r"((a)|b\2)+", r"g", r#"[[0,"a",[0,"a"],[0,"a"]]]"#),
        (r"aa", r"(a)(?:\1)*", r"", r#"[[0,"aa",[0,"a"]]]"#),
    ];
    for &(text, pattern, flags, expected) in ROWS {
        assert_eq!(
            match_on(text, pattern, flags)?,
            expected,
            "{text:?} ~ {pattern:?} [{flags}]"
        );
    }
    Ok(())
}

/// The refusal needs the reference: the same repeated group without one is on the `regex` crate's
/// route and answers as jq does.
#[test]
fn a_repeated_group_without_a_reference_is_unaffected() -> Result<()> {
    assert_eq!(
        match_on("ab", r"(a|\w)+", "")?,
        r#"[[0,"ab",[1,"b"]]]"#,
        "a repeated group with no reference takes the regex route"
    );
    Ok(())
}
