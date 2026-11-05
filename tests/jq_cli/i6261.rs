//! #6261: a repeat operator after a comment with nothing before it (`(?#c)*`) has no target in jq
//! 1.7.1 (`Regex failure: target of repeat operator is not specified`). Under the `i` or `x` flag the
//! `(?i)` or `(?x)` put before the pattern made the `regex` crate read the operator as repeating the
//! comment, and the pattern was accepted. Every refusal was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_a_repeat_after_a_comment_with_nothing_before_it_is_refused_under_every_flag_6261(
) -> Result<()> {
    for pattern in [
        "(?#c)*",
        "(?#c)+",
        "(?#c)?",
        "(?#c){2}",
        "(?#c)*?",
        "a|(?#c)*",
        "(?:(?#c)*)",
        "(?#c)(?#d)*",
        // Or at the start of another group, or after a bare option group.
        "((?#c)*)",
        "(?<n>(?#c)+)",
        "(?=(?#c)*)",
        "(?i:(?#c)*)",
        "(?i)(?#c)*",
        "(?x)(?#c)*",
        "(?-x)(?#c)*",
        "a(?i)(?#c)*",
    ] {
        for flags in ["g", "gi", "gx", "gn", "gnx"] {
            let (stdout, stderr, code) = run_jq_full(
                &[
                    "-nc",
                    "--arg",
                    "p",
                    pattern,
                    "--arg",
                    "f",
                    flags,
                    r#""aab" | [match($p; $f) | .string]"#,
                ],
                None,
            )?;
            assert_eq!(
                (stdout.trim_end(), code),
                ("", 5),
                "{pattern} {flags}: {stderr}"
            );
            assert!(
                stderr.contains("target of repeat operator is not specified"),
                "{pattern} {flags}: {stderr}"
            );
        }
    }
    Ok(())
}

/// A comment after an item repeats that item, whatever the flags (`b(?#)*` is `b*`, #6128).
#[test]
fn test_a_repeat_after_a_comment_with_an_item_before_it_still_repeats_the_item_6261() -> Result<()>
{
    for flags in ["g", "gi", "gx"] {
        let (stdout, stderr, code) = run_jq_full(
            &[
                "-nc",
                "--arg",
                "f",
                flags,
                r#""aab" | [match("a(?#c)*"; $f) | .string]"#,
            ],
            None,
        )?;
        assert_eq!(
            (stdout.trim_end(), code),
            (r#"["aa","",""]"#, 0),
            "{flags}: {stderr}"
        );
    }
    Ok(())
}
