//! #6127: Oniguruma refuses to repeat a group it reads as an assertion with `target of repeat
//! operator is invalid`, where the `regex` crate repeated the empty match and `succinctly jq`
//! answered. It decides from the parsed node (`is_invalid_quantifier_target`, `regparse.c`): an
//! assertion is an invalid target, a sequence of several nodes is not, an alternation is when any
//! one alternative is, a plain `(?:...)` group is its body, and a capturing, named, atomic or option
//! group is an enclosure that may repeat anything. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -n`); `scripts/jq-regex-repeat-target-sweep.py` regenerates the 75,780-group sweep
//! they were picked from.

use anyhow::Result;

use crate::run_jq_full;

fn filter(pattern: &str) -> String {
    format!(
        r#""ab" | test("{}")"#,
        pattern.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

#[test]
fn test_a_repeated_group_that_is_an_assertion_is_refused_6127() -> Result<()> {
    for pattern in [
        r"(?:\b)*",
        r"(?:\b)+",
        r"(?:\B)?",
        "(?:^){1,2}",
        "(?:$)*?",
        r"(?:\A)+",
        r"(?:\z){2}",
        r"(?:\Z)*",
        r"(?:\G)*",
        r"(?:a|\b)*",
        r"(?:a|\b|b)*",
        "(?:a|$|b)+",
        r"(?:a|\B)+",
        r"(?:\w|\b| )+",
        r"(?:\b|)*",
        r"(?:|\b)*",
        r"(?:()|\b)?",
        r"(?:(?:\b))*",
        r"(?:(?:\b)|c)*",
        r"(?:a|(?:b|\b))+",
        r"(?:\b(?#c))*",
        r"(?:(?i)\b)*",
        r"(?:a|(?i)\b)*",
        r"(?:(?i-m)\b)*",
        r"(?:\b)*+",
        r"(?:a|\b)**",
        "(?=a)*",
        "(?!a)+",
        "(?<=a)?",
        "(?:(?=a))*",
        "(?:a|(?<!a))*",
        r"(?:\Q(\E|\b)*",
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
fn test_a_repeated_group_that_is_not_an_assertion_still_answers_6127() -> Result<()> {
    for (pattern, expected) in [
        (r"(?:\b\b)*", "true"),
        (r"(?:|\b\b)+", "true"),
        (r"(?:a|\b\b)?", "true"),
        (r"(?:\b\b|a){2}", "true"),
        (r"(?:\b\b|())*", "true"),
        (r"(\b)*", "true"),
        (r"(?<n>\b)*", "true"),
        (r"(?>\b)*", "true"),
        (r"(?i:\b)*", "true"),
        (r"(?:x(?:\b|c))*", "true"),
        (r"(?:\bx|c)*", "true"),
        (r"(?:\b)x*", "true"),
        (r"(?:a|\b)(?:)", "true"),
        (r"(?:\b(?i))*", "true"),
        (r"(?:(?i)\b\b)*", "true"),
        (r"(?:(?i)a\b)*", "true"),
        (r"(?:\Qab\E\b)*", "true"),
        (r"(?:\Q\E\b)*", "true"),
        (r"(?:\b){,2}", "false"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter(pattern)], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{pattern}: stderr={stderr:?}"
        );
    }
    Ok(())
}

/// Under the `x` flag the check reads the pattern without the blanks and comments the flag ignores
/// (#6215), so the repeated assertion group is refused there as it is without the flag; jq refuses
/// it in both.
#[test]
fn test_the_extended_mode_is_checked_without_its_blanks_6127() -> Result<()> {
    for pattern in [r"(?:a|\\b)*", r"(?: a | \\b )*"] {
        let (stdout, stderr, code) =
            run_jq_full(&["-nc", &format!(r#""ab" | test("{pattern}";"x")"#)], None)?;
        assert_eq!((stdout.trim_end(), code), ("", 5), "{pattern}: {stderr:?}");
        assert!(
            stderr.contains("target of repeat operator is invalid"),
            "{pattern}: {stderr:?}"
        );
    }
    Ok(())
}
