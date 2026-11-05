//! #6305: a loop whose repeated item is a capture group ends at the body's empty way, as Oniguruma's
//! does (`"ba" | match("(b*|a)+")` is `"b"`), where the `regex` crate matched `"ba"`. The capture
//! rewrite of #6069 leaves the repeated group's own loop as a plain `(?:X)*`, and the loop rewrite of
//! #6106 reads that. Every row was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

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
fn test_a_repeated_capture_group_ends_at_its_empty_way_6305() -> Result<()> {
    assert_rows(&[
        (r#""ba" | [match("(b*|a)+") | .string]"#, r#"["b"]"#),
        (
            r#""ba" | [match("(b*|a)+") | .captures | map([.offset,.length,.string])]"#,
            "[[[1,0,\"\"]]]",
        ),
        (
            r#""bab" | [match("(b*|a)+";"g") | .string]"#,
            r#"["b","","b",""]"#,
        ),
        (
            r#""bab" | [match("(b*|a)*";"g") | [.string, (.captures|map(.string))]]"#,
            r#"[["b",[""]],["",[""]],["b",[""]],["",[""]]]"#,
        ),
        (
            r#""ba" | [match("(?<x>b*|a)+") | [.string, (.captures|map([.name,.string]))]]"#,
            r#"[["b",[["x",""]]]]"#,
        ),
        (r#""ba" | capture("(?<x>b*|a)+")"#, r#"{"x":""}"#),
        (
            r#""xba" | [match("x(b*|a)+") | [.string, (.captures|map(.string))]]"#,
            r#"[["xb",[""]]]"#,
        ),
    ])
}

#[test]
fn test_the_builtins_over_a_repeated_capture_group_agree_with_jq_6305() -> Result<()> {
    assert_rows(&[
        (r#""ba" | sub("(b*|a)+"; "X")"#, r#""Xa""#),
        (
            r#""foo-bar" | [match("(\\w*|-)+") | .string]"#,
            r#"["foo"]"#,
        ),
        (r#""foo-bar" | gsub("(\\w*|-)+"; "X")"#, r#""XX-XX""#),
        (r#""ba" | [scan("(b*|a)+")]"#, r#"[[""],[""],[""]]"#),
        (
            r#""BA" | [match("(b*|a)+";"gi") | .string]"#,
            r#"["B","",""]"#,
        ),
        (r#""ba" | test("(b*|a)+a")"#, "true"),
    ])
}

/// What already agreed keeps agreeing: a group whose body has no later way, and one whose empty
/// iteration is the capture.
#[test]
fn test_repeated_capture_groups_that_already_agreed_still_do_6305() -> Result<()> {
    assert_rows(&[
        (
            r#""aab" | [match("(a*|b)*ab";"g") | [.string, (.captures|map(.string))]]"#,
            r#"[["aab",[""]]]"#,
        ),
        (
            r#""aab" | match("(a*)*ab") | [.string, (.captures|map([.offset,.length,.string]))]"#,
            r#"["aab",[[1,0,""]]]"#,
        ),
    ])
}

/// The other spellings of the unbounded repeat, a lazy one, a named group, `l`, and a pattern that
/// holds a plain loop (rewritten by #6106 before the capture rewrite sees it) beside the capture
/// loop: the second pass leaves the first one's output alone.
#[test]
fn test_other_repeat_spellings_and_a_plain_loop_beside_it_6305() -> Result<()> {
    assert_rows(&[
        (
            r#""bacd" | [match("(?:b*|a)+(c*|d)*";"g") | .string]"#,
            r#"["b","","c","",""]"#,
        ),
        (
            r#""bacd" | [match("(b*|a)+(c*|d)*";"g") | [.string, (.captures|map(.string))]]"#,
            r#"[["b",["",""]],["",["",""]],["c",["",""]],["",["",""]],["",["",""]]]"#,
        ),
        (
            r#""bacdd" | [match("(?:b*|a)+(?:c*|d)+(x|(b*|a)+)";"g") | [.string, (.captures|map(.string))]]"#,
            r#"[["b",["",""]],["",["",""]],["c",["",""]],["",["",""]],["",["",""]],["",["",""]]]"#,
        ),
        (
            r#""ba" | [match("(b*|a){0,}";"g") | .string]"#,
            r#"["b","",""]"#,
        ),
        (
            r#""ba" | [match("(b*|a){1,}";"g") | [.string, (.captures|map(.string))]]"#,
            r#"[["b",[""]],["",[""]],["",[""]]]"#,
        ),
        (
            r#""ba" | [match("(b*|a)+?";"g") | [.string, (.captures|map(.string))]]"#,
            r#"[["b",["b"]],["",[""]],["",[""]]]"#,
        ),
        (
            r#""ba" | [match("(?<y>c)?(?<x>b*|a)+z?";"g") | [.string, (.captures|map([.name,.string]))]]"#,
            r#"[["b",[["y",null],["x",""]]],["",[["y",""],["x",""]]],["",[["y",""],["x",""]]]]"#,
        ),
        (
            r#""ba" | [match("(b*|a)+";"gl") | .string]"#,
            r#"["ba",""]"#,
        ),
    ])
}
