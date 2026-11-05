//! #6249: under the `x` flag a repeat operator with nothing to repeat (`*0`, `a| *b`, `(?: *a)`) is
//! refused as jq refuses it, `target of repeat operator is not specified`. The flag's `(?x)` prefix
//! is a group that `fancy-regex` would repeat, so it answered as if the operator were not there.
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_repeat_with_nothing_to_repeat_is_refused_6249() -> Result<()> {
    for (subject, pattern, flags) in [
        (r"x0", r"*0", "gx"),
        (r"x0", r" *0", "gx"),
        (r"x0", r"+0", "gx"),
        (r"x0", r" +0", "gx"),
        (r"x0", r"?0", "gx"),
        (r"x0", r" {2}0", "gx"),
        (r"x0", r"a| *b", "gx"),
        (r"x0", r"a|*b", "gx"),
        (r"x0", r"(?: *a)", "gx"),
        (r"x0", r"(?:a| *)", "gx"),
        (r"x0", r"(?=*a)", "gx"),
        (r"x0", r"*", "gx"),
        (r"x0", r" *", "gx"),
        (r"x0", r"*0", "g"),
        (r"x0", r"a|+", "g"),
        (r"x0", r"(?:?a)", "g"),
    ] {
        let filter = format!(
            r#"{} | [match({}; {}) | .string]"#,
            serde_json::to_string(subject)?,
            serde_json::to_string(pattern)?,
            serde_json::to_string(flags)?
        );
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter], None)?;
        assert_eq!(stdout.trim_end(), "", "{filter}");
        assert_ne!(code, 0, "{filter}");
        assert!(
            stderr.contains("repeat operator") || stderr.contains("regex"),
            "{filter}: {stderr:?}"
        );
    }
    Ok(())
}

/// An operator with a target still repeats it, past blanks and comments.
#[test]
fn an_operator_with_a_target_still_repeats_it_6249() -> Result<()> {
    for (filter, expected) in [
        (r#""x0 a b" | [match("a *0"; "gx") | .string]"#, r#"["0"]"#),
        (
            r#""aab" | [match("a *"; "gx") | .string]"#,
            r#"["aa","",""]"#,
        ),
        (
            r#""a b" | [match("a|b*"; "gx") | .string]"#,
            r#"["a","","b",""]"#,
        ),
        (r#""aab" | [match("(?:a )+"; "gx") | .string]"#, r#"["aa"]"#),
        (r#""a0" | [match("a #c\n*0"; "gx") | .string]"#, r#"["a0"]"#),
        (
            r#""a b" | [match("a|b*"; "g") | .string]"#,
            r#"["a","","b",""]"#,
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}
