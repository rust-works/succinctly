//! #6428: Oniguruma counts a lazy bounded repeat of two or more, and an iteration that matches nothing
//! leaves the count, so a group the empty way set is unset again when what follows fails there. Every row
//! was captured from jq 1.7.1 (`jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_lazy_bounded_repeat_unsets_the_group_of_an_empty_iteration_that_failed_6428() -> Result<()> {
    answers_as_jq_does(&[
        (r"(?:(b*)|a){0,2}?b", "ab", r#"[[0,"ab",[null]]]"#),
        (
            r"(?:(b*)|a){0,2}?b",
            "bab",
            r#"[[0,"b",[null]],[1,"ab",[null]]]"#,
        ),
        (r"(?:(b*)|a){1,2}?b", "ab", r#"[[0,"ab",[null]]]"#),
        (r"(?:(b*)|a){1,3}?b", "aab", r#"[[0,"aab",[null]]]"#),
        (r"(?:(b*)|a){0,3}?b", "aab", r#"[[0,"aab",[null]]]"#),
        (r"((b*)|a){0,2}?b", "ab", r#"[[0,"ab",["a",null]]]"#),
        (r"(?:a|(b*)|c){0,2}?d", "acd", r#"[[0,"acd",[null]]]"#),
        (r"(?:(a)|(b*)|c){1,3}?b", "cab", r#"[[0,"cab",["a",null]]]"#),
        (
            r"(?:(b*)|a){0,2}?",
            "ab",
            r#"[[0,"",[""]],[1,"",[""]],[2,"",[""]]]"#,
        ),
        (r"(?:(?:b*|(a))+){0,2}?b", "ab", r#"[[0,"ab",["a"]]]"#),
        (r"(?:(b*)|a){0,1}?b", "ab", r#"[[0,"ab",[null]]]"#),
    ])
}

#[test]
fn a_lazy_bounded_repeat_leaves_the_rest_of_the_pattern_rewritten_and_any_count_counted_6428(
) -> Result<()> {
    answers_as_jq_does(&[
        (
            r"(?:^b*|(?:(a*)|b)+(?:(a)|b?){0,2}){1,2}?$",
            "abba",
            r#"[[0,"abba",["",null]],[4,"",["",""]]]"#,
        ),
        (
            r"(?:(a)|b*|c)+(?:(d*)|x){0,20}?",
            "bcab",
            r#"[[0,"b",[null,null]],[1,"",["",""]],[2,"ab",["a",null]],[4,"",["",""]]]"#,
        ),
        (
            r"(?:(a)|b*|c)+(?:()|d){0,2}?",
            "bcab",
            r#"[[0,"b",[null,null]],[1,"",["",""]],[2,"ab",["a",null]],[4,"",["",""]]]"#,
        ),
        (r"(?:()|b){0,2}?a", "ba", r#"[[0,"ba",[null]]]"#),
        (r"(?:(b*)|a){0,11}?c", "aac", r#"[[0,"aac",[null]]]"#),
        (r"(?:(b*)|a){0,30}?b", "aab", r#"[[0,"aab",[null]]]"#),
        (r"(?:(?:(b*)|a){0,2}?|c)+d", "acd", r#"[[0,"acd",[null]]]"#),
        (
            r"((a)*|c)+(((b*())(()|a){0,2}?(())){4}){3}",
            "aac",
            r#"[[0,"aa",["","a","","","","",null,null,"",""]],[2,"",["","","","","","","","","",""]],[3,"",["","","","","","","","","",""]]]"#,
        ),
        (
            r"(?:(a)|b*|c)+(?:(?:(x)|y*|z){200}|d){0,2}?",
            "bcab",
            r#"[[0,"b",[null,null]],[1,"",["",""]],[2,"ab",["a",null]],[4,"",["",""]]]"#,
        ),
        (
            r"(?:(a)?|(b|)*(\b)){0,12}?$",
            "bb",
            r#"[[0,"bb",[null,"",""]],[2,"",["","",""]]]"#,
        ),
        (r"(?:b*(\b)){1,3}?$", "bb", r#"[[0,"bb",[""]],[2,"",[""]]]"#),
        (r"(\b|(b*)|a){1,2}?b", "ab", r#"[[0,"ab",["a",null]]]"#),
        (
            r"(?:(a)|b*|c)+(?:(d*)|x){1,20000}?",
            "bcab",
            r#"[[0,"b",[null,""]],[1,"",["",""]],[2,"ab",["a",""]],[4,"",["",""]]]"#,
        ),
    ])
}

fn answers_as_jq_does(rows: &[(&str, &str, &str)]) -> Result<()> {
    for (pattern, input, expected) in rows {
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
            (*expected, 0),
            "{pattern} on {input:?}"
        );
    }
    Ok(())
}
