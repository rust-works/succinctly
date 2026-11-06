//! #6427: a lazy `*?` or `+?` over a body with a group answers as Oniguruma does, and so does a loop with
//! groups inside one. An empty iteration leaves `*?`; `+?` over a short body is `X X*?`, the first copy with
//! no empty check. Every row was captured from jq 1.7.1 (`jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_lazy_star_or_plus_around_groups_answers_as_jq_does_6427() -> Result<()> {
    for (pattern, input, expected) in [
        (
            r"(?:(?:a|(b))+?(?:(a*)|b)+|b)+?",
            "bab",
            r#"[[0,"ba",["b",""]],[2,"b",["b",""]]]"#,
        ),
        (
            r"(?:(?:(b*)|(a))+?)(b)",
            "abab",
            r#"[[0,"ab",["","a","b"]],[2,"ab",["","a","b"]]]"#,
        ),
        (
            r"(?:(b*)|(a))+?(b)",
            "abab",
            r#"[[0,"ab",["","a","b"]],[2,"ab",["","a","b"]]]"#,
        ),
        (r"(?:(b*)|a)*?b", "aab", r#"[[0,"aab",[null]]]"#),
        (
            r"(?:x*|(?:b*|(a))+(?:(b)*|a)+)+?b",
            "abab",
            r#"[[0,"abab",[null,"b"]]]"#,
        ),
        (r"(?:(?:(b*)|a){0,2}?)*?c", "aac", r#"[[0,"aac",[null]]]"#),
        (
            r"(?:(?:(b*)|a){0,2}?|c){0,2}?d",
            "acd",
            r#"[[0,"acd",[null]]]"#,
        ),
        (
            r"(?:((b|)*|(a)(?:(a*)|b)+){0,2}?)(b)",
            "ab",
            r#"[[0,"ab",["a",null,"a","","b"]]]"#,
        ),
        (
            r"(?:(?:(b*)|a){1,2}?|c)+?d",
            "cacd",
            r#"[[0,"cacd",[null]]]"#,
        ),
        (r"(?:(b*)|aaaaaaaaaaaaaa)+?c", "aac", r#"[[2,"c",[""]]]"#),
        (
            r"((b?|a*)+?)+",
            "ba",
            r#"[[0,"b",["",""]],[1,"",["",""]],[2,"",["",""]]]"#,
        ),
        (
            r"(x?(?:b?|(a|b))+?)+",
            "ba",
            r#"[[0,"b",["",null]],[1,"",["",""]],[2,"",["",""]]]"#,
        ),
        (
            r"(?:(a)|b*|c)+(?:(?:(x)|y*|z){200}|d)*?",
            "bcab",
            r#"[[0,"b",[null,null]],[1,"",["",""]],[2,"ab",["a",null]],[4,"",["",""]]]"#,
        ),
        (
            r"(?:(a)|b*|c)+(?:(?:(x)|y*|z){200}|d)+?",
            "bcab",
            r#"[[0,"b",[null,null]],[1,"",["",""]],[2,"ab",["a",null]],[4,"",["",""]]]"#,
        ),
        (
            r"(?:^b*|(?:(a*)|b)+(?:(a)|b?){0,2})+?$",
            "abba",
            r#"[[0,"abba",["",null]],[4,"",["",""]]]"#,
        ),
    ] {
        let (stdout, _, status) = run_jq_full(
            &[
                "-nc",
                "--arg",
                "s",
                input,
                "--arg",
                "p",
                pattern,
                r#"[$s | match($p; "g") | [.offset, .string, (.captures | map(.string))]]"#,
            ],
            None,
        )?;
        assert_eq!(
            (stdout.trim_end(), status),
            (expected, 0),
            "{pattern} on {input:?}"
        );
    }
    Ok(())
}
