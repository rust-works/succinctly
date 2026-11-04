//! #6181: a bare option group (`(?i)`, `(?-i)`, `(?i-m)`) is no item to Oniguruma, so a repeat
//! operator after it, directly or past comments, has nothing to repeat and is refused with `target
//! of repeat operator is not specified`. The `regex` crate repeated the zero-width flag item and
//! `succinctly jq` answered as if the operator were not there. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -n`); the matrix below is a slice of the 1,350-pattern sweep it was read from.
//! jq words it `not specified` after an item as well as at the start of a pattern.
//!
//! Not covered: a pattern with an inline `(?x)` (the repeat check is off there, #6127's extended
//! mode), and a possessive operator (`(?i)*+`), which the possessive refusal words first (#6184).

use anyhow::Result;

use crate::run_jq_full;

fn filter(pattern: &str) -> String {
    format!(
        r#""caab" | test("{}")"#,
        pattern.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

fn assert_not_specified(pattern: &str) -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-nc", &filter(pattern)], None)?;
    assert_eq!(code, 5, "{pattern}: stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stdout, "", "{pattern}");
    assert!(
        stderr.contains("invalid regex: target of repeat operator is not specified"),
        "{pattern}: {stderr:?}"
    );
    Ok(())
}

#[test]
fn test_a_repeat_after_a_bare_option_group_is_refused_6181() -> Result<()> {
    for pattern in [
        "(?i){1}a",
        "(?i)*a",
        "a(?i)+",
        "a(?i){2}b",
        "(?i)(?m)*a",
        "(?i)?a",
        "a|(?i)*",
        "(?:(?i)*)",
        "(a(?i)*)",
        "(?i)*?a",
        "a(?i)+?b",
        "(?-i)*a",
        "(?i-m)+a",
        "(?s){1,2}a",
        "(?i)(?#c)*a",
        "(?#c)(?i)*a",
        "(?=(?i)*)",
        "(?<=a(?i)?)",
        "a(?i){2,}",
        "(?i){0}a",
        "(?i){0,1}a",
        "x|(?i){1}",
        // A group of only minus signs is a bare option group too.
        "(?-)*a",
        "(?i-)*a",
    ] {
        assert_not_specified(pattern)?;
    }
    Ok(())
}

#[test]
fn test_every_operator_after_every_option_group_is_refused_6181() -> Result<()> {
    for option in ["(?i)", "(?-i)", "(?m)", "(?i-m)", "(?s)", "(?i)(?m)"] {
        for operator in [
            "*", "+", "?", "{1}", "{2}", "{1,2}", "{2,}", "{0}", "*?", "+?", "??", "{1}?",
        ] {
            for shape in ["{o}{q}a", "a{o}{q}", "(?:a|{o}{q})b"] {
                let pattern = shape.replace("{o}", option).replace("{q}", operator);
                assert_not_specified(&pattern)?;
            }
        }
    }
    Ok(())
}

#[test]
fn test_what_is_not_a_repeat_of_an_option_group_still_answers_6181() -> Result<()> {
    for (pattern, expected) in [
        ("(?i)a", "true"),
        // `{,2}` is the literal run it is everywhere (#6068).
        ("(?i){,2}a", "false"),
        ("a(?i){,2}b", "false"),
        ("(?i)a*", "true"),
        ("a(?i)b+", "true"),
        ("(?i)(a)*", "true"),
        ("(?i:a)*", "true"),
        ("(?i:a*)", "true"),
        ("((?i))*", "true"),
        ("(?i)a{2}", "true"),
        ("(?i)(?:)*", "true"),
        ("(?:(?i)a)*", "true"),
        ("a(?i)|b*", "true"),
        (r"(?i)\*", "false"),
        ("(?i)[*]", "false"),
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

/// The empty group `(?)` is `undefined group option` before any repeat is looked at.
#[test]
fn test_an_empty_option_group_keeps_its_own_refusal_6181() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-nc", &filter("(?)*a")], None)?;
    assert_eq!((stdout.as_str(), code), ("", 5), "stderr={stderr:?}");
    assert!(
        stderr.contains("invalid regex: undefined group option"),
        "{stderr:?}"
    );
    Ok(())
}

/// An inline `(?x)` makes the scan stand down (it cannot see the mode's ignored blanks), so these
/// stay as they were. jq refuses all three (`target of repeat operator is not specified`): an
/// unfixed gap, recorded in `docs/compliance/jq/limitations.md`, so this pins what is.
#[test]
fn test_the_extended_mode_is_left_alone_6181() -> Result<()> {
    for pattern in ["(?x)(?i)*a", "(?x) a(?i)+", "(?i)(?x)*a"] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter(pattern)], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            ("true", 0),
            "{pattern}: {stderr:?}"
        );
    }
    Ok(())
}
