//! #6133: Oniguruma lets an option run repeat `-` (a later `-` is a no-op, and letters after it are
//! still turned off) and end on one (`(?-)`, `(?i-:...)`); the `regex` crate refuses both. Every
//! row was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_repeated_or_trailing_minus_in_an_option_run_is_read_as_jq_does_6133() -> Result<()> {
    for (filter, expected) in [
        (r#""a" | test("(?-)a")"#, "true"),
        (r#""a" | test("(?--i)a")"#, "true"),
        (r#""A" | test("(?--i)a")"#, "false"),
        (r#""A" | test("(?i-m-s)a")"#, "true"),
        (r#""A" | test("(?i--)a")"#, "true"),
        (r#""A" | test("(?i-:a)")"#, "true"),
        (r#""A" | test("(?-:a)")"#, "false"),
        (r#""a" | test("(?-:a)b")"#, "false"),
        (r#""A" | test("(?i--m)a")"#, "true"),
        (r#""a b" | test("(?x-)a b")"#, "false"),
        (r#""ab" | test("(?x-)a b")"#, "true"),
        (r#""ab" | test("(?x-i-x)a b")"#, "false"),
        (r#""a b" | test("(?x-i-x)a b")"#, "true"),
        (r#""a b" | test("(?i-m-x)a b")"#, "true"),
        (r#""ab" | test("(?i-m-x)a b")"#, "false"),
        (r#""ab" | test("(?--x:a b)")"#, "false"),
        (r#""ab" | test("(?x--:a b)")"#, "true"),
        (r#""a\nb" | test("(?s-)a.b")"#, "true"),
        (r#""a\nb" | test("(?-s-m)a.b")"#, "false"),
        (r#""xa" | test("x(?-)a")"#, "true"),
        (r#""xA" | test("x(?i-:a)")"#, "true"),
        (r#""xA" | test("x(?i-:a)|b")"#, "true"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

/// The option run is rewritten for every regex builtin, not only `test`.
#[test]
fn the_rewrite_reaches_match_sub_and_scan_6133() -> Result<()> {
    for (filter, expected) in [
        (
            r#""aA" | [match("(?i-m-s)a";"g") | .string]"#,
            r#"["a","A"]"#,
        ),
        (r#""aA" | sub("(?--i)a";"X")"#, r#""XA""#),
        (r#""aAa" | [scan("(?i-:a)")]"#, r#"["a","A","a"]"#),
        (r#""ab" | capture("(?-)(?<x>a)")"#, r#"{"x":"a"}"#),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

/// An option group that names no option is dropped, so a repeat after it has no target in jq either.
#[test]
fn a_repeat_after_an_empty_option_group_is_refused_6133() -> Result<()> {
    for pattern in [
        "(?-)*a",
        "(?-)+a",
        "(?-)?a",
        "(?-){2}a",
        "(?-){1,}a",
        "a(?-)*",
        "a(?-)+",
        "a(?-)*?",
        "a(?--)?",
        "a(?-){2}",
        "x(?--){1,2}",
        "a|(?-){2}",
        "^(?-)*a",
        "^(?-)+",
        "$(?-)*",
        r"\\b(?-)*",
        r"\\b(?--)+a",
    ] {
        let filter = format!(r#""aab" | test("{pattern}")"#);
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter], None)?;
        assert_eq!(code, 5, "{filter}: stdout={stdout:?} stderr={stderr:?}");
        assert!(
            stderr.contains("Regex failure: target of repeat operator is not specified")
                || stderr.contains("Regex failure: target of repeat operator is not specified"),
            "{filter}: {stderr:?}"
        );
    }
    Ok(())
}

/// Review finding: the `regex` crate drops blanks inside a class under `x`, Oniguruma keeps them,
/// and the rewritten `(?x-:` run reached that route (`(?x:[a b])+` was already wrong). Rows from
/// jq 1.7.1.
#[test]
fn a_blank_in_a_class_stays_literal_under_the_extended_mode_6133() -> Result<()> {
    for (filter, expected) in [
        (
            r#""a b\nA B" | [match("(?x-:[a b])+";"g") | .string]"#,
            r#"["a b"," "]"#,
        ),
        (
            r#""a b\nA B" | [match("(?x:[a b])+";"g") | .string]"#,
            r#"["a b"," "]"#,
        ),
        (
            r#""a b\nA B" | [match("(?x)[a b]+";"g") | .string]"#,
            r#"["a b"," "]"#,
        ),
        (r#""a b" | [match("[a b]+";"gx") | .string]"#, r#"["a b"]"#),
        (r#""a b" | test("(?-x)[a b]+$")"#, "true"),
        // Outside a class the blank is still ignored.
        (r#""ab" | test("(?x-:a b)")"#, "true"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

/// A letter repeated in a run, or named on and then off, is applied in order as Oniguruma does.
#[test]
fn a_repeated_letter_in_a_dash_run_is_applied_in_order_6133() -> Result<()> {
    for (filter, expected) in [
        (r#""aAbB" | [match("((?i-ss-)a)B";"g") | .string]"#, "[]"),
        (
            r#""a b #c\nA" | [match("(?x)(a (?-x-x) b) #c\nA";"g") | .string]"#,
            "[]",
        ),
        (r#""A" | test("(?i-i)a")"#, "false"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}
