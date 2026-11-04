//! #6170: a comment is invisible to jq, so a repeat interval that follows only comments (`(?#){0,1}a`,
//! `(?:(?#c){2}|b)`, `x|(?#){1}a`) has no target and is refused, as `{0,1}a` is (#6070). The scan
//! that looks for an interval with nothing before it forgot what stood before a comment; it keeps
//! it now. The extended mode skips the check altogether (`{2}a` under `x` answers `true` where jq
//! refuses it, a documented gap), so no row of it is a refusal. Every row was captured from
//! jq 1.7.1 (`/usr/bin/jq -n`) over `"caab"`.

use anyhow::Result;

use crate::run_jq_full;

fn run(pattern: &str, flags: &str) -> Result<(String, String, i32)> {
    run_jq_full(
        &[
            "-nc",
            "--arg",
            "p",
            pattern,
            "--arg",
            "f",
            flags,
            r#""caab" | test($p; $f)"#,
        ],
        None,
    )
}

#[test]
fn test_an_interval_after_only_comments_is_refused_6170() -> Result<()> {
    for (pattern, flags) in [
        ("(?#){0,1}a", ""),
        ("(?#c){2}a", ""),
        ("(?#){2,}a", ""),
        ("(?:(?#){0,1}a)", ""),
        ("(?:(?#){1}|b)", ""),
        ("x|(?#){1}a", ""),
        ("(?#)(?#){1}a", ""),
        ("(?i:(?#){1}a)", ""),
        ("(?=(?#){1}a)", ""),
        ("(?#){1}", ""),
        ("(?#){2}?a", ""),
        ("((?#)(?#){1,2}?a)", ""),
        ("(?#c)a|(?#){2}b", ""),
    ] {
        let (stdout, stderr, code) = run(pattern, flags)?;
        assert_eq!(
            (stdout.as_str(), code),
            ("", 5),
            "{pattern:?} {flags}: {stderr}"
        );
        assert!(
            stderr.contains("invalid regex: target of repeat operator is not specified"),
            "{pattern:?} {flags}: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_an_interval_that_has_a_target_or_is_literal_still_answers_6170() -> Result<()> {
    for (pattern, flags, expected) in [
        ("a(?#){2}", "", "true"),
        ("(?#){,2}a", "", "false"),
        ("(?#){x}", "", "false"),
        ("(?#){2", "", "false"),
        ("(?#)a{2}", "", "true"),
        ("a|(?#)b{2}", "", "true"),
        ("(?#)(a){1,2}", "", "true"),
        ("a # c\n{2}", "x", "true"),
        ("# c\na{2}", "x", "true"),
    ] {
        let (stdout, stderr, code) = run(pattern, flags)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{pattern:?} {flags}: {stderr}"
        );
    }
    Ok(())
}
