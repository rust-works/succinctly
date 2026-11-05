//! #6309: a `{n,}` repeat (n of 2 or more) of a capture group is the `*` loop for a body that always
//! matches empty, since Oniguruma's empty-iteration check ends the loop before the minimum count: it
//! ends at the body's empty way and the group holds that empty iteration. Every row was captured from
//! jq 1.7.1 (`/usr/bin/jq -nc`).

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
fn test_a_counted_repeat_of_a_capture_group_reports_its_empty_iteration_6309() -> Result<()> {
    assert_rows(&[
        (
            r#""abb" | [match("(b|){2,}$";"g") | [.string, .captures[0].string]]"#,
            r#"[["bb",""],["",""]]"#,
        ),
        (
            r#""ba" | [match("(b*|a){2,}";"g") | [.string, .captures[0].string]]"#,
            r#"[["b",""],["",""],["",""]]"#,
        ),
        (
            r#""bab" | [match("(b*|a){3,}";"g") | [.string, .captures[0].string]]"#,
            r#"[["b",""],["",""],["b",""],["",""]]"#,
        ),
        (
            r#""ba" | [match("(?<x>b*|a){2,}";"g") | [.string, (.captures|map([.name,.string]))]]"#,
            r#"[["b",[["x",""]]],["",[["x",""]]],["",[["x",""]]]]"#,
        ),
        (
            r#""abab" | [match("(a*|b){2,}ab";"g") | [.string, .captures[0].string]]"#,
            r#"[["abab",""]]"#,
        ),
        (r#""ba" | sub("(b*|a){2,}"; "X")"#, r#""Xa""#),
        (
            r#""ba" | [match("(b|){2,}c?"; "g") | .string]"#,
            r#"["b","",""]"#,
        ),
        (
            r#""bacb" | [match("(a|b*){2,}c";"g") | [.string, .captures[0].string]]"#,
            r#"[["bac",""]]"#,
        ),
    ])
}

/// Bodies with a group inside them keep the `regex` crate's own copies, which answer as jq does for
/// these.
#[test]
fn test_a_counted_repeat_over_a_nested_group_is_left_alone_6309() -> Result<()> {
    assert_rows(&[
        (
            r#""ab" | [match("(?:(a*|(a|))|b){2,}";"g") | .string]"#,
            r#"["a","",""]"#,
        ),
        (
            r#""aca" | [match("(c)?((a*)|(a)|c){2,}";"g") | .string]"#,
            r#"["a","ca",""]"#,
        ),
    ])
}
