//! #6183: an inline option run (`(?i)`) in a lookaround or an atomic group lasts past its `)` in
//! `fancy-regex`, and Oniguruma ends it there as it does for any group; a letter repeated in a run
//! (`(?ii)`) is applied once. Every row was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn an_inline_option_in_a_lookaround_or_atomic_group_ends_with_it_6183() -> Result<()> {
    for (filter, expected) in [
        (r#""aAbB" | [match("(?=(?i)a)a";"g")|.string]"#, r#"["a"]"#),
        (r#""aAbB" | [match("(?!(?i)a)b";"g")|.string]"#, r#"["b"]"#),
        (
            r#""aAbB" | [match("(?=(?i)a|b)a";"g")|.string]"#,
            r#"["a"]"#,
        ),
        (
            r#""aAbB" | [match("(?=((?i)a))a";"g")|.string]"#,
            r#"["a"]"#,
        ),
        (r#""ab" | [match("(?<=(?i)a)B";"g")|.string]"#, "[]"),
        (r#""ab" | [match("(?<!(?i)x)B";"g")|.string]"#, "[]"),
        (r#""ab" | [match("(?>(?i)a)B";"g")|.string]"#, "[]"),
        (r#""ab" | [match("(?=(?i)a)aB";"g")|.string]"#, "[]"),
        (
            r#""aAbB" | [match("(?i)(?=(?-i)a)A";"g")|.string]"#,
            r#"["a"]"#,
        ),
        (r#""aAbB" | [match("(?=(?i)ab)aB";"g")|.string]"#, "[]"),
        (
            r#""aAbB" | [match("(?=(?i)a)(?=(?i)a)a";"g")|.string]"#,
            r#"["a"]"#,
        ),
        (r#""aAbB" | [match("(?<=(?i)a)b";"g")|.string]"#, r#"["b"]"#),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

#[test]
fn a_letter_repeated_in_a_run_is_applied_once_6183() -> Result<()> {
    for (filter, expected) in [
        (r#""aAbB" | [match("((?ii)a)B";"g")|.string]"#, "[]"),
        (r#""aAbB" | [match("(?ii)a";"g")|.string]"#, r#"["a","A"]"#),
        (r#""ab" | [match("(?<n>(?ii)a)B";"g")|.string]"#, "[]"),
        (r#""aAbB" | [match("(?i-ii:a)B";"g")|.string]"#, "[]"),
        (r#""aAbB" | [match("(?i-ii)a";"g")|.string]"#, r#"["a"]"#),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

/// A `)` in a class, or in a `#` comment under `x`, before the option run does not end the body.
#[test]
fn a_paren_in_a_class_or_comment_does_not_end_the_wrapped_body_6183() -> Result<()> {
    for (filter, expected) in [
        (r#"")ab" | [match("(?=[)](?i)a)[)]aB";"g")|.string]"#, "[]"),
        (
            r#""xab" | [match("(?=[^)](?i)a)[^)]aB";"g")|.string]"#,
            "[]",
        ),
        (r#"")abb" | [match("(?<=[)](?i)a)bB";"g")|.string]"#, "[]"),
        (r#"")abb" | [match("(?>[)](?i)a)bB";"g")|.string]"#, "[]"),
        (
            r#""ab" | [match("(?x)(?=a # )\n(?i)b)aB";"g")|.string]"#,
            "[]",
        ),
        (
            r#""abC" | [match("(?x)(?<=a # )\n(?i)b)c";"g")|.string]"#,
            "[]",
        ),
        (
            r#""aab" | [match("(?=[[:alpha:])](?i)a)aB";"g")|.string]"#,
            "[]",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}
