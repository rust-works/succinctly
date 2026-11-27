//! #6262: a backreference inside the group it refers to, with that group under a repeat that can
//! run twice (`(\\1|\\w)+`). Oniguruma reads it as the group's last completed value; `fancy-regex`
//! (the second engine, taken for a backreference) reads from the group's new start to its old end,
//! which is empty for contiguous passes, so `"ab" | match("(\\1|\\w)+")` split into `a` and `b`
//! where jq has one match and the group at `[1,"b"]`. The shape was refused on that engine (`Regex
//! failure`), as #6096's was, a recorded divergence; on Oniguruma it answers as jq 1.7.1 does
//! (#7965). A reference to a group that is open under no such repeat, or to one that has closed,
//! answers as `/usr/bin/jq` 1.7.1 does for `match($p; $f)`.
//! Pin moved (#7520): a row whose empty overall match leaves a capture group unset is jq 1.8.2's answer
//! (`null`, offset -1), where jq 1.7.1 reported an empty string.

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

/// The group is under a repeat that can run twice, so the reference may read a previous pass: the
/// earlier engine refused it whatever the text, and each row is now jq's answer (#7965). Rows whose
/// empty overall match leaves the group unset are jq 1.8.2's (#7520).
#[test]
fn a_reference_to_an_open_group_of_a_repeat_answers_as_jq_does() -> Result<()> {
    const ROWS: &[(&str, &str, &str, &str)] = &[
        (r"", r"(\1|\w)+", r"", r"[]"),
        (r"a", r"(\1|\w)+", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(\1|\w)+", r"", r#"[[0,"ab",[1,"b"]]]"#),
        (r"aab", r"(\1|\w)+", r"", r#"[[0,"aab",[2,"b"]]]"#),
        (r"abab", r"(\1|\w)+", r"", r#"[[0,"abab",[3,"b"]]]"#),
        (r"", r"(\1|\w)+", r"g", r"[]"),
        (r"a", r"(\1|\w)+", r"g", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(\1|\w)+", r"g", r#"[[0,"ab",[1,"b"]]]"#),
        (r"aab", r"(\1|\w)+", r"g", r#"[[0,"aab",[2,"b"]]]"#),
        (r"abab", r"(\1|\w)+", r"g", r#"[[0,"abab",[3,"b"]]]"#),
        (r"", r"(a|\1)+", r"", r"[]"),
        (r"a", r"(a|\1)+", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(a|\1)+", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"aab", r"(a|\1)+", r"", r#"[[0,"aa",[1,"a"]]]"#),
        (r"abab", r"(a|\1)+", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"", r"(a|\1)+", r"g", r"[]"),
        (r"a", r"(a|\1)+", r"g", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(a|\1)+", r"g", r#"[[0,"a",[0,"a"]]]"#),
        (r"aab", r"(a|\1)+", r"g", r#"[[0,"aa",[1,"a"]]]"#),
        (
            r"abab",
            r"(a|\1)+",
            r"g",
            r#"[[0,"a",[0,"a"]],[2,"a",[2,"a"]]]"#,
        ),
        (r"", r"(\w|\1)*", r"", r#"[[0,"",[-1,null]]]"#),
        (r"a", r"(\w|\1)*", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(\w|\1)*", r"", r#"[[0,"ab",[1,"b"]]]"#),
        (r"aab", r"(\w|\1)*", r"", r#"[[0,"aab",[2,"b"]]]"#),
        (r"abab", r"(\w|\1)*", r"", r#"[[0,"abab",[3,"b"]]]"#),
        (r"", r"(\w|\1)*", r"g", r#"[[0,"",[-1,null]]]"#),
        (
            r"a",
            r"(\w|\1)*",
            r"g",
            r#"[[0,"a",[0,"a"]],[1,"",[-1,null]]]"#,
        ),
        (
            r"ab",
            r"(\w|\1)*",
            r"g",
            r#"[[0,"ab",[1,"b"]],[2,"",[-1,null]]]"#,
        ),
        (
            r"aab",
            r"(\w|\1)*",
            r"g",
            r#"[[0,"aab",[2,"b"]],[3,"",[-1,null]]]"#,
        ),
        (
            r"abab",
            r"(\w|\1)*",
            r"g",
            r#"[[0,"abab",[3,"b"]],[4,"",[-1,null]]]"#,
        ),
        (r"", r"(a|\1){2}", r"", r"[]"),
        (r"a", r"(a|\1){2}", r"", r"[]"),
        (r"ab", r"(a|\1){2}", r"", r"[]"),
        (r"aab", r"(a|\1){2}", r"", r#"[[0,"aa",[1,"a"]]]"#),
        (r"abab", r"(a|\1){2}", r"", r"[]"),
        (r"", r"(a|\1){2}", r"g", r"[]"),
        (r"a", r"(a|\1){2}", r"g", r"[]"),
        (r"ab", r"(a|\1){2}", r"g", r"[]"),
        (r"aab", r"(a|\1){2}", r"g", r#"[[0,"aa",[1,"a"]]]"#),
        (r"abab", r"(a|\1){2}", r"g", r"[]"),
        (r"", r"(a|\1){1,2}", r"", r"[]"),
        (r"a", r"(a|\1){1,2}", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(a|\1){1,2}", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"aab", r"(a|\1){1,2}", r"", r#"[[0,"aa",[1,"a"]]]"#),
        (r"abab", r"(a|\1){1,2}", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"", r"(a|\1){1,2}", r"g", r"[]"),
        (r"a", r"(a|\1){1,2}", r"g", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(a|\1){1,2}", r"g", r#"[[0,"a",[0,"a"]]]"#),
        (r"aab", r"(a|\1){1,2}", r"g", r#"[[0,"aa",[1,"a"]]]"#),
        (
            r"abab",
            r"(a|\1){1,2}",
            r"g",
            r#"[[0,"a",[0,"a"]],[2,"a",[2,"a"]]]"#,
        ),
        (r"", r"(a|\1){2,}", r"", r"[]"),
        (r"a", r"(a|\1){2,}", r"", r"[]"),
        (r"ab", r"(a|\1){2,}", r"", r"[]"),
        (r"aab", r"(a|\1){2,}", r"", r#"[[0,"aa",[1,"a"]]]"#),
        (r"abab", r"(a|\1){2,}", r"", r"[]"),
        (r"", r"(a|\1){2,}", r"g", r"[]"),
        (r"a", r"(a|\1){2,}", r"g", r"[]"),
        (r"ab", r"(a|\1){2,}", r"g", r"[]"),
        (r"aab", r"(a|\1){2,}", r"g", r#"[[0,"aa",[1,"a"]]]"#),
        (r"abab", r"(a|\1){2,}", r"g", r"[]"),
        (r"", r"(a|\1)+?", r"", r"[]"),
        (r"a", r"(a|\1)+?", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(a|\1)+?", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"aab", r"(a|\1)+?", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"abab", r"(a|\1)+?", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"", r"(a|\1)+?", r"g", r"[]"),
        (r"a", r"(a|\1)+?", r"g", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(a|\1)+?", r"g", r#"[[0,"a",[0,"a"]]]"#),
        (
            r"aab",
            r"(a|\1)+?",
            r"g",
            r#"[[0,"a",[0,"a"]],[1,"a",[1,"a"]]]"#,
        ),
        (
            r"abab",
            r"(a|\1)+?",
            r"g",
            r#"[[0,"a",[0,"a"]],[2,"a",[2,"a"]]]"#,
        ),
        (r"", r"(a|\1)*?", r"", r#"[[0,"",[-1,null]]]"#),
        (r"a", r"(a|\1)*?", r"", r#"[[0,"",[-1,null]]]"#),
        (r"ab", r"(a|\1)*?", r"", r#"[[0,"",[-1,null]]]"#),
        (r"aab", r"(a|\1)*?", r"", r#"[[0,"",[-1,null]]]"#),
        (r"abab", r"(a|\1)*?", r"", r#"[[0,"",[-1,null]]]"#),
        (r"", r"(a|\1)*?", r"g", r#"[[0,"",[-1,null]]]"#),
        (
            r"a",
            r"(a|\1)*?",
            r"g",
            r#"[[0,"",[-1,null]],[1,"",[-1,null]]]"#,
        ),
        (
            r"ab",
            r"(a|\1)*?",
            r"g",
            r#"[[0,"",[-1,null]],[1,"",[-1,null]],[2,"",[-1,null]]]"#,
        ),
        (
            r"aab",
            r"(a|\1)*?",
            r"g",
            r#"[[0,"",[-1,null]],[1,"",[-1,null]],[2,"",[-1,null]],[3,"",[-1,null]]]"#,
        ),
        (
            r"abab",
            r"(a|\1)*?",
            r"g",
            r#"[[0,"",[-1,null]],[1,"",[-1,null]],[2,"",[-1,null]],[3,"",[-1,null]],[4,"",[-1,null]]]"#,
        ),
        (r"", r"((?!\1)){2}", r"", r#"[[0,"",[0,""]]]"#),
        (r"a", r"((?!\1)){2}", r"", r#"[[0,"",[0,""]]]"#),
        (r"ab", r"((?!\1)){2}", r"", r#"[[0,"",[0,""]]]"#),
        (r"aab", r"((?!\1)){2}", r"", r#"[[0,"",[0,""]]]"#),
        (r"abab", r"((?!\1)){2}", r"", r#"[[0,"",[0,""]]]"#),
        (r"", r"((?!\1)){2}", r"g", r#"[[0,"",[0,""]]]"#),
        (
            r"a",
            r"((?!\1)){2}",
            r"g",
            r#"[[0,"",[0,""]],[1,"",[1,""]]]"#,
        ),
        (
            r"ab",
            r"((?!\1)){2}",
            r"g",
            r#"[[0,"",[0,""]],[1,"",[1,""]],[2,"",[2,""]]]"#,
        ),
        (
            r"aab",
            r"((?!\1)){2}",
            r"g",
            r#"[[0,"",[0,""]],[1,"",[1,""]],[2,"",[2,""]],[3,"",[3,""]]]"#,
        ),
        (
            r"abab",
            r"((?!\1)){2}",
            r"g",
            r#"[[0,"",[0,""]],[1,"",[1,""]],[2,"",[2,""]],[3,"",[3,""]],[4,"",[4,""]]]"#,
        ),
        (r"", r"((?!\1)){2,}", r"", r#"[[0,"",[0,""]]]"#),
        (r"a", r"((?!\1)){2,}", r"", r#"[[0,"",[0,""]]]"#),
        (r"ab", r"((?!\1)){2,}", r"", r#"[[0,"",[0,""]]]"#),
        (r"aab", r"((?!\1)){2,}", r"", r#"[[0,"",[0,""]]]"#),
        (r"abab", r"((?!\1)){2,}", r"", r#"[[0,"",[0,""]]]"#),
        (r"", r"((?!\1)){2,}", r"g", r#"[[0,"",[0,""]]]"#),
        (
            r"a",
            r"((?!\1)){2,}",
            r"g",
            r#"[[0,"",[0,""]],[1,"",[1,""]]]"#,
        ),
        (
            r"ab",
            r"((?!\1)){2,}",
            r"g",
            r#"[[0,"",[0,""]],[1,"",[1,""]],[2,"",[2,""]]]"#,
        ),
        (
            r"aab",
            r"((?!\1)){2,}",
            r"g",
            r#"[[0,"",[0,""]],[1,"",[1,""]],[2,"",[2,""]],[3,"",[3,""]]]"#,
        ),
        (
            r"abab",
            r"((?!\1)){2,}",
            r"g",
            r#"[[0,"",[0,""]],[1,"",[1,""]],[2,"",[2,""]],[3,"",[3,""]],[4,"",[4,""]]]"#,
        ),
        (r"", r"(a(?=\1)|b)+", r"", r"[]"),
        (r"a", r"(a(?=\1)|b)+", r"", r"[]"),
        (r"ab", r"(a(?=\1)|b)+", r"", r#"[[1,"b",[1,"b"]]]"#),
        (r"aab", r"(a(?=\1)|b)+", r"", r#"[[2,"b",[2,"b"]]]"#),
        (r"abab", r"(a(?=\1)|b)+", r"", r#"[[1,"b",[1,"b"]]]"#),
        (r"", r"(a(?=\1)|b)+", r"g", r"[]"),
        (r"a", r"(a(?=\1)|b)+", r"g", r"[]"),
        (r"ab", r"(a(?=\1)|b)+", r"g", r#"[[1,"b",[1,"b"]]]"#),
        (r"aab", r"(a(?=\1)|b)+", r"g", r#"[[2,"b",[2,"b"]]]"#),
        (
            r"abab",
            r"(a(?=\1)|b)+",
            r"g",
            r#"[[1,"b",[1,"b"]],[3,"b",[3,"b"]]]"#,
        ),
        (r"", r"(a(?!\1))+", r"", r"[]"),
        (r"a", r"(a(?!\1))+", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(a(?!\1))+", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"aab", r"(a(?!\1))+", r"", r#"[[0,"aa",[1,"a"]]]"#),
        (r"abab", r"(a(?!\1))+", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"", r"(a(?!\1))+", r"g", r"[]"),
        (r"a", r"(a(?!\1))+", r"g", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(a(?!\1))+", r"g", r#"[[0,"a",[0,"a"]]]"#),
        (r"aab", r"(a(?!\1))+", r"g", r#"[[0,"aa",[1,"a"]]]"#),
        (
            r"abab",
            r"(a(?!\1))+",
            r"g",
            r#"[[0,"a",[0,"a"]],[2,"a",[2,"a"]]]"#,
        ),
        (r"", r"(a\1)+", r"", r"[]"),
        (r"a", r"(a\1)+", r"", r"[]"),
        (r"ab", r"(a\1)+", r"", r"[]"),
        (r"aab", r"(a\1)+", r"", r"[]"),
        (r"abab", r"(a\1)+", r"", r"[]"),
        (r"", r"(a\1)+", r"g", r"[]"),
        (r"a", r"(a\1)+", r"g", r"[]"),
        (r"ab", r"(a\1)+", r"g", r"[]"),
        (r"aab", r"(a\1)+", r"g", r"[]"),
        (r"abab", r"(a\1)+", r"g", r"[]"),
        (r"", r"(?:(a|\1)b)+", r"", r"[]"),
        (r"a", r"(?:(a|\1)b)+", r"", r"[]"),
        (r"ab", r"(?:(a|\1)b)+", r"", r#"[[0,"ab",[0,"a"]]]"#),
        (r"aab", r"(?:(a|\1)b)+", r"", r#"[[1,"ab",[1,"a"]]]"#),
        (r"abab", r"(?:(a|\1)b)+", r"", r#"[[0,"abab",[2,"a"]]]"#),
        (r"", r"(?:(a|\1)b)+", r"g", r"[]"),
        (r"a", r"(?:(a|\1)b)+", r"g", r"[]"),
        (r"ab", r"(?:(a|\1)b)+", r"g", r#"[[0,"ab",[0,"a"]]]"#),
        (r"aab", r"(?:(a|\1)b)+", r"g", r#"[[1,"ab",[1,"a"]]]"#),
        (r"abab", r"(?:(a|\1)b)+", r"g", r#"[[0,"abab",[2,"a"]]]"#),
        (r"", r"(?:(?:(a|\1))|x)+", r"", r"[]"),
        (r"a", r"(?:(?:(a|\1))|x)+", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(?:(?:(a|\1))|x)+", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"aab", r"(?:(?:(a|\1))|x)+", r"", r#"[[0,"aa",[1,"a"]]]"#),
        (r"abab", r"(?:(?:(a|\1))|x)+", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"", r"(?:(?:(a|\1))|x)+", r"g", r"[]"),
        (r"a", r"(?:(?:(a|\1))|x)+", r"g", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(?:(?:(a|\1))|x)+", r"g", r#"[[0,"a",[0,"a"]]]"#),
        (r"aab", r"(?:(?:(a|\1))|x)+", r"g", r#"[[0,"aa",[1,"a"]]]"#),
        (
            r"abab",
            r"(?:(?:(a|\1))|x)+",
            r"g",
            r#"[[0,"a",[0,"a"]],[2,"a",[2,"a"]]]"#,
        ),
        (r"", r"(?:(a(?>\1)?))+", r"", r"[]"),
        (r"a", r"(?:(a(?>\1)?))+", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(?:(a(?>\1)?))+", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"aab", r"(?:(a(?>\1)?))+", r"", r#"[[0,"aa",[1,"a"]]]"#),
        (r"abab", r"(?:(a(?>\1)?))+", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"", r"(?:(a(?>\1)?))+", r"g", r"[]"),
        (r"a", r"(?:(a(?>\1)?))+", r"g", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(?:(a(?>\1)?))+", r"g", r#"[[0,"a",[0,"a"]]]"#),
        (r"aab", r"(?:(a(?>\1)?))+", r"g", r#"[[0,"aa",[1,"a"]]]"#),
        (
            r"abab",
            r"(?:(a(?>\1)?))+",
            r"g",
            r#"[[0,"a",[0,"a"]],[2,"a",[2,"a"]]]"#,
        ),
        (r"", r"(?<n>a|\k<n>)+", r"", r"[]"),
        (r"a", r"(?<n>a|\k<n>)+", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(?<n>a|\k<n>)+", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"aab", r"(?<n>a|\k<n>)+", r"", r#"[[0,"aa",[1,"a"]]]"#),
        (r"abab", r"(?<n>a|\k<n>)+", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"", r"(?<n>a|\k<n>)+", r"g", r"[]"),
        (r"a", r"(?<n>a|\k<n>)+", r"g", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(?<n>a|\k<n>)+", r"g", r#"[[0,"a",[0,"a"]]]"#),
        (r"aab", r"(?<n>a|\k<n>)+", r"g", r#"[[0,"aa",[1,"a"]]]"#),
        (
            r"abab",
            r"(?<n>a|\k<n>)+",
            r"g",
            r#"[[0,"a",[0,"a"]],[2,"a",[2,"a"]]]"#,
        ),
        (r"", r"(x)(a|\2)+", r"", r"[]"),
        (r"a", r"(x)(a|\2)+", r"", r"[]"),
        (r"ab", r"(x)(a|\2)+", r"", r"[]"),
        (r"aab", r"(x)(a|\2)+", r"", r"[]"),
        (r"abab", r"(x)(a|\2)+", r"", r"[]"),
        (r"", r"(x)(a|\2)+", r"g", r"[]"),
        (r"a", r"(x)(a|\2)+", r"g", r"[]"),
        (r"ab", r"(x)(a|\2)+", r"g", r"[]"),
        (r"aab", r"(x)(a|\2)+", r"g", r"[]"),
        (r"abab", r"(x)(a|\2)+", r"g", r"[]"),
        (r"", r"(x)(?<n>a|\k<n>)+", r"", r"[]"),
        (r"a", r"(x)(?<n>a|\k<n>)+", r"", r"[]"),
        (r"ab", r"(x)(?<n>a|\k<n>)+", r"", r"[]"),
        (r"aab", r"(x)(?<n>a|\k<n>)+", r"", r"[]"),
        (r"abab", r"(x)(?<n>a|\k<n>)+", r"", r"[]"),
        (r"", r"(x)(?<n>a|\k<n>)+", r"g", r"[]"),
        (r"a", r"(x)(?<n>a|\k<n>)+", r"g", r"[]"),
        (r"ab", r"(x)(?<n>a|\k<n>)+", r"g", r"[]"),
        (r"aab", r"(x)(?<n>a|\k<n>)+", r"g", r"[]"),
        (r"abab", r"(x)(?<n>a|\k<n>)+", r"g", r"[]"),
        (r"", r"((b)|a(?:\1|c))+", r"", r"[]"),
        (r"a", r"((b)|a(?:\1|c))+", r"", r"[]"),
        (
            r"ab",
            r"((b)|a(?:\1|c))+",
            r"",
            r#"[[1,"b",[1,"b"],[1,"b"]]]"#,
        ),
        (
            r"aab",
            r"((b)|a(?:\1|c))+",
            r"",
            r#"[[2,"b",[2,"b"],[2,"b"]]]"#,
        ),
        (
            r"abab",
            r"((b)|a(?:\1|c))+",
            r"",
            r#"[[1,"b",[1,"b"],[1,"b"]]]"#,
        ),
        (r"", r"((b)|a(?:\1|c))+", r"g", r"[]"),
        (r"a", r"((b)|a(?:\1|c))+", r"g", r"[]"),
        (
            r"ab",
            r"((b)|a(?:\1|c))+",
            r"g",
            r#"[[1,"b",[1,"b"],[1,"b"]]]"#,
        ),
        (
            r"aab",
            r"((b)|a(?:\1|c))+",
            r"g",
            r#"[[2,"b",[2,"b"],[2,"b"]]]"#,
        ),
        (
            r"abab",
            r"((b)|a(?:\1|c))+",
            r"g",
            r#"[[1,"b",[1,"b"],[1,"b"]],[3,"b",[3,"b"],[3,"b"]]]"#,
        ),
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
            r#"[[0,"a",[0,"a"]],[1,"",[-1,null]],[2,"",[-1,null]]]"#,
        ),
        (r"ab", r"(a|\1){1}", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(a|\1){0,1}", r"", r#"[[0,"a",[0,"a"]]]"#),
        (r"ab", r"(a|\1){0}", r"", r#"[[0,"",[-1,null]]]"#),
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

/// Every regex builtin builds its pattern the same way. The earlier engine refused the shape in
/// each of them; they now answer as jq 1.7.1 does on `"xab"` (#7965).
#[test]
fn every_regex_builtin_answers_the_shape_as_jq_does() -> Result<()> {
    const PROGRAMS: &[(&str, &str)] = &[
        (r#"test("(\\1|\\w)+")"#, "true"),
        (r#"test("(\\1|\\w)+"; "i")"#, "true"),
        (r#"[scan("(\\1|\\w)+")]"#, r#"[["b"]]"#),
        (r#"sub("(\\1|\\w)+"; "X")"#, r#""X""#),
        (r#"gsub("(a|\\1)+"; "X")"#, r#""xXb""#),
        (r#"[splits("(a|\\1)+")]"#, r#"["x","b"]"#),
        (r#"capture("(?<n>a|\\k<n>)+")"#, r#"{"n":"a"}"#),
    ];
    for &(program, expected) in PROGRAMS {
        let filter = format!(r#""xab" | try ({program}) catch "ERR""#);
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter], None)?;
        assert_eq!(code, 0, "{program}: {stderr}");
        assert_eq!(stdout.trim(), expected, "{program}");
    }
    Ok(())
}
