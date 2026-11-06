//! #6424: a loop body with a second way that is only an anchor, or an empty way, after the first.
//! Oniguruma tries what follows the loop once at a head, at the first exit that holds there, so a
//! second exit with no consuming way after it is the one every loop ends with, and an anchor that
//! holds only where the first does is never the first to hold. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq`), and the first group differed from it before.

use anyhow::Result;

use crate::run_jq_full;

/// The same body inside another group or repeat, under a flag switch, or beside a group of its own.
#[test]
fn a_second_exit_in_a_nested_group_or_beside_a_flag_switch_6424() -> Result<()> {
    for (pattern, input, expected) in [
        (
            r"((\A|a|\B))*",
            "aab",
            r#"[[0,"",["",""]],[1,"a",["",""]],[2,"",["",""]],[3,"",["",""]]]"#,
        ),
        (
            r"((?:\A|a|\B))*",
            "aab",
            r#"[[0,"",[""]],[1,"a",[""]],[2,"",[""]],[3,"",[""]]]"#,
        ),
        (
            r"x?(\A|a|\B)+",
            "aab",
            r#"[[0,"",[""]],[1,"a",[""]],[2,"",[""]]]"#,
        ),
        (
            r"(((\b|a|\B))*b)*",
            "ab",
            r#"[[0,"ab",["ab","",""]],[2,"",["","",""]]]"#,
        ),
        (
            r"(\b|(?i)a|\b)*",
            "ba",
            r#"[[0,"",[""]],[1,"a",[""]],[2,"",[""]]]"#,
        ),
        (
            r"((?i)\b|a|\B)*",
            "ba",
            r#"[[0,"",[""]],[1,"a",[""]],[2,"",[""]]]"#,
        ),
        (
            r"(?i)(\b|a|\B)+",
            "ba",
            r#"[[0,"",[""]],[1,"a",[""]],[2,"",[""]]]"#,
        ),
        (
            r"(\b|(a)|\B)*",
            "ba",
            r#"[[0,"",["",""]],[1,"a",["","a"]],[2,"",["",""]]]"#,
        ),
        (
            r"(\b|a|\B)*(\b|b|\B)*",
            "ab",
            r#"[[0,"",["",""]],[1,"b",["",""]],[2,"",["",""]]]"#,
        ),
    ] {
        assert_matches(pattern, input, expected)?;
    }
    Ok(())
}

/// A second anchor, or a way that matches nothing, after the first with no consuming way after it
/// sets the group at the end of every head's ways, not at the first exit.
#[test]
fn a_second_exit_with_no_way_after_it_ends_the_loop_last_6424() -> Result<()> {
    for (pattern, input, expected) in [
        (
            r"(\A|a|\B)+",
            "bab",
            r#"[[0,"",[""]],[1,"a",[""]],[2,"",[""]]]"#,
        ),
        (
            r"(\B|(?:a|b*)){2,}",
            "ba",
            r#"[[0,"b",[""]],[1,"",[""]],[2,"",[""]]]"#,
        ),
        (
            r"(\B|(?:a|b*)){2,}",
            "bab",
            r#"[[0,"b",[""]],[1,"",[""]],[2,"",[""]],[3,"",[""]]]"#,
        ),
        (
            r"(\A|a|\z)*",
            "ba",
            r#"[[0,"",[""]],[1,"a",[""]],[2,"",[""]]]"#,
        ),
        (
            r"(\A|a|\z)+",
            "ba",
            r#"[[0,"",[""]],[1,"a",[""]],[2,"",[""]]]"#,
        ),
        (
            r"(\A|a|\z){2,}",
            "baa",
            r#"[[0,"",[""]],[1,"aa",[""]],[3,"",[""]]]"#,
        ),
        (
            r"(\z|a|\b)+",
            "a b",
            r#"[[0,"a",[""]],[1,"",[""]],[2,"",[""]],[3,"",[""]]]"#,
        ),
        (
            r"(a|\b|\B)*",
            "ab",
            r#"[[0,"a",[""]],[1,"",[""]],[2,"",[""]]]"#,
        ),
        (
            r"(a|\A|b|\z)*",
            "ba",
            r#"[[0,"",[""]],[1,"a",[""]],[2,"",[""]]]"#,
        ),
        (r"(a|\z|b|\A)*", "ba", r#"[[0,"ba",[""]],[2,"",[""]]]"#),
        (
            r"(\b|a|\B)*",
            "ba",
            r#"[[0,"",[""]],[1,"a",[""]],[2,"",[""]]]"#,
        ),
        (
            r"(\b|a|\B){2,}",
            "baa",
            r#"[[0,"",[""]],[1,"aa",[""]],[3,"",[""]]]"#,
        ),
        (r"(\B|a|\A){2}", "a b", r#"[[0,"",[""]]]"#),
        (r"(\B|a|\A){2,3}", "a b", r#"[[0,"",[""]]]"#),
        (r"(b|\B|a|\A){2,3}", "a b", r#"[[0,"",[""]]]"#),
    ] {
        assert_matches(pattern, input, expected)?;
    }
    Ok(())
}

/// Rows jq answers as the loop did before, which the second exit must not move: counted and bounded
/// repeats, a lazy repeat, and an empty way as the second exit.
#[test]
fn a_second_exit_that_changes_nothing_stays_as_jq_answers_6424() -> Result<()> {
    for (pattern, input, expected) in [
        (
            r"(\b|a|\B){0,3}",
            "ab",
            r#"[[0,"",[""]],[1,"",[""]],[2,"",[""]]]"#,
        ),
        (
            r"(\b|a|\B){2,3}",
            "a b",
            r#"[[0,"",[""]],[1,"",[""]],[2,"",[""]],[3,"",[""]]]"#,
        ),
        (r"(\z|a|\b){1,3}", "aab", r#"[[0,"aa",["a"]],[3,"",[""]]]"#),
        (
            r"(\z|a|\b){2}",
            "a b",
            r#"[[0,"a",[""]],[1,"",[""]],[2,"",[""]],[3,"",[""]]]"#,
        ),
        (
            r"(\b|(?:a|b*)){2}",
            "ab",
            r#"[[0,"",[""]],[1,"b",[""]],[2,"",[""]]]"#,
        ),
        (
            r"(\B|(?:a|b*)){2}",
            "ab",
            r#"[[0,"a",[""]],[1,"",[""]],[2,"",[""]]]"#,
        ),
        (
            r"(\A|(?:a|b*))*",
            "ba",
            r#"[[0,"",[""]],[1,"a",[""]],[2,"",[""]]]"#,
        ),
        (r"(\z|(?:a|b*))+", "ba", r#"[[0,"ba",[""]],[2,"",[""]]]"#),
        (
            r"(\b|a|\B)+?",
            "ab",
            r#"[[0,"",[""]],[1,"",[""]],[2,"",[""]]]"#,
        ),
        (
            r"(\A|a|\z){2,}?",
            "baa",
            r#"[[0,"",[""]],[1,"aa",["a"]],[3,"",[""]]]"#,
        ),
    ] {
        assert_matches(pattern, input, expected)?;
    }
    Ok(())
}

/// The same anchor again, or `\A` and `^`, holds only where the first does.
#[test]
fn an_anchor_that_holds_only_where_the_first_does_is_dropped_6424() -> Result<()> {
    for (pattern, input, expected) in [
        (
            r"(\b|a|\b|b)*",
            "ab",
            r#"[[0,"",[""]],[1,"b",[""]],[2,"",[""]]]"#,
        ),
        (r"(\b|a|\b)+", "ab", r#"[[0,"",[""]],[2,"",[""]]]"#),
        (
            r"(\b|a|\b|b){2,}",
            "ab",
            r#"[[0,"",[""]],[1,"b",[""]],[2,"",[""]]]"#,
        ),
        (r"(\A|a|^|b)+", "ab", r#"[[0,"",[""]],[1,"b",["b"]]]"#),
        (
            r"(\A|a|^|b){0,2}",
            "ab",
            r#"[[0,"",[""]],[1,"b",["b"]],[2,"",[""]]]"#,
        ),
        (r"($|a|\z|b)*", "ab", r#"[[0,"ab",[""]],[2,"",[""]]]"#),
        (
            r"(\z|a|\z|ab)*",
            "ab",
            r#"[[0,"a",["a"]],[1,"",[""]],[2,"",[""]]]"#,
        ),
    ] {
        assert_matches(pattern, input, expected)?;
    }
    Ok(())
}

/// Matches of `pattern` in `input` under `g`, as `[offset, string, captures]` rows.
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
