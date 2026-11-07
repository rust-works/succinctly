//! #6518: what is left of the capture-loop family after #6358: a lazy bounded repeat whose body has an anchor in
//! a way beside a group.

use anyhow::Result;

use crate::run_jq_full;

/// #6518: a lazy `{0,n}?` or `{1,n}?` over a body with several exits (an anchor's way and a way that matches
/// nothing anywhere), with a way that consumes after the first. Oniguruma tries the repeat's exit before an
/// iteration, so an exit's iteration only ends the loop and its group stays unset; the crate went on from it
/// into the consuming way with the group kept. Every row was captured from jq 1.7.1 (`jq -nc`).
#[test]
fn a_lazy_short_repeat_over_several_exits_answers_as_jq_does_6518() -> Result<()> {
    for (pattern, rows) in [
        (
            r"((\b)|(\w*)){0,4}?$",
            &[
                ("a b", r#"[[2,"b",["b",null,"b"]],[3,"",["","",""]]]"#),
                ("ba", r#"[[0,"ba",["ba",null,"ba"]],[2,"",["","",""]]]"#),
                ("bab", r#"[[0,"bab",["bab",null,"bab"]],[3,"",["","",""]]]"#),
                (
                    "abab",
                    r#"[[0,"abab",["abab",null,"abab"]],[4,"",["","",""]]]"#,
                ),
                ("aab", r#"[[0,"aab",["aab",null,"aab"]],[3,"",["","",""]]]"#),
                ("abc", r#"[[0,"abc",["abc",null,"abc"]],[3,"",["","",""]]]"#),
                ("c", r#"[[0,"c",["c",null,"c"]],[1,"",["","",""]]]"#),
                ("b c", r#"[[2,"c",["c",null,"c"]],[3,"",["","",""]]]"#),
                ("ac ", r#"[[3,"",["","",""]]]"#),
            ][..],
        ),
        (
            r"(?:(\b)|(?:(a)|b)*){0,3}?c",
            &[
                ("a b", "[]"),
                ("ba", "[]"),
                ("bab", "[]"),
                ("abab", "[]"),
                ("aab", "[]"),
                ("abc", r#"[[0,"abc",[null,"a"]]]"#),
                ("c", r#"[[0,"c",[null,null]]]"#),
                ("b c", r#"[[2,"c",[null,null]]]"#),
                ("ac ", r#"[[0,"ac",[null,"a"]]]"#),
            ][..],
        ),
        (
            r"(?:(\b)|a*|(?:(a)|b)){1,2}?$",
            &[
                ("a b", r#"[[2,"b",[null,null]],[3,"",["",""]]]"#),
                ("ba", r#"[[0,"ba",[null,null]],[2,"",["",""]]]"#),
                ("bab", r#"[[1,"ab",[null,null]],[3,"",["",""]]]"#),
                ("abab", r#"[[2,"ab",[null,null]],[4,"",["",""]]]"#),
                ("aab", r#"[[0,"aab",[null,null]],[3,"",["",""]]]"#),
                ("abc", r#"[[3,"",["",""]]]"#),
                ("c", r#"[[1,"",["",""]]]"#),
                ("b c", r#"[[3,"",["",""]]]"#),
                ("ac ", r#"[[3,"",["",""]]]"#),
            ][..],
        ),
        (
            r"(\A(\b)|(a)?(.*?)){0,2}?c",
            &[
                ("a b", "[]"),
                ("ba", "[]"),
                ("bab", "[]"),
                ("abab", "[]"),
                ("aab", "[]"),
                ("abc", r#"[[0,"abc",["b",null,"a","b"]]]"#),
                ("c", r#"[[0,"c",[null,null,null,null]]]"#),
                ("b c", r#"[[0,"b c",[" ",null,null," "]]]"#),
                ("ac ", r#"[[0,"ac",["a",null,"a",""]]]"#),
            ][..],
        ),
        (
            r"(\b(b|)*|(\z)|(?:b*|(a))+){1,3}?c",
            &[
                ("a b", "[]"),
                ("ba", "[]"),
                ("bab", "[]"),
                ("abab", "[]"),
                ("aab", "[]"),
                ("abc", r#"[[0,"abc",["ab",null,null,"a"]]]"#),
                ("c", r#"[[0,"c",["","",null,null]]]"#),
                ("b c", r#"[[2,"c",["","",null,null]]]"#),
                ("ac ", r#"[[0,"ac",["a",null,null,"a"]]]"#),
            ][..],
        ),
        (
            r"(?:(?:b*|(a))+(\b)|(.*?)){0,2}?c",
            &[
                ("a b", "[]"),
                ("ba", "[]"),
                ("bab", "[]"),
                ("abab", "[]"),
                ("aab", "[]"),
                ("abc", r#"[[0,"abc",[null,null,"b"]]]"#),
                ("c", r#"[[0,"c",[null,null,null]]]"#),
                ("b c", r#"[[0,"b c",[null,""," "]]]"#),
                ("ac ", r#"[[0,"ac",[null,null,"a"]]]"#),
            ][..],
        ),
    ] {
        for &(input, expected) in rows {
            assert_matches(pattern, input, expected)?;
        }
    }
    Ok(())
}

/// The loop pin of #6427 for `{1,2}?`: a body with loops of its own is written out since #6518, and answers as jq does.
#[test]
fn a_lazy_short_repeat_over_loops_and_exits_answers_as_jq_does_6518() -> Result<()> {
    for (pattern, rows) in [(
        r"(?:^b*|(?:(a*)|b)+(?:(a)|b?){0,2}){1,2}?$",
        &[
            ("abba", r#"[[0,"abba",["",null]],[4,"",["",""]]]"#),
            ("ba", r#"[[0,"ba",["",null]],[2,"",["",""]]]"#),
            ("bab", r#"[[0,"bab",["",null]],[3,"",["",""]]]"#),
            ("aab", r#"[[0,"aab",["",null]],[3,"",["",""]]]"#),
            ("abab", r#"[[0,"abab",["","a"]],[4,"",["",""]]]"#),
            ("a", r#"[[0,"a",["",null]],[1,"",["",""]]]"#),
            ("b", r#"[[0,"b",[null,null]],[1,"",["",""]]]"#),
            ("", r#"[[0,"",["",""]]]"#),
            ("ab", r#"[[0,"ab",["",null]],[2,"",["",""]]]"#),
        ][..],
    )] {
        for &(input, expected) in rows {
            assert_matches(pattern, input, expected)?;
        }
    }
    Ok(())
}

fn assert_matches(pattern: &str, input: &str, expected: &str) -> Result<()> {
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
    Ok(())
}
