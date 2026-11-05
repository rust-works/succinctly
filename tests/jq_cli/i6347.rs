//! #6347: a counted `{n,}` (n of 2 or more) of a capture group that has a group inside it ends at the
//! body's empty way and sets the outer group there, as Oniguruma's does (`"ba" | match("((a)|b*){2,}")`
//! has `["","a"]`), where the crate's copies of the body kept `["a","a"]`. The capture rewrite of
//! #6309 now writes such a repeat as the `*` loop, and the loop rewrite of #6332 reads its groups.
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

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

fn row(subject: &str, pattern: &str) -> String {
    format!(
        "\"{subject}\" | [match(\"{pattern}\";\"g\") | [.offset, .string, (.captures|map(.string))]]"
    )
}

#[test]
fn test_a_counted_repeat_of_a_group_with_a_group_inside_ends_at_the_empty_way_6347() -> Result<()> {
    for (subject, pattern, expected) in [
        (
            "ba",
            "((a)|b*){2,}",
            r#"[[0,"ba",["","a"]],[2,"",["",""]]]"#,
        ),
        (
            "bab",
            "((a)|b*){3,}",
            r#"[[0,"bab",["","a"]],[3,"",["",""]]]"#,
        ),
        (
            "abab",
            "((a)|b*){2,}",
            r#"[[0,"abab",["","a"]],[4,"",["",""]]]"#,
        ),
        (
            "ba",
            "((a)|(b)|c*){2,}",
            r#"[[0,"ba",["","a","b"]],[2,"",["","",""]]]"#,
        ),
        (
            "ba",
            "(((a)|b*)c*){2,}",
            r#"[[0,"ba",["","","a"]],[2,"",["","",""]]]"#,
        ),
        (
            "ba",
            "(?<x>(a)|b*){2,}",
            r#"[[0,"ba",["","a"]],[2,"",["",""]]]"#,
        ),
    ] {
        assert_rows(&[(&row(subject, pattern), expected)])?;
    }
    assert_rows(&[(
        r#""ba" | [capture("(?<x>(?<y>a)|b*){2,}";"g")]"#,
        r#"[{"x":"","y":"a"},{"x":"","y":""}]"#,
    )])
}

/// What already agreed keeps agreeing: a counted repeat of a capture group with no group inside it,
/// and the `*` and `+` loops over the same group.
#[test]
fn test_loops_over_a_group_that_already_agreed_still_do_6347() -> Result<()> {
    for (subject, pattern, expected) in [
        ("abb", "(b|){2,}$", r#"[[1,"bb",[""]],[3,"",[""]]]"#),
        ("ba", "((a)|b*)+", r#"[[0,"ba",["","a"]],[2,"",["",""]]]"#),
    ] {
        assert_rows(&[(&row(subject, pattern), expected)])?;
    }
    Ok(())
}
