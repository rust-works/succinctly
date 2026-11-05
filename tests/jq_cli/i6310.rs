//! #6310: a loop whose body has a capture group in a way after its empty one ends at that empty
//! way, as Oniguruma's does (`"ba" | match("(?:b*|(a))+")` is `"b"`), where the `regex` crate
//! matched `"ba"`. The loop rewrite of #6106 now lists a capture group's ways, and keeps the groups
//! of the pattern in number and order. Every row was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

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
fn test_a_loop_with_a_capture_group_in_a_later_way_ends_at_the_empty_way_6310() -> Result<()> {
    assert_rows(&[
        (r#""ba" | [match("(?:b*|(a))+") | .string]"#, r#"["b"]"#),
        (r#""ba" | [match("(b*|(a))+") | .string]"#, r#"["b"]"#),
        (
            r#""bab" | [match("(?:b*|(a))+";"g") | [.string, (.captures|map(.string))]]"#,
            r#"[["b",[null]],["",[""]],["b",[null]],["",[""]]]"#,
        ),
        (
            r#""ba" | [match("(?<x>b*|(?<y>a))+") | [.string, (.captures|map([.name,.string]))]]"#,
            r#"[["b",[["x",""],["y",null]]]]"#,
        ),
        (
            r#""xba" | [match("x(?:b*|(a))+") | [.string, (.captures|map(.string))]]"#,
            r#"[["xb",[null]]]"#,
        ),
        // Two groups, one of them in a group of its own, and a group a later way holds of its own.
        (
            r#""bab" | [match("(?:b*|(a)|(c))*";"g") | [.string, (.captures|map(.string))]]"#,
            r#"[["b",[null,null]],["",["",""]],["b",[null,null]],["",["",""]]]"#,
        ),
        (
            r#""ba" | [match("(?:b*|((a)))+") | [.string, (.captures|map(.string))]]"#,
            r#"[["b",[null,null]]]"#,
        ),
        // A group that may match nothing, in a later way: its own empty way is an exit already tried.
        (
            r#""ba" | [match("(?:b*|(a*))+") | [.string, (.captures|map(.string))]]"#,
            r#"[["b",[null]]]"#,
        ),
    ])
}

#[test]
fn test_the_builtins_over_a_loop_with_a_capture_group_agree_with_jq_6310() -> Result<()> {
    assert_rows(&[
        (r#""ba" | sub("(?:b*|(a))+"; "X")"#, r#""Xa""#),
        (r#""foo-bar" | gsub("(?:\\w*|(-))+"; "X")"#, r#""XX-XX""#),
        (r#""ba" | [scan("(?:b*|(a))+")]"#, r#"[[null],[""],[""]]"#),
        (
            r#""BA" | [match("(?:b*|(a))+";"gi") | .string]"#,
            r#"["B","",""]"#,
        ),
        (r#""ba" | test("(?:b*|(a))+a")"#, "true"),
    ])
}

/// What already agreed keeps agreeing: a group in the first way, a group in the empty way's own
/// loop, a counted loop (its copies would each hold the group), and a loop over a group whose
/// empty way comes first.
#[test]
fn test_loops_with_a_capture_group_that_already_agreed_still_do_6310() -> Result<()> {
    assert_rows(&[
        (
            r#""ba" | [match("(?:|(a))+") | [.string, (.captures|map(.string))]]"#,
            r#"[["",[""]]]"#,
        ),
        (
            r#""ba" | [match("(?:(a)|b*|c)+") | [.string, (.captures|map(.string))]]"#,
            r#"[["ba",["a"]]]"#,
        ),
        (
            r#""ba" | [match("(?:b*|(a)){2}") | [.string, (.captures|map(.string))]]"#,
            r#"[["b",[null]]]"#,
        ),
        (
            r#""bab" | [match("(?:b*|(a)){2,3}?";"g") | [.string, (.captures|map(.string))]]"#,
            r#"[["b",[null]],["",[""]],["b",[null]],["",[""]]]"#,
        ),
        (
            r#""ba" | [match("(?:(?:|a)|b?)*(c)?") | [.string, (.captures|map(.string))]]"#,
            r#"[["",[""]]]"#,
        ),
        (
            r#""ac" | [match("((?:|a)|b?)*c") | [.string, (.captures|map(.string))]]"#,
            r#"[["ac",[""]]]"#,
        ),
    ])
}
