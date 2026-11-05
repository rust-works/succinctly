//! #6332: a loop whose body has a capture group before its empty way, in it, or with its own empty
//! way first ends at the empty iteration, and sets the groups that iteration sets, as Oniguruma's
//! does (`"abca" | match("(?:(a)|b*|c)+")` is `"ab"`), where the `regex` crate matched `"abca"`. The
//! loop is written with the groups copied, and each copy is read back into its group. Every row was
//! captured from jq 1.7.1 (`/usr/bin/jq -nc`).

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

const MATCHES: &str = r#"[match(P;"g") | [.offset, .string, (.captures|map(.string))]]"#;

fn row(subject: &str, pattern: &str) -> String {
    format!(
        "\"{subject}\" | {}",
        MATCHES.replace('P', &format!("\"{pattern}\""))
    )
}

#[test]
fn test_a_group_before_the_empty_way_is_copied_and_read_back_6332() -> Result<()> {
    for (subject, pattern, expected) in [
        (
            "abca",
            r"(?:(a)|b*|c)+",
            r#"[[0,"ab",["a"]],[2,"",[""]],[3,"a",["a"]],[4,"",[""]]]"#,
        ),
        (
            "abc",
            r"(?:(a)b|b*|(c))+",
            r#"[[0,"ab",["a",null]],[2,"",["",""]],[3,"",["",""]]]"#,
        ),
        (
            "abca",
            r"(?:(a)|b*|c)+(?:(d)|e*|f)*",
            r#"[[0,"ab",["a",null]],[2,"",["",""]],[3,"a",["a",null]],[4,"",["",""]]]"#,
        ),
    ] {
        assert_rows(&[(&row(subject, pattern), expected)])?;
    }
    // Beside a repeated group the capture rewrite of #6069 writes too, and the copies are composed.
    assert_rows(&[(
        &row("abc", r"(a*)*(?:(a)|b*|c)+"),
        r#"[[0,"ab",["",null]],[2,"",["",""]],[3,"",["",""]]]"#,
    )])?;
    assert_rows(&[(
        &row("abc", r"((?:(a)|b*|c)+)(b*)*"),
        r#"[[0,"ab",["ab","a",""]],[2,"",["","",""]],[3,"",["","",""]]]"#,
    )])?;
    // A name stays on its group, whichever copy the match set.
    assert_rows(&[(
        r#""abca" | [match("(?:(?<x>a)|b*|c)+";"g") | [.string, (.captures|map([.name,.string]))]]"#,
        r#"[["ab",[["x","a"]]],["",[["x",""]]],["a",[["x","a"]]],["",[["x",""]]]]"#,
    )])?;
    assert_rows(&[(
        r#""abca" | [capture("(?:(?<x>a)|b*|c)+";"g")]"#,
        r#"[{"x":"a"},{"x":""},{"x":"a"},{"x":""}]"#,
    )])
}

#[test]
fn test_a_group_in_the_empty_way_is_set_where_the_loop_leaves_6332() -> Result<()> {
    for (subject, pattern, expected) in [
        (
            "ba",
            r"(?:b*|()|a)+",
            r#"[[0,"b",[null]],[1,"",[""]],[2,"",[""]]]"#,
        ),
        (
            "ba",
            r"(?:b*|(a)|())*",
            r#"[[0,"b",[null,null]],[1,"",["",""]],[2,"",["",""]]]"#,
        ),
        (
            "bab",
            r"(?:a|(a*)|(\\w*))+",
            r#"[[0,"",["",""]],[1,"a",["",null]],[2,"",["",""]],[3,"",["",""]]]"#,
        ),
        (
            "bab",
            r"(?:a*(a*)|(b*))*",
            r#"[[0,"",["",""]],[1,"a",["",null]],[2,"",["",""]],[3,"",["",""]]]"#,
        ),
        (
            "bab",
            r"(?:a*|(b)*|(?:a|b*)())*",
            r#"[[0,"",["",""]],[1,"a",[null,null]],[2,"",["",""]],[3,"",["",""]]]"#,
        ),
    ] {
        assert_rows(&[(&row(subject, pattern), expected)])?;
    }
    Ok(())
}

#[test]
fn test_the_builtins_over_a_loop_with_a_group_before_its_empty_way_agree_with_jq_6332() -> Result<()>
{
    assert_rows(&[
        (r#""abca" | sub("(?:(a)|b*|c)+"; "X")"#, r#""Xca""#),
        (r#""abca" | gsub("(?:(a)|b*|c)+"; "X")"#, r#""XXcXX""#),
        (
            r#""abca" | [scan("(?:(a)|b*|c)+")]"#,
            r#"[["a"],[""],["a"],[""]]"#,
        ),
        (r#""abca" | test("(?:(a)|b*|c)+d")"#, "false"),
    ])
}

/// The flags `i` and `p` put a switch in front of the pattern, which the rewrite leaves there.
#[test]
fn test_the_flags_that_prefix_a_switch_keep_the_fix_6332() -> Result<()> {
    assert_rows(&[
        (
            r#""ABCA" | [match("(?:(a)|b*|c)+";"gi") | [.string, (.captures|map(.string))]]"#,
            r#"[["AB",["A"]],["",[""]],["A",["A"]],["",[""]]]"#,
        ),
        (
            r#""abca" | [match("(?:(a)|b*|c)+";"gp") | .string]"#,
            r#"["ab","","a",""]"#,
        ),
        (r#""ABCA" | gsub("(?:(a)|b*|c)+";"X";"gi")"#, r#""XXCXX""#),
    ])
}

/// What already agreed keeps agreeing: a counted repeat whose copies Oniguruma writes out, a group
/// in the first way of a loop with no way after the empty one, and a loop with no group.
#[test]
fn test_loops_that_already_agreed_still_do_6332() -> Result<()> {
    for (subject, pattern, expected) in [
        (
            "aab",
            r"(?:(a)|b*|c){2}",
            r#"[[0,"aa",["a"]],[2,"b",[null]],[3,"",[""]]]"#,
        ),
        (
            "abab",
            r"(?:(a)|b*|c){1,3}",
            r#"[[0,"aba",["a"]],[3,"b",[null]],[4,"",[""]]]"#,
        ),
        (
            "abab",
            r"(?:(a)|b*|c){2,}",
            r#"[[0,"abab",["a"]],[4,"",[""]]]"#,
        ),
        (
            "ba",
            r"(?:(|a)|b)+",
            r#"[[0,"",[""]],[1,"",[""]],[2,"",[""]]]"#,
        ),
        // A loop with groups inside a repeat that runs it again is left as written.
        (
            "abab",
            r"(?:(?:(a|b)|(?:|a)){0,3}){2}?",
            r#"[[0,"abab",["b"]],[4,"",[""]]]"#,
        ),
        (
            "bab",
            r"(?:(?:b*|(a))+(a|b)|(?:b*|(a))+){2}",
            r#"[[0,"bab",[null,"b",null]],[3,"",["","",""]]]"#,
        ),
        (
            "abc",
            r"(?:((b|)(|c)){3})+",
            r#"[[0,"",["","",""]],[1,"b",["","",""]],[2,"",["","",""]],[3,"",["","",""]]]"#,
        ),
    ] {
        assert_rows(&[(&row(subject, pattern), expected)])?;
    }
    Ok(())
}
