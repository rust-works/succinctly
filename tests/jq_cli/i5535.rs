//! #5535: Oniguruma refuses a repetition operator right after an assertion (`^*`, `$+`, `\b?`,
//! `\A{2}`) with `target of repeat operator is invalid`, where the `regex` crate repeats the
//! empty match and `succinctly jq` answered `true`. It is refused now, with the `invalid regex:`
//! prefix every refused pattern here has. Rows were captured from jq 1.7.1 (`/usr/bin/jq -n`).

use anyhow::Result;

use crate::run_jq_full;

fn filter(pattern: &str) -> String {
    format!(
        r#""a" | test("{}")"#,
        pattern.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

#[test]
fn test_a_repetition_after_an_assertion_is_refused_5535() -> Result<()> {
    for pattern in [
        r"\b*", r"\b+", r"\b?", r"\b{2}", r"\b{2,}", r"\b{2,3}", r"\B*", r"\A*", r"\z+", r"\Z?",
        r"\G*", "^*", "^+", "^?", "^{2}", "$*", "$+", "$?", "$*?", "^*?", "^+?", r"\b*+", "a|^*",
        "(?i)^*", "a^*", "a$*", "^$*", r"\b+\b", "x^+", r"a\Z*", "^{2,3}",
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter(pattern)], None)?;
        assert_eq!(code, 5, "{pattern}: stdout={stdout:?} stderr={stderr:?}");
        assert_eq!(stdout, "", "{pattern}");
        assert!(
            stderr.contains("invalid regex: target of repeat operator is invalid"),
            "{pattern}: {stderr:?}"
        );
    }
    Ok(())
}

#[test]
fn test_what_is_not_a_repetition_of_an_assertion_is_unchanged_5535() -> Result<()> {
    // Escaped, in a class, separated by a literal, in a group or alternation, or a `{` that is
    // not an interval: all answer in jq and here.
    for (pattern, expected) in [
        (r"\^*", "true"),
        (r"\$+", "false"),
        ("[$]*", "true"),
        ("[^$]*", "true"),
        ("[a^]+", "true"),
        ("^ *", "true"),
        (r"\b *", "true"),
        (r"\<*", "true"),
        ("(^)*", "true"),
        ("(a|$)?", "true"),
        ("(a|^)+", "true"),
        ("^a*", "true"),
        ("^.*$", "true"),
        ("^(a|b)+$", "true"),
        (r"\b\w+", "true"),
        (r"^\d+$", "false"),
        ("^^", "true"),
        (r"\b\b", "true"),
        ("a$|b", "true"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter(pattern)], None)?;
        assert_eq!(stdout.trim_end(), expected, "{pattern}: stderr={stderr:?}");
        assert_eq!(code, 0, "{pattern}");
    }
    Ok(())
}
